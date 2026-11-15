//! sqlite-backed store of the optional email each wallet shares, keyed by its Lightning node id (emails.db).

use std::path::Path;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};
use tracing::error;

use crate::fixed_window::FixedWindow;

/// Most nodes kept; past this a node with no channel is refused, while stored nodes can still change or remove theirs.
const MAX_EMAILS: i64 = 100_000;
/// Most new nodes with no channel stored per hour, so a flood of fresh keys takes weeks, not minutes, to reach the cap.
const MAX_NEW_STRANGERS_PER_HOUR: u32 = 200;
const HOUR_SECS: u64 = 3_600;
const MAX_EMAIL_LEN: usize = 254;
const MAX_EMAIL_LOCAL_LEN: usize = 64;
/// Longest DNS label.
const MAX_LABEL_LEN: usize = 63;

/// A shareable address, already lowercased by the wallet: a dot-separated local part of plain address characters, '@', and a
/// dotted ASCII domain with a TLD of two or more characters. Quotes, markup and non-ASCII are refused so a stored address is
/// safe to show or mail as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailAddress(String);

impl EmailAddress {
    /// The address, or None when `email` breaks the rule above.
    pub fn parse(email: &str) -> Option<Self> {
        if email.len() > MAX_EMAIL_LEN {
            return None;
        }
        let (local, domain) = email.split_once('@')?;
        let local_char = |c: char| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || "!#$%&'*+/=?^_`{|}~-".contains(c)
        };
        let local_ok = local.len() <= MAX_EMAIL_LOCAL_LEN
            && local
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(local_char));
        let label_ok = |l: &str| {
            !l.is_empty()
                && l.len() <= MAX_LABEL_LEN
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        };
        let labels: Vec<&str> = domain.split('.').collect();
        let valid = local_ok
            && labels.len() >= 2
            && labels.iter().all(|l| label_ok(l))
            && labels.last().is_some_and(|tld| tld.len() >= 2);
        valid.then(|| Self(email.to_owned()))
    }
}

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
    /// New nodes with no channel stored this hour.
    new_strangers: FixedWindow,
}

pub struct Emails {
    /// Writes take turns on the one connection from a blocking thread.
    store: Arc<tokio::sync::Mutex<Store>>,
    max_rows: i64,
    max_new_per_hour: u32,
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
        max_new_per_hour: u32,
    ) -> rusqlite::Result<Self> {
        let path = data_dir.join("emails.db");
        create_owner_only(&path).map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                Some(e.to_string()),
            )
        })?;
        Self::with_connection(Connection::open(path)?, max_rows, max_new_per_hour)
    }

    fn with_connection(
        conn: Connection,
        max_rows: i64,
        max_new_per_hour: u32,
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
                new_strangers: FixedWindow::default(),
            })),
            max_rows,
            max_new_per_hour,
        })
    }

    /// Store a node's email, or remove it when `email` is None; the row stays as a tombstone (an empty email) so its timestamp
    /// still guards against replays. A node with a channel is never held back by the cap or the hourly limit.
    pub async fn save(
        &self,
        node_id: &str,
        email: Option<EmailAddress>,
        signed_ts: i64,
        has_channel: bool,
    ) -> SaveOutcome {
        self.save_at_time(
            node_id,
            email,
            signed_ts,
            has_channel,
            crate::revenue::now_secs(),
        )
        .await
    }

    pub(crate) async fn save_at_time(
        &self,
        node_id: &str,
        email: Option<EmailAddress>,
        signed_ts: i64,
        has_channel: bool,
        now: i64,
    ) -> SaveOutcome {
        let store = Arc::clone(&self.store).lock_owned().await;
        let (node_id, email) = (node_id.to_owned(), email.map(|e| e.0).unwrap_or_default());
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

/// Create emails.db readable by its owner only, or make an existing one so; SQLite gives its journal the same mode.
fn create_owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    crate::secret_file::make_owner_only(&file).map(|_| ())
}

fn save_at(
    store: &mut Store,
    node_id: &str,
    email: &str,
    signed_ts: i64,
    has_channel: bool,
    now: i64,
    (max_rows, max_new_per_hour): (i64, u32),
) -> rusqlite::Result<SaveOutcome> {
    // The clock is never before 1970, so the window's unsigned seconds lose nothing.
    let window_now = u64::try_from(now).unwrap_or(0);
    store.new_strangers.roll(window_now, HOUR_SECS);
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
        None if store
            .new_strangers
            .spent(window_now, HOUR_SECS, max_new_per_hour) =>
        {
            return Ok(SaveOutcome::Limited)
        }
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
            // Room was checked above, so this always counts.
            store
                .new_strangers
                .allow(window_now, HOUR_SECS, max_new_per_hour);
        }
    }
    Ok(SaveOutcome::Saved)
}

#[cfg(test)]
impl Emails {
    pub(crate) fn in_memory(max_rows: i64) -> Self {
        Self::in_memory_limited(max_rows, u32::MAX)
    }

    pub(crate) fn in_memory_limited(max_rows: i64, max_new_per_hour: u32) -> Self {
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

    /// The change a wallet sends: an empty email removes the address.
    fn email(email: &str) -> Option<EmailAddress> {
        (!email.is_empty()).then(|| EmailAddress::parse(email).unwrap())
    }

    #[tokio::test]
    async fn an_older_signed_change_never_replaces_a_newer_one() {
        let emails = Emails::in_memory(10);
        assert_eq!(
            emails.save("n1", email("a@example.com"), 100, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n1", email("b@example.com"), 200, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n1", email("a@example.com"), 100, false).await,
            SaveOutcome::Stale
        );
        assert_eq!(
            emails.save("n1", email("c@example.com"), 200, false).await,
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
            emails.save("n1", email("a@example.com"), 100, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n1", email(""), 200, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n1", email("a@example.com"), 100, false).await,
            SaveOutcome::Stale
        );
        assert_eq!(
            emails.email_of("n1").await.as_deref(),
            Some(""),
            "the tombstone stays"
        );
        assert_eq!(
            emails
                .save("n1", email("new@example.com"), 300, false)
                .await,
            SaveOutcome::Saved
        );
    }

    #[tokio::test]
    async fn a_full_store_refuses_new_nodes_but_not_stored_ones() {
        let emails = Emails::in_memory(2);
        assert_eq!(
            emails.save("n1", email("a@example.com"), 100, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n2", email(""), 100, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n3", email("c@example.com"), 100, false).await,
            SaveOutcome::Full
        );
        assert_eq!(emails.email_of("n3").await, None);
        assert_eq!(
            emails.save("n1", email(""), 200, false).await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails.save("n2", email("b@example.com"), 200, false).await,
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
                .save_at_time("n1", email("a@example.com"), 1, false, t)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails
                .save_at_time("n2", email("a@example.com"), 1, false, t + 1)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails
                .save_at_time("n3", email("a@example.com"), 1, false, t + 2)
                .await,
            SaveOutcome::Limited
        );
        assert_eq!(emails.email_of("n3").await, None);
        assert_eq!(
            emails
                .save_at_time("n3", email("a@example.com"), 1, false, t + HOUR)
                .await,
            SaveOutcome::Saved,
            "a new hour opens new room"
        );
    }

    #[tokio::test]
    async fn a_clock_stepped_back_restarts_the_hourly_window() {
        let emails = Emails::in_memory_limited(100, 1);
        let t = 1_000_000;
        assert_eq!(
            emails
                .save_at_time("n1", email("a@example.com"), 1, false, t)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails
                .save_at_time("n2", email("a@example.com"), 1, false, t - 10)
                .await,
            SaveOutcome::Saved,
            "a clock stepped back opens a new window, as RequestWindow does"
        );
        assert_eq!(
            emails
                .save_at_time("n3", email("a@example.com"), 1, false, t - 9)
                .await,
            SaveOutcome::Limited
        );
    }

    #[tokio::test]
    async fn a_node_with_a_channel_saves_past_the_cap_and_the_hourly_limit() {
        let emails = Emails::in_memory_limited(1, 1);
        let t = 1_000_000;
        assert_eq!(
            emails
                .save_at_time("stranger", email("a@example.com"), 1, false, t)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            emails
                .save_at_time("flood", email("a@example.com"), 1, false, t)
                .await,
            SaveOutcome::Full
        );
        assert_eq!(
            emails
                .save_at_time("customer", email("c@example.com"), 1, true, t)
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
                .save_at_time("n1", email("a@example.com"), 1, false, t)
                .await,
            SaveOutcome::Saved
        );
        for ts in 2..10 {
            assert_eq!(
                emails
                    .save_at_time("n1", email("a@example.com"), ts, false, t)
                    .await,
                SaveOutcome::Saved
            );
        }
        assert_eq!(
            emails
                .save_at_time("n2", email("a@example.com"), 1, false, t)
                .await,
            SaveOutcome::Limited
        );
    }

    #[tokio::test]
    async fn the_cap_counts_rows_already_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Emails::open_limited(dir.path(), 1, 10).unwrap();
        assert_eq!(
            store.save("n1", email("a@example.com"), 1, false).await,
            SaveOutcome::Saved
        );
        drop(store);
        let reopened = Emails::open_limited(dir.path(), 1, 10).unwrap();
        assert_eq!(
            reopened.save("n2", email("a@example.com"), 1, false).await,
            SaveOutcome::Full
        );
    }

    #[test]
    fn emails_db_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emails.db");
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        Emails::open(dir.path()).unwrap();
        assert_eq!(mode(&path), 0o600);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        Emails::open(dir.path()).unwrap();
        assert_eq!(mode(&path), 0o600, "an existing file is made owner only");
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
                .save("n1", email("a@example.com"), 100, false)
                .await,
            SaveOutcome::Saved
        );
        assert_eq!(
            Emails::open(dir.path())
                .unwrap()
                .save("n1", email("a@example.com"), 100, false)
                .await,
            SaveOutcome::Stale
        );
    }

    fn valid(email: &str) -> bool {
        EmailAddress::parse(email).is_some()
    }

    #[test]
    fn email_rules() {
        for ok in [
            "a@b.co",
            "you@example.com",
            "first.last+tag@mail.example.org",
            &format!("{}@example.com", "a".repeat(64)),
        ] {
            assert!(valid(ok), "{ok}");
        }
        let too_long = format!("a@{}.com", "b".repeat(250));
        let long_local = format!("{}@example.com", "a".repeat(65));
        for bad in [
            "",
            "a",
            "@example.com",
            "a@",
            "a@b",
            "a@b.c",
            "a@@b.co",
            "a@b@c.co",
            "a@.co",
            "a@b..co",
            "a@b.co.",
            " a@b.co",
            "a@b.co ",
            "a b@c.co",
            "a@b\u{7}.co",
            "A@b.co",
            "a@B.co",
            &too_long,
            &long_local,
        ] {
            assert!(!valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_domain_label_is_at_most_63_bytes() {
        assert!(valid(&format!("a@{}.com", "b".repeat(MAX_LABEL_LEN))));
        assert!(!valid(&format!("a@{}.com", "b".repeat(MAX_LABEL_LEN + 1))));
        assert!(!valid(&format!(
            "a@example.{}",
            "c".repeat(MAX_LABEL_LEN + 1)
        )));
    }

    #[test]
    fn an_email_with_markup_quotes_or_header_breaks_is_refused() {
        for bad in [
            "a<b@example.com",
            "a>b@example.com",
            "\"a\"@example.com",
            "a@exa<mple.com",
            "a@example.com/x",
            "a@exa_mple.com",
            "a@-example.com",
            "a@example-.com",
            "a\\b@example.com",
            "a@b.co\r\nbcc: x@y.co",
            "ä@example.com",
            "a@exämple.com",
            ".a@example.com",
            "a.@example.com",
            "a..b@example.com",
        ] {
            assert!(!valid(bad), "{bad:?}");
        }
        for ok in [
            "o'brien@example.com",
            "a_b-c+d@sub-domain.example.co",
            "x@xn--bcher-kva.example",
        ] {
            assert!(valid(ok), "{ok}");
        }
    }
}
