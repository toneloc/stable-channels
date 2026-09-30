//! Durable stream coverage, independent of payment activity and reconnect/error audit traffic.

use rusqlite::{params, Connection, OptionalExtension, Result, TransactionBehavior};

use super::Database;
use crate::ledger::{self, LedgerEventDraft};

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_stream_checkpoint (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            healthy_at_ms INTEGER,
            gap_started_ms INTEGER,
            gap_correlation_id TEXT,
            generation INTEGER NOT NULL,
            CHECK ((gap_started_ms IS NULL) = (gap_correlation_id IS NULL))
        );",
    )
}

/// Only explicit, unmatched stream-gap transitions establish a legacy outage. Normal historical
/// activity (or the newest retry/error) says nothing about whether the subscription was healthy.
fn legacy_open_gap(conn: &Connection) -> Result<Option<(i64, String)>> {
    let mut open = std::collections::HashMap::<String, i64>::new();
    let mut stmt = conn.prepare(
        "SELECT event_type, occurred_at_ms, detail_json FROM ledger_events
         WHERE event_type IN ('EVENT_STREAM_GAP_STARTED', 'EVENT_STREAM_GAP_CLOSED')
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
    })?;
    for row in rows {
        let (event, occurred_at_ms, json) = row?;
        let detail: serde_json::Value = serde_json::from_str(&json)
            .map_err(|error| rusqlite::Error::FromSqlConversionFailure(
                2, rusqlite::types::Type::Text, Box::new(error),
            ))?;
        let Some(id) = detail["correlation_id"].as_str().filter(|id| !id.is_empty()) else {
            continue;
        };
        if event == "EVENT_STREAM_GAP_CLOSED" {
            open.remove(id);
        } else {
            let started = detail["gap_started_ms"].as_i64().unwrap_or(occurred_at_ms);
            open.entry(id.to_owned()).and_modify(|old| *old = (*old).min(started)).or_insert(started);
        }
    }
    Ok(open.into_iter().map(|(id, start)| (start, id)).min())
}

impl Database {
    /// Open (or recover) a gap before subscribing. Repeated retries never move its start.
    /// After a crash, the last *stream-health* checkpoint bounds the new gap, even if the stream
    /// was idle. A legacy database without explicit gap evidence starts known coverage now and
    /// gets a separate unknown-coverage audit row; old activity is not proof of historical loss.
    /// The returned pair is (earliest unresolved start in milliseconds, correlation id).
    pub fn begin_event_stream_gap(&self, now_ms: i64) -> Result<(i64, String)> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        init_schema(&tx)?;
        let previous: Option<(Option<i64>, Option<i64>, Option<String>, i64)> = tx.query_row(
            "SELECT healthy_at_ms, gap_started_ms, gap_correlation_id, generation
             FROM event_stream_checkpoint WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional()?;
        if let Some((_, Some(start), Some(id), _)) = &previous {
            return Ok((*start, id.clone()));
        }

        let legacy = if previous.is_none() { legacy_open_gap(&tx)? } else { None };
        let generation = previous.as_ref().map_or(1, |row| row.3 + 1);
        let start = legacy.as_ref().map(|row| row.0)
            .or_else(|| previous.as_ref().and_then(|row| row.0))
            .unwrap_or(now_ms).min(now_ms);
        let id = legacy.as_ref().map(|row| row.1.clone())
            .unwrap_or_else(|| format!("event-stream-gap-{now_ms}-{generation}"));
        let mut audit_rows = Vec::new();
        if previous.is_none() && legacy.is_none() {
            audit_rows.push(LedgerEventDraft::from_audit_event(
                "EVENT_STREAM_COVERAGE_UNKNOWN",
                serde_json::json!({
                    "source": "stream_checkpoint",
                    "status": "unknown",
                    "history_before_ms": now_ms,
                    "reason": "no previous stream-health checkpoint or explicit open gap; historical coverage is unknown, not established loss",
                    "occurred_at_ms": now_ms,
                }),
            ));
        }
        if legacy.is_none() {
            audit_rows.push(LedgerEventDraft::from_audit_event(
                "EVENT_STREAM_GAP_STARTED",
                serde_json::json!({
                    "correlation_id": id,
                    "gap_started_ms": start,
                    "occurred_at_ms": now_ms,
                    "source": "stream_checkpoint",
                }),
            ));
        }
        tx.execute(
            "INSERT INTO event_stream_checkpoint
                (singleton, healthy_at_ms, gap_started_ms, gap_correlation_id, generation)
             VALUES (1, NULL, ?1, ?2, ?3)
             ON CONFLICT(singleton) DO UPDATE SET gap_started_ms = excluded.gap_started_ms,
                gap_correlation_id = excluded.gap_correlation_id, generation = excluded.generation",
            params![start, id, generation],
        )?;
        let mut committed = Vec::new();
        for draft in &audit_rows {
            committed.push(ledger::append_on_connection(&tx, draft)?);
        }
        tx.commit()?;
        drop(conn);
        for (draft, outcome) in audit_rows.iter().zip(committed) {
            if outcome.inserted {
                crate::audit::mirror_committed_ledger_event(draft, outcome.event_id);
            }
        }
        Ok((start, id))
    }

    /// Close exactly the reconciled gap and establish health atomically with its audit row.
    /// The caller must have persisted its reconciliation result and drained the still-active
    /// subscription. Explicit retention loss is carried in `reconciliation`, never hidden.
    /// False means the gap changed; a stale completion must not clear a newer outage.
    pub fn close_event_stream_gap(
        &self,
        correlation_id: &str,
        healthy_at_ms: i64,
        reconciliation: &serde_json::Value,
    ) -> Result<bool> {
        if !matches!(reconciliation["status"].as_str(), Some("completed" | "completed_with_loss")) {
            return Err(rusqlite::Error::InvalidParameterName(
                "cannot close stream gap with partial or unknown reconciliation".into(),
            ));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        init_schema(&tx)?;
        let changed = tx.execute(
            "UPDATE event_stream_checkpoint SET healthy_at_ms = ?1,
                gap_started_ms = NULL, gap_correlation_id = NULL
             WHERE singleton = 1 AND gap_correlation_id = ?2",
            params![healthy_at_ms, correlation_id],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        let draft = LedgerEventDraft::from_audit_event(
            "EVENT_STREAM_GAP_CLOSED",
            serde_json::json!({
                "correlation_id": correlation_id,
                "healthy_at_ms": healthy_at_ms,
                "occurred_at_ms": healthy_at_ms,
                "source": "stream_checkpoint",
                "reconciliation": reconciliation,
                "status": reconciliation["status"],
                "history_complete": reconciliation["history_complete"],
            }),
        );
        let outcome = ledger::append_on_connection(&tx, &draft)?;
        tx.commit()?;
        drop(conn);
        if outcome.inserted {
            crate::audit::mirror_committed_ledger_event(&draft, outcome.event_id);
        }
        Ok(true)
    }

    /// Throttled, idle-capable heartbeat. It cannot advance past an unresolved gap.
    /// The event loop supplies a time sampled before checking reader liveness and queue drainage.
    pub fn checkpoint_event_stream_health(&self, healthy_at_ms: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        init_schema(&conn)?;
        Ok(conn.execute(
            "UPDATE event_stream_checkpoint SET healthy_at_ms = ?1
             WHERE singleton = 1 AND gap_correlation_id IS NULL",
            [healthy_at_ms],
        )? == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed() -> serde_json::Value {
        serde_json::json!({"status": "completed", "history_complete": true})
    }

    fn audit(db: &Database, event: &str, ms: i64, id: &str) {
        db.append_ledger_event(&LedgerEventDraft::from_audit_event(
            event, serde_json::json!({"occurred_at_ms": ms, "correlation_id": id}),
        )).unwrap();
    }

    #[test]
    fn long_outage_retry_logs_and_restarts_preserve_earliest_gap() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let gap = db.begin_event_stream_gap(1_000).unwrap();
        assert!(db.close_event_stream_gap(&gap.1, 2_000, &completed()).unwrap());
        let gap = db.begin_event_stream_gap(3_000).unwrap();
        assert_eq!(gap.0, 2_000);
        audit(&db, "EVENT_STREAM_CONNECT_FAILED", 99_000_000, &gap.1);
        audit(&db, "DB_WRITE_FAILED", 99_000_001, &gap.1);
        drop(db);
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(db.begin_event_stream_gap(100_000_000).unwrap(), gap);
        assert!(!db.checkpoint_event_stream_health(100_000_001).unwrap());
        assert_eq!(db.begin_event_stream_gap(200_000_000).unwrap(), gap);
    }

    #[test]
    fn idle_health_not_last_audit_activity_bounds_a_brief_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let gap = db.begin_event_stream_gap(1_000).unwrap();
        db.close_event_stream_gap(&gap.1, 2_000, &completed()).unwrap();
        let rows_before: i64 = db.conn.lock().unwrap()
            .query_row("SELECT count(*) FROM ledger_events", [], |row| row.get(0)).unwrap();
        for now in [10_000_000, 10_030_000, 10_060_000] {
            assert!(db.checkpoint_event_stream_health(now).unwrap());
        }
        let rows_after: i64 = db.conn.lock().unwrap()
            .query_row("SELECT count(*) FROM ledger_events", [], |row| row.get(0)).unwrap();
        assert_eq!(rows_before, rows_after, "heartbeats must not flood the audit log");
        drop(db);
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(db.begin_event_stream_gap(10_065_000).unwrap().0, 10_060_000);
    }

    #[test]
    fn legacy_normal_activity_is_unknown_but_explicit_open_gaps_are_recovered() {
        let db = Database::open_in_memory().unwrap();
        audit(&db, "PAYMENT_SETTLED", 1, "payment");
        assert_eq!(db.begin_event_stream_gap(90_000_000).unwrap().0, 90_000_000);
        let unknown: String = db.conn.lock().unwrap().query_row(
            "SELECT status FROM ledger_events WHERE event_type = 'EVENT_STREAM_COVERAGE_UNKNOWN'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(unknown, "unknown");

        let db = Database::open_in_memory().unwrap();
        audit(&db, "EVENT_STREAM_GAP_STARTED", 5, "closed");
        audit(&db, "EVENT_STREAM_GAP_CLOSED", 10, "closed");
        audit(&db, "EVENT_STREAM_GAP_STARTED", 20, "oldest-open");
        audit(&db, "EVENT_STREAM_GAP_STARTED", 30, "later-open");
        audit(&db, "EVENT_STREAM_CONNECT_FAILED", 80_000_000, "oldest-open");
        assert_eq!(db.begin_event_stream_gap(90_000_000).unwrap(), (20, "oldest-open".into()));
    }

    #[test]
    fn failed_gap_close_audit_rolls_back_health_and_preserves_gap_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let gap = db.begin_event_stream_gap(1_000).unwrap();
        db.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER fail_gap_close BEFORE INSERT ON ledger_events
             WHEN NEW.event_type = 'EVENT_STREAM_GAP_CLOSED'
             BEGIN SELECT RAISE(FAIL, 'injected gap audit failure'); END;",
        ).unwrap();
        assert!(db.close_event_stream_gap(&gap.1, 2_000, &completed()).is_err());
        drop(db);
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(db.begin_event_stream_gap(3_000).unwrap(), gap);
        assert!(!db.checkpoint_event_stream_health(4_000).unwrap());
    }

    #[test]
    fn partial_reconciliation_cannot_close_gap_or_establish_health() {
        let db = Database::open_in_memory().unwrap();
        let gap = db.begin_event_stream_gap(1_000).unwrap();
        let partial = serde_json::json!({"status": "partial", "history_complete": false});
        assert!(db.close_event_stream_gap(&gap.1, 2_000, &partial).is_err());
        assert!(!db.checkpoint_event_stream_health(3_000).unwrap());
        assert_eq!(db.begin_event_stream_gap(4_000).unwrap(), gap);
    }

    #[test]
    fn failed_start_or_heartbeat_never_discards_previous_health() {
        let db = Database::open_in_memory().unwrap();
        let gap = db.begin_event_stream_gap(1_000).unwrap();
        db.close_event_stream_gap(&gap.1, 2_000, &completed()).unwrap();
        db.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER fail_checkpoint BEFORE UPDATE ON event_stream_checkpoint
             BEGIN SELECT RAISE(FAIL, 'injected checkpoint failure'); END;",
        ).unwrap();
        assert!(db.checkpoint_event_stream_health(3_000).is_err());
        assert!(db.begin_event_stream_gap(4_000).is_err());
        db.conn.lock().unwrap().execute_batch("DROP TRIGGER fail_checkpoint").unwrap();
        assert_eq!(db.begin_event_stream_gap(5_000).unwrap().0, 2_000);
    }

    #[test]
    fn startup_audit_failure_rolls_back_lazy_initialization() {
        let db = Database::open_in_memory().unwrap();
        db.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER fail_gap_start BEFORE INSERT ON ledger_events
             WHEN NEW.event_type = 'EVENT_STREAM_GAP_STARTED'
             BEGIN SELECT RAISE(FAIL, 'injected startup audit failure'); END;",
        ).unwrap();
        assert!(db.begin_event_stream_gap(1_000).is_err());
        let conn = db.conn.lock().unwrap();
        let rows: i64 = conn.query_row(
            "SELECT count(*) FROM ledger_events WHERE event_type = 'EVENT_STREAM_COVERAGE_UNKNOWN'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(rows, 0, "startup coverage audit and checkpoint must commit together");
        let tables: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'event_stream_checkpoint'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(tables, 0);
        conn.execute_batch("DROP TRIGGER fail_gap_start").unwrap();
        drop(conn);
        assert_eq!(db.begin_event_stream_gap(2_000).unwrap().0, 2_000);
    }

    #[test]
    fn repeated_reconnects_reject_stale_closure_and_preserve_loss_details() {
        let db = Database::open_in_memory().unwrap();
        let old = db.begin_event_stream_gap(1_000).unwrap();
        db.close_event_stream_gap(&old.1, 2_000, &completed()).unwrap();
        let next = db.begin_event_stream_gap(2_000).unwrap();
        assert_ne!(old.1, next.1);
        assert!(!db.close_event_stream_gap(&old.1, 3_000, &completed()).unwrap());
        assert_eq!(db.begin_event_stream_gap(4_000).unwrap(), next);
        let loss = serde_json::json!({"status": "completed_with_loss", "history_complete": false});
        assert!(db.close_event_stream_gap(&next.1, 5_000, &loss).unwrap());
        let detail: String = db.conn.lock().unwrap().query_row(
            "SELECT detail_json FROM ledger_events WHERE event_type = 'EVENT_STREAM_GAP_CLOSED'
             ORDER BY id DESC LIMIT 1", [], |row| row.get(0),
        ).unwrap();
        let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["history_complete"], false);
        assert_eq!(detail["status"], "completed_with_loss");
    }
}
