//! The daemon's durable event inbox: each event read from LDK Server is saved here before it is handled and deleted once handled, in its own SQLite file so saving never waits on the main database.

use std::path::Path;
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
    pub(crate) fn open(dir: &Path) -> rusqlite::Result<Inbox> {
        Self::init(Connection::open(dir.join(INBOX_FILENAME))?)
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
             );",
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

    /// Take the rows whose handler was running when the daemon stopped; they are deleted, not replayed.
    pub(crate) fn take_interrupted(&self) -> rusqlite::Result<Vec<Row>> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = tx
            .prepare("SELECT id, kind, payload, dropped, received_ms, started FROM events WHERE started != 0 ORDER BY id")?
            .query_map([], read_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        tx.execute("DELETE FROM events WHERE started != 0", [])?;
        tx.commit()?;
        Ok(rows)
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
        let interrupted = inbox.take_interrupted().unwrap();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].id, second);
        assert_eq!(interrupted[0].entry, event("pay-1"));
        assert!(interrupted[0].started);
        assert_eq!(inbox.oldest().unwrap().unwrap(), first);
        inbox.finish(first.id).unwrap();
        assert_eq!(inbox.oldest().unwrap().unwrap().entry, event("pay-2"));
        assert!(inbox.take_interrupted().unwrap().is_empty());
    }

    #[test]
    fn clearing_started_leaves_the_row_queued() {
        let inbox = Inbox::in_memory();
        inbox.save(&[(event("pay-0"), 0)]).unwrap();
        let id = inbox.oldest().unwrap().unwrap().id;
        inbox.set_started(id, true).unwrap();
        assert!(inbox.oldest().unwrap().unwrap().started);
        inbox.set_started(id, false).unwrap();
        assert!(inbox.take_interrupted().unwrap().is_empty());
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
}
