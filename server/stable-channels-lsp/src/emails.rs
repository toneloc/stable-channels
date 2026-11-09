//! sqlite-backed store of the optional email each wallet shares, keyed by its Lightning node id (emails.db).

use std::path::Path;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};
use tracing::error;

/// Most nodes kept; past this a node with no channel is refused, while stored nodes can still change or remove theirs.
const MAX_EMAILS: i64 = 100_000;
/// Most new nodes with no channel stored per hour, so a flood of fresh keys takes weeks, not minutes, to reach the cap.
const MAX_NEW_STRANGERS_PER_HOUR: i64 = 200;
const HOUR_SECS: i64 = 3_600;

/// What became of a signed email change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    /// Signed no later than the node's last accepted change, so a replayed request never undoes a newer one.
    Stale,
    /// The store holds `max_rows` nodes and this new node has no channel.
    Full,
    /// This hour's room for new nodes with no channel is used up.
    Limited,
    Failed,
}

/// The connection and the counts the limits read, kept together under one lock so a check and its write never race.
struct Store {
    conn: Connection,
    rows: i64,
    hour_start: i64,
    new_this_hour: i64,
}

pub struct Emails {
    /// Writes take turns on the one connection from a blocking thread.
    store: Arc<tokio::sync::Mutex<Store>>,
    max_rows: i64,
    max_new_per_hour: i64,
}

impl Emails {
    pub fn open(data_dir: &Path) -> rusqlite::Result<Self> {
        Self::open_limited(data_dir, MAX_EMAILS, MAX_NEW_STRANGERS_PER_HOUR)
    }

    /// The store, or None when emails.db cannot be opened: email is optional, so the LSP keeps running and the route answers 503.
    pub fn open_or_off(data_dir: &Path) -> Option<Arc<Self>> {
        match Self::open(data_dir) {
            Ok(emails) => Some(Arc::new(emails)),
            Err(e) => {
                error!(
                    "[emails] emails.db could not be opened; email registration is off: {}",
                    e
                );
                None
            }
        }
    }

    pub(crate) fn open_limited(
        data_dir: &Path,
        max_rows: i64,
        max_new_per_hour: i64,
    ) -> rusqlite::Result<Self> {
        Self::with_connection(
            Connection::open(data_dir.join("emails.db"))?,
            max_rows,
            max_new_per_hour,
        )
    }

    fn with_connection(
        conn: Connection,
        max_rows: i64,
        max_new_per_hour: i64,
    ) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS emails (
                node_id    TEXT PRIMARY KEY,
                email      TEXT NOT NULL,
                signed_ts  INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )?;
        // Counted once here; each insert then keeps the count, so the cap check never scans the table.
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM emails", [], |row| row.get(0))?;
        Ok(Self {
            store: Arc::new(tokio::sync::Mutex::new(Store {
                conn,
                rows,
                hour_start: 0,
                new_this_hour: 0,
            })),
            max_rows,
            max_new_per_hour,
        })
    }

    /// Store a node's email, or remove it when `email` is empty; the row stays as a tombstone so its timestamp still guards
    /// against replays. A node with a channel is never held back by the cap or the hourly limit.
    pub async fn save(
        &self,
        node_id: &str,
        email: &str,
        signed_ts: i64,
        has_channel: bool,
    ) -> SaveOutcome {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.save_at_time(node_id, email, signed_ts, has_channel, now)
            .await
    }

    pub(crate) async fn save_at_time(
        &self,
        node_id: &str,
        email: &str,
        signed_ts: i64,
        has_channel: bool,
        now: i64,
    ) -> SaveOutcome {
        let store = Arc::clone(&self.store).lock_owned().await;
        let (node_id, email) = (node_id.to_owned(), email.to_owned());
        let limits = (self.max_rows, self.max_new_per_hour);
        // SQLite can wait on its file lock, so the write runs on a blocking thread, never on a runtime worker.
        tokio::task::spawn_blocking(move || {
            let mut store = store;
            save_at(
                &mut store,
                &node_id,
                &email,
                signed_ts,
                has_channel,
                now,
                limits,
            )
            .unwrap_or_else(|e| {
                error!("[emails] Failed to save for node {}: {}", node_id, e);
                SaveOutcome::Failed
            })
        })
        .await
        .unwrap_or(SaveOutcome::Failed)
    }
}

fn save_at(
    store: &mut Store,
    node_id: &str,
    email: &str,
    signed_ts: i64,
    has_channel: bool,
    now: i64,
    (max_rows, max_new_per_hour): (i64, i64),
) -> rusqlite::Result<SaveOutcome> {
    if now - store.hour_start >= HOUR_SECS {
        store.hour_start = now;
        store.new_this_hour = 0;
    }
    let tx = store
        .conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let last: Option<i64> = tx
        .query_row(
            "SELECT signed_ts FROM emails WHERE node_id = ?1",
            rusqlite::params![node_id],
            |row| row.get(0),
        )
        .optional()?;
    let is_new = match last {
        Some(last) if signed_ts <= last => return Ok(SaveOutcome::Stale),
        Some(_) => false,
        None if has_channel => true,
        None if store.rows >= max_rows => return Ok(SaveOutcome::Full),
        None if store.new_this_hour >= max_new_per_hour => return Ok(SaveOutcome::Limited),
        None => true,
    };
    tx.execute(
        "INSERT INTO emails (node_id, email, signed_ts, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(node_id) DO UPDATE SET email = excluded.email, signed_ts = excluded.signed_ts, updated_at = excluded.updated_at",
        rusqlite::params![node_id, email, signed_ts, now],
    )?;
    tx.commit()?;
    if is_new {
        store.rows += 1;
        if !has_channel {
            store.new_this_hour += 1;
        }
    }
    Ok(SaveOutcome::Saved)
}

#[cfg(test)]
impl Emails {
    pub(crate) fn in_memory(max_rows: i64) -> Self {
        Self::in_memory_limited(max_rows, i64::MAX)
    }

    pub(crate) fn in_memory_limited(max_rows: i64, max_new_per_hour: i64) -> Self {
        Self::with_connection(
            Connection::open_in_memory().unwrap(),
            max_rows,
            max_new_per_hour,
        )
        .unwrap()
    }

    pub(crate) async fn email_of(&self, node_id: &str) -> Option<String> {
        let store = self.store.lock().await;
        store
            .conn
            .query_row(
                "SELECT email FROM emails WHERE node_id = ?1",
                rusqlite::params![node_id],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_older_signed_change_never_replaces_a_newer_one() {
        let emails = Emails::in_memory(10);
        assert_eq!(
            emails.save("n1", "a@example.com", 100, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n1", "b@example.com", 200, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n1", "a@example.com", 100, false).await,
            SaveOutcome::Stale
        );
        assert_eq!(
            emails.save("n1", "c@example.com", 200, false).await,
            SaveOutcome::Stale,
            "the same timestamp is a replay too"
        );
        assert_eq!(
            emails.email_of("n1").await.as_deref(),
            Some("b@example.com")
        );
    }

    #[tokio::test]
    async fn a_removed_email_cannot_be_restored_by_replaying_its_save() {
        let emails = Emails::in_memory(10);
        assert_eq!(
            emails.save("n1", "a@example.com", 100, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(emails.save("n1", "", 200, false).await, SaveOutcome::Saved);
        assert_eq!(
            emails.save("n1", "a@example.com", 100, false).await,
            SaveOutcome::Stale
        );
        assert_eq!(
            emails.email_of("n1").await.as_deref(),
            Some(""),
            "the tombstone stays"
        );
        assert_eq!(
            emails.save("n1", "new@example.com", 300, false).await,
            SaveOutcome::Saved
        );
    }

    #[tokio::test]
    async fn a_full_store_refuses_new_nodes_but_not_stored_ones() {
        let emails = Emails::in_memory(2);
        assert_eq!(
            emails.save("n1", "a@example.com", 100, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(emails.save("n2", "", 100, false).await, SaveOutcome::Saved);
        assert_eq!(
            emails.save("n3", "c@example.com", 100, false).await,
            SaveOutcome::Full
        );
        assert_eq!(emails.email_of("n3").await, None);
        assert_eq!(emails.save("n1", "", 200, false).await, SaveOutcome::Saved);
        assert_eq!(
            emails.save("n2", "b@example.com", 200, false).await,
            SaveOutcome::Saved
        );
    }

    const HOUR: i64 = 3_600;

    #[tokio::test]
    async fn a_flood_of_new_nodes_without_a_channel_is_limited_per_hour() {
        let emails = Emails::in_memory_limited(100, 2);
        let t = 1_000_000;
        assert_eq!(
            emails
                .save_at_time("n1", "a@example.com", 1, false, t)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails
                .save_at_time("n2", "a@example.com", 1, false, t + 1)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails
                .save_at_time("n3", "a@example.com", 1, false, t + 2)
                .await,
            SaveOutcome::Limited
        );
        assert_eq!(emails.email_of("n3").await, None);
        assert_eq!(
            emails
                .save_at_time("n3", "a@example.com", 1, false, t + HOUR)
                .await,
            SaveOutcome::Saved,
            "a new hour opens new room"
        );
    }

    #[tokio::test]
    async fn a_node_with_a_channel_saves_past_the_cap_and_the_hourly_limit() {
        let emails = Emails::in_memory_limited(1, 1);
        let t = 1_000_000;
        assert_eq!(
            emails
                .save_at_time("stranger", "a@example.com", 1, false, t)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails
                .save_at_time("flood", "a@example.com", 1, false, t)
                .await,
            SaveOutcome::Full
        );
        assert_eq!(
            emails
                .save_at_time("customer", "c@example.com", 1, true, t)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.email_of("customer").await.as_deref(),
            Some("c@example.com")
        );
    }

    #[tokio::test]
    async fn changes_by_stored_nodes_never_use_up_the_hourly_room() {
        let emails = Emails::in_memory_limited(100, 1);
        let t = 1_000_000;
        assert_eq!(
            emails
                .save_at_time("n1", "a@example.com", 1, false, t)
                .await,
            SaveOutcome::Saved
        );
        for ts in 2..10 {
            assert_eq!(
                emails
                    .save_at_time("n1", "a@example.com", ts, false, t)
                    .await,
                SaveOutcome::Saved
            );
        }
        assert_eq!(
            emails
                .save_at_time("n2", "a@example.com", 1, false, t)
                .await,
            SaveOutcome::Limited
        );
    }

    #[tokio::test]
    async fn the_cap_counts_rows_already_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Emails::open_limited(dir.path(), 1, 10).unwrap();
        assert_eq!(
            store.save("n1", "a@example.com", 1, false).await,
            SaveOutcome::Saved
        );
        drop(store);
        let reopened = Emails::open_limited(dir.path(), 1, 10).unwrap();
        assert_eq!(
            reopened.save("n2", "a@example.com", 1, false).await,
            SaveOutcome::Full
        );
    }

    // Email is optional: a store that cannot open turns the feature off instead of stopping the LSP.
    #[test]
    fn a_store_that_cannot_open_is_off_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("emails.db")).unwrap();
        assert!(Emails::open_or_off(dir.path()).is_none());
        let healthy = tempfile::tempdir().unwrap();
        assert!(Emails::open_or_off(healthy.path()).is_some());
    }

    #[tokio::test]
    async fn the_store_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Emails::open(dir.path())
                .unwrap()
                .save("n1", "a@example.com", 100, false)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            Emails::open(dir.path())
                .unwrap()
                .save("n1", "a@example.com", 100, false)
                .await,
            SaveOutcome::Stale
        );
    }
}
