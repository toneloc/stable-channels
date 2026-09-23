//! Durable, bounded discovery and retry work for channel on-chain audit rows.
//! The ledger remains append-only; this projection can also ingest reconnect/legacy writers.

use rusqlite::{params, Connection, OptionalExtension, Result};
use serde_json::Value;

use crate::ledger::{self, AppendOutcome, LedgerEventDraft};

fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS onchain_audit_progress (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            ledger_cursor INTEGER NOT NULL DEFAULT 0,
            check_sequence INTEGER NOT NULL DEFAULT 0,
            page_token TEXT, boundary_id TEXT, head_id TEXT
         );
         INSERT OR IGNORE INTO onchain_audit_progress (singleton) VALUES (1);
         CREATE TABLE IF NOT EXISTS onchain_audit_work (
            payment_id TEXT PRIMARY KEY,
            event_id INTEGER NOT NULL,
            terminal INTEGER NOT NULL,
            checked_sequence INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_onchain_audit_work
            ON onchain_audit_work(terminal, checked_sequence, payment_id);
         CREATE INDEX IF NOT EXISTS idx_onchain_audit_ledger_scan
            ON ledger_events(id) WHERE event_type = 'CHANNEL_ONCHAIN_TX';
         CREATE INDEX IF NOT EXISTS idx_onchain_audit_ledger_latest
            ON ledger_events(json_extract(detail_json, '$.payment_id'),
                COALESCE(json_extract(detail_json, '$.latest_update_timestamp'), 0) DESC, id DESC)
            WHERE event_type = 'CHANNEL_ONCHAIN_TX';",
    )
}

fn latest(conn: &Connection, payment_id: &str) -> Result<Option<(i64, String, Value)>> {
    let row: Option<(i64, String, String)> = conn.query_row(
        "SELECT id, status, detail_json FROM ledger_events
         WHERE event_type = 'CHANNEL_ONCHAIN_TX'
           AND json_extract(detail_json, '$.payment_id') = ?1
         ORDER BY COALESCE(json_extract(detail_json, '$.latest_update_timestamp'), 0) DESC, id DESC
         LIMIT 1",
        [payment_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    row.map(|(id, status, json)| {
        serde_json::from_str(&json)
            .map(|detail| (id, status, detail))
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
    }).transpose()
}

fn project_latest(conn: &Connection, payment_id: &str) -> Result<()> {
    if let Some((id, status, detail)) = latest(conn, payment_id)? {
        let terminal = match detail.get("ldk_status").and_then(Value::as_str) {
            Some("SUCCEEDED" | "FAILED") => true,
            Some(_) => false,
            // Preserve legacy terminal states; the initial discovery pass revalidates them.
            None => status == "completed" || status == "failed",
        };
        conn.execute(
            "INSERT INTO onchain_audit_work (payment_id, event_id, terminal)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(payment_id) DO UPDATE SET
                event_id = excluded.event_id, terminal = excluded.terminal",
            params![payment_id, id, terminal],
        )?;
    }
    Ok(())
}

impl super::Database {
    /// (next opaque page token, previous completed head, head of this unfinished pass).
    /// The previous head is a stop boundary, never advanced until every intervening page commits.
    pub fn onchain_audit_scan(&self) -> Result<(Option<String>, Option<String>, Option<String>)> {
        let conn = self.conn.lock().unwrap();
        init(&conn)?;
        conn.query_row(
            "SELECT page_token, boundary_id, head_id FROM onchain_audit_progress WHERE singleton = 1",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }

    /// Called only after all rows in the page were persisted (duplicates count as persisted).
    pub fn save_onchain_audit_scan(
        &self, page_token: Option<&str>, boundary_id: Option<&str>, head_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        init(&conn)?;
        conn.execute(
            "UPDATE onchain_audit_progress SET page_token = ?1, boundary_id = ?2, head_id = ?3
             WHERE singleton = 1",
            params![page_token, boundary_id, head_id],
        )?;
        Ok(())
    }

    /// Incrementally enroll ledger writers such as reconnect backfill. Fetch each payment's
    /// genuine latest state, not just the status of a historical row inside this bounded batch.
    pub fn refresh_onchain_audit_pending(&self, limit: usize) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        init(&conn)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let cursor: i64 = tx.query_row(
            "SELECT ledger_cursor FROM onchain_audit_progress WHERE singleton = 1", [], |r| r.get(0),
        )?;
        let rows = {
            let mut stmt = tx.prepare(
                "SELECT id, json_extract(detail_json, '$.payment_id') FROM ledger_events
                 WHERE event_type = 'CHANNEL_ONCHAIN_TX' AND id > ?1 ORDER BY id LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![cursor, limit.min(1000) as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            })?.collect::<Result<Vec<_>>>()?;
            rows
        };
        for (_, payment_id) in &rows {
            if let Some(id) = payment_id.as_deref().filter(|id| !id.is_empty()) {
                project_latest(&tx, id)?;
            }
        }
        if let Some((id, _)) = rows.last() {
            tx.execute("UPDATE onchain_audit_progress SET ledger_cursor = ?1 WHERE singleton = 1", [id])?;
        }
        tx.commit()
    }

    /// Bounded, restart-safe round robin. Rotating an attempted batch does not clear its retry
    /// obligation: missing details, RPC errors and ledger failures all remain nonterminal.
    pub fn onchain_audit_pending_ids(&self, limit: usize, rotate: bool) -> Result<Vec<String>> {
        let mut conn = self.conn.lock().unwrap();
        init(&conn)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ids = {
            let mut stmt = tx.prepare(
                "SELECT payment_id FROM onchain_audit_work WHERE terminal = 0
                 ORDER BY checked_sequence, payment_id LIMIT ?1",
            )?;
            let ids = stmt.query_map([limit.min(1000) as i64], |r| r.get(0))?.collect::<Result<Vec<String>>>()?;
            ids
        };
        if rotate && !ids.is_empty() {
            tx.execute("UPDATE onchain_audit_progress SET check_sequence = check_sequence + 1 WHERE singleton = 1", [])?;
            for id in &ids {
                tx.execute(
                    "UPDATE onchain_audit_work SET checked_sequence =
                        (SELECT check_sequence FROM onchain_audit_progress WHERE singleton = 1)
                     WHERE payment_id = ?1", [id],
                )?;
            }
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Commit the ledger observation and the retry obligation atomically, then mirror it.
    /// A -> B -> A within a single LDK timestamp second gets a local transition identity;
    /// identical snapshots still deduplicate, including snapshots written by backfill.
    pub fn append_onchain_audit_event(&self, draft: &LedgerEventDraft) -> Result<AppendOutcome> {
        let payment_id = draft.detail.get("payment_id").and_then(Value::as_str)
            .filter(|id| !id.is_empty()).ok_or(rusqlite::Error::InvalidQuery)?;
        if draft.event_type != "CHANNEL_ONCHAIN_TX" {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut conn = self.conn.lock().unwrap();
        init(&conn)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut draft = draft.clone();
        let previous = latest(&tx, payment_id)?;
        let state_key = draft.detail.get("onchain_state_key").and_then(Value::as_str);
        if let Some((id, _, detail)) = &previous {
            let older = draft.detail["latest_update_timestamp"].as_u64().unwrap_or(0)
                < detail["latest_update_timestamp"].as_u64().unwrap_or(0);
            if older || (state_key.is_some() && state_key == detail.get("onchain_state_key").and_then(Value::as_str)) {
                project_latest(&tx, payment_id)?;
                tx.commit()?;
                return Ok(AppendOutcome { event_id: *id, inserted: false });
            }
            if let Some(key) = draft.dedup_key.as_deref() {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM ledger_events WHERE dedup_key = ?1)", [key], |r| r.get(0),
                )?;
                if exists {
                    let key = format!("{key}:after:{id}");
                    draft.detail["dedup_key"] = Value::String(key.clone());
                    draft.dedup_key = Some(key);
                }
            }
        }
        let outcome = ledger::append_on_connection(&tx, &draft)?;
        project_latest(&tx, payment_id)?;
        tx.commit()?;
        drop(conn);
        if outcome.inserted {
            crate::audit::mirror_committed_ledger_event(&draft, outcome.event_id);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Database;
    use serde_json::json;

    fn observation(id: &str, status: &str, revision: u64, confirmation: &str) -> LedgerEventDraft {
        let key = format!("{id}:{status}:{revision}:{confirmation}");
        LedgerEventDraft::from_audit_event("CHANNEL_ONCHAIN_TX", json!({
            "payment_id": id, "status": status,
            "ldk_status": match status { "completed" => "SUCCEEDED", "failed" => "FAILED", _ => "PENDING" },
            "latest_update_timestamp": revision, "confirmation": confirmation,
            "onchain_state_key": key, "dedup_key": key,
        }))
    }

    #[test]
    fn latest_states_exclude_over_200_historical_pending_rows() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..240 {
            db.append_ledger_event(&observation(&format!("p{i}"), "pending", 1, "unconfirmed")).unwrap();
        }
        for i in 0..240 {
            db.append_ledger_event(&observation(&format!("p{i}"), "completed", 2, "block-a")).unwrap();
        }
        // Even the first partial projection sees each payment's later terminal row.
        db.refresh_onchain_audit_pending(100).unwrap();
        assert!(db.onchain_audit_pending_ids(100, false).unwrap().is_empty());
        db.refresh_onchain_audit_pending(500).unwrap();
        assert!(db.onchain_audit_pending_ids(100, false).unwrap().is_empty());
    }

    #[test]
    fn late_backfill_enrolls_and_older_replays_do_not_reopen_terminal_work() {
        let db = Database::open_in_memory().unwrap();
        db.refresh_onchain_audit_pending(500).unwrap();
        assert!(db.onchain_audit_pending_ids(100, false).unwrap().is_empty());
        db.append_ledger_event(&observation("late", "pending", 20, "block-a")).unwrap();
        db.refresh_onchain_audit_pending(500).unwrap();
        assert_eq!(db.onchain_audit_pending_ids(100, false).unwrap(), vec!["late"]);
        db.append_onchain_audit_event(&observation("late", "completed", 30, "block-a")).unwrap();
        // A reconnect response obtained before the terminal poll may finish writing afterwards.
        db.append_ledger_event(&observation("late", "pending", 21, "block-a")).unwrap();
        db.refresh_onchain_audit_pending(500).unwrap();
        assert!(db.onchain_audit_pending_ids(100, false).unwrap().is_empty());
    }

    #[test]
    fn failed_insert_preserves_retry_until_terminal_row_commits() {
        let db = Database::open_in_memory().unwrap();
        db.append_onchain_audit_event(&observation("retry", "pending", 1, "unconfirmed")).unwrap();
        db.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER reject_onchain BEFORE INSERT ON ledger_events
             BEGIN SELECT RAISE(FAIL, 'injected ledger failure'); END;",
        ).unwrap();
        let completed = observation("retry", "completed", 2, "block-a");
        assert!(db.append_onchain_audit_event(&completed).is_err());
        assert_eq!(db.onchain_audit_pending_ids(100, true).unwrap(), vec!["retry"]);
        assert_eq!(db.onchain_audit_pending_ids(100, false).unwrap(), vec!["retry"]);
        db.conn.lock().unwrap().execute_batch("DROP TRIGGER reject_onchain").unwrap();
        assert!(db.append_onchain_audit_event(&completed).unwrap().inserted);
        assert!(!db.append_onchain_audit_event(&completed).unwrap().inserted);
        assert!(db.onchain_audit_pending_ids(100, false).unwrap().is_empty());
    }

    #[test]
    fn reorg_and_same_second_reconfirmation_are_distinct_but_replays_deduplicate() {
        let db = Database::open_in_memory().unwrap();
        let a = observation("reorg", "pending", 1, "block-a");
        let b = observation("reorg", "pending", 1, "unconfirmed");
        for row in [&a, &b, &a, &b, &a] {
            assert!(db.append_onchain_audit_event(row).unwrap().inserted);
            assert!(!db.append_onchain_audit_event(row).unwrap().inserted);
            assert_eq!(db.onchain_audit_pending_ids(100, false).unwrap(), vec!["reorg"]);
        }
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM ledger_events", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn pending_round_robin_and_unfinished_page_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let first_batch = {
            let db = Database::open(dir.path()).unwrap();
            for i in 0..230 {
                db.append_ledger_event(&observation(&format!("p{i:03}"), "pending", 1, "unconfirmed")).unwrap();
            }
            db.refresh_onchain_audit_pending(500).unwrap();
            db.save_onchain_audit_scan(Some("opaque-next"), Some("old-head"), Some("new-head")).unwrap();
            db.onchain_audit_pending_ids(100, true).unwrap()
        };
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(db.onchain_audit_scan().unwrap(), (Some("opaque-next".into()), Some("old-head".into()), Some("new-head".into())));
        let second = db.onchain_audit_pending_ids(100, true).unwrap();
        assert!(second.iter().all(|id| !first_batch.contains(id)));
        let third = db.onchain_audit_pending_ids(100, true).unwrap();
        let all: std::collections::HashSet<_> = first_batch.into_iter().chain(second).chain(third).collect();
        assert_eq!(all.len(), 230, "no 200-row cutoff or starvation after restart");
    }

    #[test]
    fn legacy_terminal_rows_supersede_pending_without_rewriting_history() {
        let db = Database::open_in_memory().unwrap();
        let pending = LedgerEventDraft::from_audit_event("CHANNEL_ONCHAIN_TX", json!({
            "payment_id": "legacy", "status": "pending", "dedup_key": "legacy:unconfirmed",
        }));
        let completed = LedgerEventDraft::from_audit_event("CHANNEL_ONCHAIN_TX", json!({
            "payment_id": "legacy", "status": "completed", "dedup_key": "legacy:confirmed",
        }));
        db.append_ledger_event(&pending).unwrap();
        db.append_ledger_event(&completed).unwrap();
        db.refresh_onchain_audit_pending(1).unwrap();
        assert!(db.onchain_audit_pending_ids(100, false).unwrap().is_empty());
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT detail_json FROM ledger_events ORDER BY id").unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(rows, vec![pending.detail.to_string(), completed.detail.to_string()]);
    }

    #[test]
    fn projection_failure_rolls_back_ledger_append() {
        let db = Database::open_in_memory().unwrap();
        db.onchain_audit_scan().unwrap();
        db.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER reject_work BEFORE INSERT ON onchain_audit_work
             BEGIN SELECT RAISE(FAIL, 'injected work failure'); END;",
        ).unwrap();
        assert!(db.append_onchain_audit_event(&observation("new", "pending", 1, "unconfirmed")).is_err());
        let conn = db.conn.lock().unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM ledger_events", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }
}
