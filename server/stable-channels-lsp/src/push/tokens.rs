//! sqlite-backed device-token store for push notifications.

use rusqlite::Connection;
use std::path::Path;
use tracing::{error, info};

pub struct TokenInfo {
    pub token: String,
    pub platform: String,
    pub environment: String,
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
            last_seen     INTEGER,
            verified      INTEGER NOT NULL DEFAULT 0
        )",
    ) {
        error!("[push-tokens] Failed to create table: {}", e);
    }

    // Migrate pre-existing prod DBs that predate the `verified` column. Errors
    // with "duplicate column name" once migrated — expected and ignored.
    let _ = conn.execute(
        "ALTER TABLE push_tokens ADD COLUMN verified INTEGER NOT NULL DEFAULT 0",
        [],
    );
}

pub fn save_token(
    data_dir: &str,
    token: &str,
    platform: &str,
    node_id: &str,
    environment: &str,
    verified: bool,
) {
    let path = db_path(data_dir);
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(e) => {
            error!("[push-tokens] Failed to open DB: {}", e);
            return;
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    match conn.execute(
        "INSERT INTO push_tokens
         (device_token, platform, registered_at, node_id, environment, last_seen, verified)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(device_token) DO UPDATE SET
            platform = excluded.platform,
            registered_at = excluded.registered_at,
            node_id = excluded.node_id,
            environment = excluded.environment,
            last_seen = excluded.last_seen,
            verified = CASE
                WHEN excluded.verified = 1 THEN 1
                WHEN push_tokens.node_id = excluded.node_id THEN push_tokens.verified
                ELSE 0
            END",
        rusqlite::params![
            token,
            platform,
            now,
            node_id,
            environment,
            now,
            verified as i64
        ],
    ) {
        Ok(_) => info!(
            "[push-tokens] Saved token for node {} ({}, verified={})",
            node_id, platform, verified
        ),
        Err(e) => error!("[push-tokens] Failed to save token: {}", e),
    }
}

/// Current state of a node's push registration, used to detect hijack attempts
/// before a write: the device_token that would currently win at notify time,
/// and whether any signed (verified) token exists for the node.
pub struct NodeTokenState {
    pub active_token: Option<String>,
    pub has_verified: bool,
}

pub fn node_token_state(data_dir: &str, node_id: &str) -> NodeTokenState {
    let path = db_path(data_dir);
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(_) => {
            return NodeTokenState {
                active_token: None,
                has_verified: false,
            }
        }
    };

    // Same selection order as load_token_for_node so `active_token` reflects
    // who would actually receive the next push.
    let active_token = conn
        .query_row(
            "SELECT device_token FROM push_tokens
             WHERE node_id = ?1
             ORDER BY verified DESC, last_seen DESC LIMIT 1",
            rusqlite::params![node_id],
            |row| row.get::<_, String>(0),
        )
        .ok();

    let has_verified = conn
        .query_row(
            "SELECT 1 FROM push_tokens WHERE node_id = ?1 AND verified = 1 LIMIT 1",
            rusqlite::params![node_id],
            |_| Ok(()),
        )
        .is_ok();

    NodeTokenState {
        active_token,
        has_verified,
    }
}

pub fn load_token_for_node(data_dir: &str, node_id: &str) -> Option<TokenInfo> {
    let path = db_path(data_dir);
    let conn = Connection::open(&path).ok()?;

    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        - (30 * 86400);

    // verified DESC first: a signed (node-owned) token always wins over an
    // unsigned one, so an unauthenticated legacy registration cannot hijack a
    // node that has registered with a signature. See issue #162.
    conn.query_row(
        "SELECT device_token, platform, environment FROM push_tokens
         WHERE node_id = ?1 AND last_seen > ?2
         ORDER BY verified DESC, last_seen DESC LIMIT 1",
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
        save_token(&data_dir, "abc123", "ios", "node-pubkey", "sandbox", false);
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
        save_token(&data_dir, "abc", "ios", "node-x", "sandbox", false);
        save_token(&data_dir, "abc", "ios", "node-x", "production", false);
        let info = load_token_for_node(&data_dir, "node-x").expect("token must exist");
        assert_eq!(info.environment, "production");
    }

    #[test]
    fn missing_node_returns_none() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        assert!(load_token_for_node(&data_dir, "nope").is_none());
    }

    #[test]
    fn verified_token_wins_over_newer_unsigned() {
        // The hijack scenario from #162: a legit signed registration, then a
        // later unsigned registration for the same node with the attacker's
        // token. The signed one must still win at notify time.
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        save_token(&data_dir, "owner-tok", "ios", "node-v", "production", true);
        save_token(
            &data_dir,
            "attacker-tok",
            "ios",
            "node-v",
            "production",
            false,
        );
        let info = load_token_for_node(&data_dir, "node-v").expect("token must exist");
        assert_eq!(info.token, "owner-tok");
    }

    #[test]
    fn unsigned_reregister_does_not_downgrade_same_verified_token() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        save_token(&data_dir, "owner-tok", "ios", "node-v", "production", true);
        save_token(&data_dir, "owner-tok", "ios", "node-v", "production", false);

        let state = node_token_state(&data_dir, "node-v");
        assert_eq!(state.active_token.as_deref(), Some("owner-tok"));
        assert!(state.has_verified);

        save_token(
            &data_dir,
            "attacker-tok",
            "ios",
            "node-v",
            "production",
            false,
        );
        let info = load_token_for_node(&data_dir, "node-v").expect("token must exist");
        assert_eq!(info.token, "owner-tok");
    }

    #[test]
    fn verified_status_does_not_carry_to_different_node_for_same_device_token() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        save_token(
            &data_dir,
            "same-device-token",
            "ios",
            "old-node",
            "production",
            true,
        );
        save_token(
            &data_dir,
            "same-device-token",
            "ios",
            "new-node",
            "production",
            false,
        );

        let old_state = node_token_state(&data_dir, "old-node");
        assert!(old_state.active_token.is_none());
        assert!(!old_state.has_verified);

        let new_state = node_token_state(&data_dir, "new-node");
        assert_eq!(new_state.active_token.as_deref(), Some("same-device-token"));
        assert!(!new_state.has_verified);
    }

    #[test]
    fn node_token_state_reports_active_and_verified() {
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        assert!(node_token_state(&data_dir, "n").active_token.is_none());

        save_token(&data_dir, "t1", "ios", "n", "production", false);
        let s = node_token_state(&data_dir, "n");
        assert_eq!(s.active_token.as_deref(), Some("t1"));
        assert!(!s.has_verified);

        save_token(&data_dir, "t2", "ios", "n", "production", true);
        let s = node_token_state(&data_dir, "n");
        assert_eq!(s.active_token.as_deref(), Some("t2"));
        assert!(s.has_verified);
    }

    #[test]
    fn migration_is_idempotent_on_reinit() {
        // Simulate an existing DB re-opened: init_db runs the ALTER again,
        // which must not wipe rows or error fatally.
        let dir = tempdir().unwrap();
        init_db(dir.path());
        let data_dir = dir.path().to_string_lossy().to_string();
        save_token(&data_dir, "tok", "ios", "n", "production", true);
        init_db(dir.path()); // second call — ALTER now hits duplicate column
        let info = load_token_for_node(&data_dir, "n").expect("row survives reinit");
        assert_eq!(info.token, "tok");
    }
}
