//! sqlite-backed device-token store for push notifications.

use tracing::{error, info};
use rusqlite::Connection;
use std::path::Path;

pub struct TokenInfo {
    pub token: String,
    pub platform: String,
    pub environment: String,
}

/// Most tokens kept for one node; its older ones beyond these are dropped.
const MAX_TOKENS_PER_NODE: i64 = 5;
/// A token not seen for this long is never used to wake its node, so it is dropped.
pub(crate) const TOKEN_TTL_SECS: i64 = 30 * 86_400;

/// What became of a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    /// A held token not stored, because its node stored a token at or after it arrived.
    Superseded,
    Failed,
}

fn db_path(data_dir: &str) -> String {
    format!("{}/push_tokens.db", data_dir)
}

pub fn init_db(data_dir: &Path) {
    let path = db_path(&data_dir.to_string_lossy());
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(e) => {
            error!("[push-tokens] Failed to open DB: {}", e);
            return;
        }
    };

    if let Err(e) = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS push_tokens (
            device_token  TEXT PRIMARY KEY,
            platform      TEXT NOT NULL,
            registered_at INTEGER,
            node_id       TEXT,
            environment   TEXT,
            last_seen     INTEGER
        )",
    ) {
        error!("[push-tokens] Failed to create table: {}", e);
    }
    // Pruning and the per-node trim run on every registration, so they must not scan the table.
    if let Err(e) = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_push_tokens_node_seen ON push_tokens(node_id, last_seen);
         CREATE INDEX IF NOT EXISTS idx_push_tokens_last_seen ON push_tokens(last_seen);",
    ) {
        error!("[push-tokens] Failed to create indexes: {}", e);
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Save a token as seen now. Callers store tokens only for nodes with a channel with the LSP (see `registrations`), which bounds the store.
pub fn save_token(data_dir: &str, token: &str, platform: &str, node_id: &str, environment: &str) -> SaveOutcome {
    save_token_with_limits(data_dir, token, platform, node_id, environment, unix_now(), None, MAX_TOKENS_PER_NODE)
}

/// Save a token that was held since `arrived_at`, as seen then, unless its node has stored a token at or after that; whatever order the writes run in, a newer token stays the newest.
pub fn save_held_token(data_dir: &str, token: &str, platform: &str, node_id: &str, environment: &str, arrived_at: i64) -> SaveOutcome {
    save_token_with_limits(data_dir, token, platform, node_id, environment, unix_now(), Some(arrived_at), MAX_TOKENS_PER_NODE)
}

/// Save a token: tokens past their use are dropped first, a held one newer tokens superseded is skipped, and a node keeps only its newest tokens.
#[allow(clippy::too_many_arguments)]
fn save_token_with_limits(
    data_dir: &str,
    token: &str,
    platform: &str,
    node_id: &str,
    environment: &str,
    now: i64,
    held_since: Option<i64>,
    max_per_node: i64,
) -> SaveOutcome {
    let path = db_path(data_dir);
    let mut conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(e) => {
            error!("[push-tokens] Failed to open DB: {}", e);
            return SaveOutcome::Failed;
        }
    };
    let result = (|| -> rusqlite::Result<SaveOutcome> {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM push_tokens WHERE last_seen IS NULL OR last_seen <= ?1",
            rusqlite::params![now - TOKEN_TTL_SECS],
        )?;
        if let Some(arrived_at) = held_since {
            let superseded: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM push_tokens WHERE node_id = ?1 AND last_seen >= ?2)",
                rusqlite::params![node_id, arrived_at],
                |row| row.get(0),
            )?;
            if superseded {
                tx.commit()?;
                return Ok(SaveOutcome::Superseded);
            }
        }
        let seen_at = held_since.unwrap_or(now);
        tx.execute(
            "INSERT OR REPLACE INTO push_tokens
             (device_token, platform, registered_at, node_id, environment, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![token, platform, seen_at, node_id, environment, seen_at],
        )?;
        // Within the same second the token just registered counts as the newest.
        tx.execute(
            "DELETE FROM push_tokens WHERE node_id = ?1 AND device_token NOT IN (
                 SELECT device_token FROM push_tokens WHERE node_id = ?1
                 ORDER BY last_seen DESC, device_token = ?3 DESC, registered_at DESC LIMIT ?2)",
            rusqlite::params![node_id, max_per_node, token],
        )?;
        tx.commit()?;
        Ok(SaveOutcome::Saved)
    })();
    match result {
        Ok(SaveOutcome::Saved) => {
            info!("[push-tokens] Saved token for node {} ({})", node_id, platform);
            SaveOutcome::Saved
        },
        Ok(outcome) => outcome,
        Err(e) => {
            error!("[push-tokens] Failed to save token: {}", e);
            SaveOutcome::Failed
        },
    }
}

pub fn load_token_for_node(data_dir: &str, node_id: &str) -> Option<TokenInfo> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    load_token_for_node_at(data_dir, node_id, now)
}

fn load_token_for_node_at(data_dir: &str, node_id: &str, now: i64) -> Option<TokenInfo> {
    let path = db_path(data_dir);
    let conn = Connection::open(&path).ok()?;

    let cutoff = now - TOKEN_TTL_SECS;

    conn.query_row(
        "SELECT device_token, platform, environment FROM push_tokens
         WHERE node_id = ?1 AND last_seen > ?2
         ORDER BY last_seen DESC LIMIT 1",
        rusqlite::params![node_id, cutoff],
        |row| {
            Ok(TokenInfo {
                token: row.get(0)?,
                platform: row.get(1)?,
                environment: row
                    .get::<_, Option<String>>(2)?
                    .unwrap_or_else(|| "sandbox".to_string()),
            })
        },
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn save_then_load_returns_token() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        save_token(&data_dir, "abc123", "ios", "node-pubkey", "sandbox");
        let info = load_token_for_node(&data_dir, "node-pubkey").expect("token must exist");
        assert_eq!(info.token, "abc123");
        assert_eq!(info.platform, "ios");
        assert_eq!(info.environment, "sandbox");
    }

    #[test]
    fn replace_updates_token_and_environment() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        save_token(&data_dir, "abc", "ios", "node-x", "sandbox");
        save_token(&data_dir, "abc", "ios", "node-x", "production");
        let info = load_token_for_node(&data_dir, "node-x").expect("token must exist");
        assert_eq!(info.environment, "production");
    }

    fn stored_tokens(data_dir: &str) -> Vec<String> {
        let conn = Connection::open(db_path(data_dir)).unwrap();
        let mut stmt = conn.prepare("SELECT device_token FROM push_tokens ORDER BY device_token").unwrap();
        stmt.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    }

    #[test]
    fn a_node_keeps_only_its_newest_tokens() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        for i in 0..7 {
            assert_eq!(save_token_with_limits(&data_dir, &format!("tok{i}"), "android", "node-x", "", 1_000 + i, None, 5), SaveOutcome::Saved);
        }
        assert_eq!(stored_tokens(&data_dir), ["tok2", "tok3", "tok4", "tok5", "tok6"]);
        assert_eq!(load_token_for_node(&data_dir, "node-x").map(|info| info.token), None, "all were seen long ago");
    }

    #[test]
    fn tokens_not_seen_for_thirty_days_are_dropped() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        let now = 2_000_000_000;
        save_token_with_limits(&data_dir, "old", "ios", "node-a", "", now - TOKEN_TTL_SECS - 1, None, 5);
        save_token_with_limits(&data_dir, "new", "ios", "node-b", "", now, None, 5);
        assert_eq!(stored_tokens(&data_dir), ["new"], "a token that can no longer be used is not kept");
    }

    #[test]
    fn the_token_just_registered_is_never_the_one_trimmed() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        let now = 2_000_000_000;
        for token in ["z1", "z2", "z3", "z4", "z5", "a0"] {
            assert_eq!(save_token_with_limits(&data_dir, token, "ios", "node-x", "", now, None, 5), SaveOutcome::Saved);
        }
        assert!(stored_tokens(&data_dir).contains(&"a0".to_owned()), "the registration that just succeeded is kept");
        assert_eq!(stored_tokens(&data_dir).len(), 5);
    }

    #[test]
    fn missing_node_returns_none() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        assert!(load_token_for_node(&data_dir, "nope").is_none());
    }
}
