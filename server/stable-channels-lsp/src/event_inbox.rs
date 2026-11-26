//! The daemon's durable event inbox: each event read from LDK Server is saved here before it is handled and deleted once handled, in its own SQLite file so saving never waits on the main database.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use ldk_server_client::ldk_server_grpc::events::EventEnvelope;
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use tracing::warn;

/// Kept next to the main database; transient, so it must not be restored from a backup.
pub(crate) const INBOX_FILENAME: &str = "event_inbox.db";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    Event(EventEnvelope),
    /// A subscription started here, so its connect reconcile runs here.
    Connected,
    /// The inbox was full, so this many events were dropped here.
    Dropped(u64),
    /// A saved row that no longer decodes; only ever read, never saved.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub(crate) id: i64,
    pub(crate) entry: Entry,
    pub(crate) received_ms: i64,
    /// Its handler was running; a row still marked at startup was interrupted and is not replayed.
    pub(crate) started: bool,
}

/// The file itself is damaged or isn't a database: the one failure that setting it aside cures.
pub(crate) fn is_unreadable(error: &rusqlite::Error) -> bool {
    matches!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt))
}

/// Rename the inbox and its journal files to `<name>.unreadable-<now_ms>`; returns the database file's new path.
/// The database goes first, made durable before the journal follows: a stop in between leaves a journal with no database, which SQLite deletes when it creates the new one, whereas a database left without its journal would lose committed deletes and bring handled rows back.
pub(crate) fn set_aside(dir: &Path, now_ms: u128) -> std::io::Result<PathBuf> {
    let aside = |suffix: &str| dir.join(format!("{INBOX_FILENAME}.unreadable-{now_ms}{suffix}"));
    std::fs::rename(dir.join(INBOX_FILENAME), aside(""))?;
    std::fs::File::open(dir)?.sync_all()?;
    for suffix in ["-wal", "-shm"] {
        match std::fs::rename(dir.join(format!("{INBOX_FILENAME}{suffix}")), aside(suffix)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {},
        }
    }
    std::fs::File::open(dir)?.sync_all()?;
    Ok(aside(""))
}

/// Encoded size an entry counts against the inbox limit; markers count nothing.
pub(crate) fn payload_size(entry: &Entry) -> usize {
    match entry {
        Entry::Event(envelope) => prost::Message::encoded_len(envelope),
        _ => 0,
    }
}

pub(crate) struct Inbox {
    conn: Mutex<Connection>,
}

impl Inbox {
    /// Open the inbox, checking its pages too, so a damaged file fails here, where startup can set it aside, rather than on every later read.
    pub(crate) fn open(dir: &Path) -> rusqlite::Result<Inbox> {
        let inbox = Self::init(Connection::open(dir.join(INBOX_FILENAME))?)?;
        let check: String = inbox.lock().query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        if check != "ok" {
            return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT), Some(check)));
        }
        Ok(inbox)
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Inbox {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> rusqlite::Result<Inbox> {
        // auto_vacuum only takes effect before the first table is created, so a drained inbox can give its pages back; the journal is cut back to 8 MiB once written out.
        conn.execute_batch(
            "PRAGMA auto_vacuum = INCREMENTAL;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA journal_size_limit = 8388608;
             CREATE TABLE IF NOT EXISTS events (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 kind TEXT NOT NULL,
                 payload BLOB,
                 dropped INTEGER,
                 received_ms INTEGER NOT NULL,
                 started INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS reruns (id INTEGER PRIMARY KEY);",
        )?;
        Ok(Inbox { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn lock_for_test(&self) -> MutexGuard<'_, Connection> {
        self.lock()
    }

    /// Save entries in arrival order in one transaction, each with the time it was read.
    #[cfg(test)]
    pub(crate) fn save(&self, entries: &[(Entry, i64)]) -> rusqlite::Result<()> {
        self.save_and_then(entries, || ())
    }

    /// Save as `save` does, running `committed` once the rows are in and before anyone else can read them.
    pub(crate) fn save_and_then(&self, entries: &[(Entry, i64)], committed: impl FnOnce()) -> rusqlite::Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut insert = tx.prepare_cached("INSERT INTO events (kind, payload, dropped, received_ms) VALUES (?1, ?2, ?3, ?4)")?;
            for (entry, received_ms) in entries {
                let (kind, payload, dropped) = match entry {
                    Entry::Event(envelope) => ("event", Some(prost::Message::encode_to_vec(envelope)), None),
                    Entry::Connected => ("connected", None, None),
                    Entry::Dropped(count) => ("dropped", None, Some(i64::try_from(*count).unwrap_or(i64::MAX))),
                    Entry::Unreadable => return Err(rusqlite::Error::InvalidParameterName("an unreadable row is never saved".into())),
                };
                insert.execute(params![kind, payload, dropped, received_ms])?;
            }
        }
        tx.commit()?;
        committed();
        Ok(())
    }

    pub(crate) fn oldest(&self) -> rusqlite::Result<Option<Row>> {
        self.lock()
            .query_row("SELECT id, kind, payload, dropped, received_ms, started FROM events ORDER BY id LIMIT 1", [], read_row)
            .optional()
    }

    /// Whether the row saved after `id` is another subscription's start.
    pub(crate) fn next_is_connected(&self, id: i64) -> rusqlite::Result<bool> {
        self.lock()
            .query_row("SELECT kind = 'connected' FROM events WHERE id > ?1 ORDER BY id LIMIT 1", [id], |row| row.get(0))
            .optional()
            .map(|next| next.unwrap_or(false))
    }

    pub(crate) fn set_started(&self, id: i64, started: bool) -> rusqlite::Result<()> {
        self.lock().execute("UPDATE events SET started = ?2 WHERE id = ?1", params![id, started])?;
        Ok(())
    }

    /// Queue an interrupted row to be handled again, remembering in the same commit that it was, so it is handled again only once.
    pub(crate) fn requeue(&self, id: i64) -> rusqlite::Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("INSERT OR IGNORE INTO reruns (id) VALUES (?1)", [id])?;
        tx.execute("UPDATE events SET started = 0 WHERE id = ?1", [id])?;
        tx.commit()
    }

    /// Whether this row was already queued again after an interruption.
    pub(crate) fn requeued_before(&self, id: i64) -> rusqlite::Result<bool> {
        self.lock().query_row("SELECT EXISTS (SELECT 1 FROM reruns WHERE id = ?1)", [id], |row| row.get(0))
    }

    /// Delete a handled row, returning the events and payload bytes it held, and give the disk back once nothing is left.
    pub(crate) fn finish(&self, id: i64) -> rusqlite::Result<(usize, usize)> {
        let mut conn = self.lock();
        // An explicit transaction, so a commit that fails is reported rather than lost when the statement is reset.
        let tx = conn.transaction()?;
        let freed = tx
            .query_row(
                "DELETE FROM events WHERE id = ?1 RETURNING kind = 'event', COALESCE(LENGTH(payload), 0)",
                [id],
                |row| Ok((row.get::<_, bool>(0)? as usize, row.get::<_, i64>(1)?.max(0) as usize)),
            )
            .optional()?
            .unwrap_or((0, 0));
        tx.execute("DELETE FROM reruns WHERE id = ?1", [id])?;
        tx.commit()?;
        // The row is gone whatever happens here, so a failure only delays giving the disk back.
        if let Err(error) = give_back_if_empty(&conn) {
            warn!("[event_inbox] couldn't give the inbox's disk back: {}", error);
        }
        Ok(freed)
    }

    pub(crate) fn is_empty(&self) -> rusqlite::Result<bool> {
        self.lock().query_row("SELECT NOT EXISTS (SELECT 1 FROM events)", [], |row| row.get(0))
    }

    /// The rows whose handler was running when the daemon stopped, left in place until each is recorded.
    pub(crate) fn interrupted(&self) -> rusqlite::Result<Vec<Row>> {
        self.lock()
            .prepare("SELECT id, kind, payload, dropped, received_ms, started FROM events WHERE started != 0 ORDER BY id")?
            .query_map([], read_row)?
            .collect()
    }

    /// Saved events and their payload bytes, markers excluded.
    pub(crate) fn held(&self) -> rusqlite::Result<(usize, usize)> {
        self.lock().query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(payload)), 0) FROM events WHERE kind = 'event'",
            [],
            |row| Ok((row.get::<_, i64>(0)? as usize, row.get::<_, i64>(1)? as usize)),
        )
    }
}

/// Once the inbox is empty, free its pages and cut the file and its journal back.
fn give_back_if_empty(conn: &Connection) -> rusqlite::Result<()> {
    let empty: bool = conn.query_row("SELECT NOT EXISTS (SELECT 1 FROM events)", [], |row| row.get(0))?;
    if empty {
        // The pragma frees one page per step, so step it to the end.
        let mut vacuum = conn.prepare("PRAGMA incremental_vacuum")?;
        let mut pages = vacuum.query([])?;
        while pages.next()?.is_some() {}
        // In WAL mode the file only shrinks when the journal is written back.
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    }
    Ok(())
}

/// A row whose columns don't read as a known entry is Unreadable, so it can't stop the queue.
fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let entry = match (row.get_ref(1)?, row.get_ref(2)?) {
        (ValueRef::Text(b"event"), ValueRef::Blob(payload)) => {
            <EventEnvelope as prost::Message>::decode(payload).map(Entry::Event).unwrap_or(Entry::Unreadable)
        },
        (ValueRef::Text(b"connected"), _) => Entry::Connected,
        (ValueRef::Text(b"dropped"), _) => row.get_ref(3)?.as_i64().map_or(Entry::Unreadable, |count| Entry::Dropped(count.max(0) as u64)),
        _ => Entry::Unreadable,
    };
    Ok(Row {
        id: row.get(0)?,
        entry,
        received_ms: row.get_ref(4)?.as_i64().unwrap_or(0),
        started: row.get_ref(5)?.as_i64().map_or(true, |started| started != 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ldk_server_client::ldk_server_grpc::events::event_envelope::Event as EventVariant;
    use ldk_server_client::ldk_server_grpc::events::EventEnvelope;

    fn claimable(payment_id: &str) -> EventEnvelope {
        use ldk_server_client::ldk_server_grpc::events::PaymentClaimable;
        use ldk_server_client::ldk_server_grpc::types::Payment;
        EventEnvelope {
            event: Some(EventVariant::PaymentClaimable(PaymentClaimable {
                payment: Some(Payment { payment_id: payment_id.to_owned(), ..Default::default() }),
                ..Default::default()
            })),
        }
    }

    fn event(payment_id: &str) -> Entry {
        Entry::Event(claimable(payment_id))
    }

    #[test]
    fn rows_come_back_in_saved_order_after_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let saved = [Entry::Connected, event("pay-0"), Entry::Dropped(3), event("pay-1")];
        let inbox = Inbox::open(dir.path()).unwrap();
        let batch: Vec<(Entry, i64)> = saved.iter().cloned().zip(10..).collect();
        inbox.save(&batch).unwrap();
        drop(inbox);
        let inbox = Inbox::open(dir.path()).unwrap();
        for (entry, received_ms) in saved.iter().zip(10..) {
            let row = inbox.oldest().unwrap().expect("a saved row");
            assert_eq!(&row.entry, entry);
            assert_eq!(row.received_ms, received_ms);
            assert!(!row.started);
            inbox.finish(row.id).unwrap();
        }
        assert_eq!(inbox.oldest().unwrap(), None);
    }

    #[test]
    fn ids_are_not_reused_after_a_delete() {
        let inbox = Inbox::in_memory();
        inbox.save(&[(event("pay-0"), 0)]).unwrap();
        let first = inbox.oldest().unwrap().unwrap().id;
        inbox.finish(first).unwrap();
        inbox.save(&[(event("pay-1"), 0)]).unwrap();
        assert!(inbox.oldest().unwrap().unwrap().id > first);
    }

    #[test]
    fn a_started_row_is_taken_as_interrupted_and_others_stay() {
        let inbox = Inbox::in_memory();
        inbox.save(&[(event("pay-0"), 0), (event("pay-1"), 1), (event("pay-2"), 2)]).unwrap();
        let first = inbox.oldest().unwrap().unwrap();
        let second = first.id + 1;
        inbox.set_started(second, true).unwrap();
        let interrupted = inbox.interrupted().unwrap();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].id, second);
        assert_eq!(interrupted[0].entry, event("pay-1"));
        assert!(interrupted[0].started);
        assert_eq!(inbox.interrupted().unwrap(), interrupted, "listing leaves it in place until it is recorded");
        assert_eq!(inbox.oldest().unwrap().unwrap(), first);
        inbox.finish(first.id).unwrap();
        inbox.finish(second).unwrap();
        assert_eq!(inbox.oldest().unwrap().unwrap().entry, event("pay-2"));
        assert!(inbox.interrupted().unwrap().is_empty());
    }

    #[test]
    fn clearing_started_leaves_the_row_queued() {
        let inbox = Inbox::in_memory();
        inbox.save(&[(event("pay-0"), 0)]).unwrap();
        let id = inbox.oldest().unwrap().unwrap().id;
        inbox.set_started(id, true).unwrap();
        assert!(inbox.oldest().unwrap().unwrap().started);
        inbox.set_started(id, false).unwrap();
        assert!(inbox.interrupted().unwrap().is_empty());
        let row = inbox.oldest().unwrap().unwrap();
        assert_eq!((row.id, row.started), (id, false));
    }

    #[test]
    fn held_counts_events_and_their_payload_bytes() {
        let inbox = Inbox::in_memory();
        let (a, b) = (event("pay-0"), event("a-longer-payment-id"));
        let expected = payload_size(&a) + payload_size(&b);
        assert!(expected > 0);
        inbox.save(&[(a, 0), (Entry::Connected, 1), (b, 2), (Entry::Dropped(5), 3)]).unwrap();
        assert_eq!(inbox.held().unwrap(), (2, expected));
        assert_eq!(payload_size(&Entry::Connected), 0);
        assert_eq!(payload_size(&Entry::Dropped(5)), 0);
    }

    #[test]
    fn a_drained_inbox_gives_its_disk_back() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        let id = "x".repeat(4_000);
        let batch: Vec<(Entry, i64)> = (0..2_000).map(|n| (event(&format!("{id}{n}")), n)).collect();
        for chunk in batch.chunks(500) {
            inbox.save(chunk).unwrap();
        }
        while let Some(row) = inbox.oldest().unwrap() {
            inbox.finish(row.id).unwrap();
        }
        let on_disk = std::fs::metadata(dir.path().join(INBOX_FILENAME)).unwrap().len();
        let journal = std::fs::metadata(dir.path().join(format!("{INBOX_FILENAME}-wal"))).map(|meta| meta.len()).unwrap_or(0);
        assert!(on_disk + journal < 64 * 1024, "given back while the daemon runs: {on_disk} + {journal} bytes");
        drop(inbox);
        let conn = rusqlite::Connection::open(dir.path().join(INBOX_FILENAME)).unwrap();
        let free: i64 = conn.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap();
        let pages: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0)).unwrap();
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0)).unwrap();
        assert_eq!(free, 0);
        assert!(pages * page_size < 64 * 1024, "{pages} pages of {page_size} bytes");
    }

    #[test]
    fn a_large_batch_leaves_no_large_journal_behind() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        let id = "x".repeat(8_000);
        let batch: Vec<(Entry, i64)> = (0..2_000).map(|n| (event(&format!("{id}{n}")), n)).collect();
        inbox.save(&batch).unwrap();
        for _ in 0..2 {
            let row = inbox.oldest().unwrap().unwrap();
            inbox.finish(row.id).unwrap();
        }
        let journal = std::fs::metadata(dir.path().join(format!("{INBOX_FILENAME}-wal"))).map(|meta| meta.len()).unwrap_or(0);
        assert!(journal <= 8 * 1024 * 1024, "the journal is cut back once written out: {journal} bytes");
    }

    #[test]
    fn a_delete_that_cannot_commit_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        inbox.save(&[(event("pay-0"), 0)]).unwrap();
        let id = inbox.oldest().unwrap().unwrap().id;
        {
            // A constraint checked at commit makes the commit fail, as a full disk would.
            let conn = inbox.lock_for_test();
            conn.execute_batch("PRAGMA foreign_keys = ON; CREATE TABLE pin (ref INTEGER REFERENCES events(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
            conn.execute("INSERT INTO pin VALUES (?1)", [id]).unwrap();
        }
        assert!(inbox.finish(id).is_err(), "a delete that didn't commit isn't reported as done");
        assert_eq!(inbox.oldest().unwrap().unwrap().id, id, "the row is still there to finish again");
    }

    #[test]
    fn a_row_that_does_not_decode_reads_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        inbox.save(&[(event("pay-0"), 7)]).unwrap();
        let id = inbox.oldest().unwrap().unwrap().id;
        let conn = rusqlite::Connection::open(dir.path().join(INBOX_FILENAME)).unwrap();
        conn.execute("UPDATE events SET payload = x'ff' WHERE id = ?1", [id]).unwrap();
        let row = inbox.oldest().unwrap().unwrap();
        assert_eq!((row.id, row.entry, row.received_ms), (id, Entry::Unreadable, 7));
    }

    #[test]
    fn damaged_pages_fail_the_open_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        let id = "x".repeat(1_000);
        let batch: Vec<(Entry, i64)> = (0..200).map(|n| (event(&format!("{id}{n}")), n)).collect();
        inbox.save(&batch).unwrap();
        let page_size: usize = inbox.lock_for_test().query_row("PRAGMA page_size", [], |row| row.get(0)).unwrap();
        drop(inbox);
        // The schema page still reads, so only a check of the other pages finds the damage.
        let path = dir.path().join(INBOX_FILENAME);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[page_size..].fill(0xaa);
        std::fs::write(&path, bytes).unwrap();
        let error = Inbox::open(dir.path()).err().expect("the damage is found at open");
        assert!(is_unreadable(&error), "{error}");
    }

    /// An inbox holding `pay-1` only, whose delete of `pay-0` is committed in the journal but not yet in the database file.
    fn inbox_with_a_delete_in_its_journal(dir: &Path) {
        let inbox = Inbox::open(dir).unwrap();
        inbox.save(&[(event("pay-0"), 0), (event("pay-1"), 1)]).unwrap();
        drop(inbox);
        let inbox = Inbox::open(dir).unwrap();
        inbox.finish(inbox.oldest().unwrap().unwrap().id).unwrap();
        let (db, wal) = (dir.join(INBOX_FILENAME), dir.join(format!("{INBOX_FILENAME}-wal")));
        let (db_bytes, wal_bytes) = (std::fs::read(&db).unwrap(), std::fs::read(&wal).unwrap());
        drop(inbox);
        std::fs::write(&db, db_bytes).unwrap();
        std::fs::write(&wal, wal_bytes).unwrap();
        let _ = std::fs::remove_file(dir.join(format!("{INBOX_FILENAME}-shm")));
    }

    #[test]
    fn a_journal_left_without_its_database_is_deleted_not_replayed() {
        let dir = tempfile::tempdir().unwrap();
        inbox_with_a_delete_in_its_journal(dir.path());
        // A stop after the database was moved and before its journal followed.
        std::fs::rename(dir.path().join(INBOX_FILENAME), dir.path().join("moved")).unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        assert_eq!(inbox.oldest().unwrap(), None, "a new inbox, with nothing of the old one");
    }

    #[test]
    fn a_set_aside_that_fails_part_way_never_leaves_a_database_without_its_journal() {
        let dir = tempfile::tempdir().unwrap();
        inbox_with_a_delete_in_its_journal(dir.path());
        // Moving the database fails: something already has its new name.
        let taken = dir.path().join(format!("{INBOX_FILENAME}.unreadable-7"));
        std::fs::create_dir(&taken).unwrap();
        std::fs::write(taken.join("x"), "x").unwrap();
        assert!(set_aside(dir.path(), 7).is_err());
        let inbox = Inbox::open(dir.path()).unwrap();
        let first = inbox.oldest().unwrap().unwrap();
        assert_eq!(first.entry, event("pay-1"), "the committed delete still holds, so a handled row doesn't come back");
    }

    #[test]
    fn a_requeued_row_is_remembered_until_it_is_finished() {
        let inbox = Inbox::in_memory();
        inbox.save(&[(event("pay-0"), 0)]).unwrap();
        let id = inbox.oldest().unwrap().unwrap().id;
        inbox.set_started(id, true).unwrap();
        assert!(!inbox.requeued_before(id).unwrap());
        inbox.requeue(id).unwrap();
        assert!(!inbox.oldest().unwrap().unwrap().started);
        assert!(inbox.requeued_before(id).unwrap());
        inbox.finish(id).unwrap();
        assert!(!inbox.requeued_before(id).unwrap());
    }

    #[test]
    fn an_inbox_from_before_requeues_gains_their_table() {
        let dir = tempfile::tempdir().unwrap();
        rusqlite::Connection::open(dir.path().join(INBOX_FILENAME)).unwrap().execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, payload BLOB, dropped INTEGER, received_ms INTEGER NOT NULL, started INTEGER NOT NULL DEFAULT 0);",
        ).unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        inbox.save(&[(event("pay-0"), 0)]).unwrap();
        let id = inbox.oldest().unwrap().unwrap().id;
        inbox.requeue(id).unwrap();
        assert!(inbox.requeued_before(id).unwrap());
    }

    #[test]
    fn setting_aside_moves_the_journal_with_the_database() {
        let dir = tempfile::tempdir().unwrap();
        for suffix in ["", "-wal", "-shm"] {
            std::fs::write(dir.path().join(format!("{INBOX_FILENAME}{suffix}")), suffix).unwrap();
        }
        let moved = set_aside(dir.path(), 7).unwrap();
        assert_eq!(moved, dir.path().join(format!("{INBOX_FILENAME}.unreadable-7")));
        for suffix in ["", "-wal", "-shm"] {
            assert!(!dir.path().join(format!("{INBOX_FILENAME}{suffix}")).exists(), "no old {suffix:?} file is left for a new inbox");
            assert_eq!(std::fs::read_to_string(dir.path().join(format!("{INBOX_FILENAME}.unreadable-7{suffix}"))).unwrap(), suffix);
        }
    }
}
