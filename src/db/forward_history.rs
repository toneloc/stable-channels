//! Forward audit identities and conservative, occurrence-scoped correlation.
//!
//! History IDs are authoritative. Live events have neither that ID nor the forwarding
//! timestamp, so even a unique nearby attribute match is only a possible correlation.
//! Keep both immutable ledger observations, including when backfill precedes buffered
//! live delivery. Consumers must not sum observations as if they were distinct payments.

use super::{forward_fingerprint, Database};
use crate::ledger::{self, LedgerEventDraft, LedgerRef};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use serde_json::{json, Value};

// Bounds heuristic matching, not deduplication or history retention. Delayed/buffered
// live events outside this window remain explicitly uncorrelated observations.
const CORRELATION_WINDOW_MS: i64 = 120_000;

fn fingerprint(detail: &Value) -> Option<String> {
    Some(forward_fingerprint(
        detail.get("prev_channel_id")?.as_str()?,
        detail.get("next_channel_id")?.as_str()?,
        detail.get("outbound_amount_msat").or_else(|| detail.get("forwarded_msat")).and_then(Value::as_u64),
        detail.get("total_fee_msat").or_else(|| detail.get("fee_msat")).and_then(Value::as_u64),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(history: bool, time: i64) -> LedgerEventDraft {
        LedgerEventDraft::from_audit_event(
            if history { "PAYMENT_FORWARDED_BACKFILL" } else { "PAYMENT_FORWARDED" },
            json!({"prev_channel_id": "aa", "next_channel_id": "bb", "outbound_amount_msat": 1_000, "total_fee_msat": 7, "occurred_at_ms": time}),
        )
    }

    fn details(db: &Database) -> Vec<Value> {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT detail_json FROM ledger_events ORDER BY id").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0)).unwrap()
            .map(|raw| serde_json::from_str(&raw.unwrap()).unwrap()).collect()
    }

    #[test]
    fn repeated_live_forwards_are_independent_occurrences_even_with_attribute_dedup_keys() {
        let db = Database::open_in_memory().unwrap();
        let mut live = draft(false, 1_000_000);
        live.dedup_key = Some("old-attribute-key".into());
        assert!(db.append_observed_forward(&live).unwrap());
        assert!(db.append_observed_forward(&live).unwrap());
        assert_eq!(details(&db).len(), 2);
    }

    #[test]
    fn live_and_history_correlate_once_in_either_order_and_preserve_provenance() {
        for history_first in [false, true] {
            let db = Database::open_in_memory().unwrap();
            if history_first {
                db.append_forwarded_event_if_unseen("h1", &draft(true, 1_000_000)).unwrap();
                db.append_observed_forward(&draft(false, 1_001_000)).unwrap();
            } else {
                db.append_observed_forward(&draft(false, 1_001_000)).unwrap();
                db.append_forwarded_event_if_unseen("h1", &draft(true, 1_000_000)).unwrap();
            }
            let rows = details(&db);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[1]["forward_correlation"]["status"], "possible_match");
            assert_eq!(rows[1]["forward_correlation"]["candidate_event_id"], 1);
            assert_eq!(rows[1]["forward_correlation"]["exact_identity"], false);
            assert_ne!(rows[0]["forward_provenance"], rows[1]["forward_provenance"]);

            assert!(!db.append_forwarded_event_if_unseen("h1", &draft(true, 1_000_000)).unwrap());
            db.append_forwarded_event_if_unseen("h2", &draft(true, 1_000_000)).unwrap();
            assert_eq!(details(&db)[2]["forward_correlation"]["status"], "unmatched", "the first live occurrence is already paired");
            db.append_observed_forward(&draft(false, 1_001_000)).unwrap();
            let rows = details(&db);
            assert_eq!(rows.len(), 4, "two distinct history IDs and two live observations survive");
            assert_eq!(rows[3]["forward_correlation"]["candidate_event_id"], 3);
        }
    }

    #[test]
    fn ambiguous_delayed_and_undated_history_never_claim_exact_live_identity() {
        let db = Database::open_in_memory().unwrap();
        db.append_observed_forward(&draft(false, 1_000_000)).unwrap();
        db.append_observed_forward(&draft(false, 1_000_000)).unwrap();
        db.append_forwarded_event_if_unseen("ambiguous", &draft(true, 1_000_000)).unwrap();
        assert_eq!(details(&db)[2]["forward_correlation"]["status"], "ambiguous");
        db.append_forwarded_event_if_unseen("old", &draft(true, 1_000)).unwrap();
        assert_eq!(details(&db)[3]["forward_correlation"]["status"], "unmatched");
        let mut undated = draft(true, 1_000_000);
        undated.detail.as_object_mut().unwrap().remove("occurred_at_ms");
        db.append_forwarded_event_if_unseen("undated", &undated).unwrap();
        assert_eq!(details(&db)[4]["forward_correlation"]["status"], "unavailable");
        assert_eq!(details(&db).len(), 5);
    }

    #[test]
    fn legacy_markers_do_not_suppress_new_occurrences_and_json_ids_migrate() {
        let db = Database::open_in_memory().unwrap();
        let mut old_history = draft(true, 1_000_000);
        old_history.detail["forwarded_payment_id"] = json!("legacy-id");
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO forwarded_seen VALUES ('aa|bb|1000|7')", []).unwrap();
            ledger::append_on_connection(&conn, &old_history).unwrap();
        }
        assert!(!db.append_forwarded_event_if_unseen("legacy-id", &old_history).unwrap());
        assert!(db.append_forwarded_event_if_unseen("new-id", &old_history).unwrap());
        db.append_observed_forward(&draft(false, 1_001_000)).unwrap();
        db.append_observed_forward(&draft(false, 1_001_000)).unwrap();
        assert_eq!(details(&db).len(), 4);
        assert_eq!(details(&db)[1]["forwarded_payment_id"], "new-id");
    }

    #[test]
    fn legacy_idless_history_is_only_a_bounded_possible_match() {
        let db = Database::open_in_memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO forwarded_seen VALUES ('aa|bb|1000|7')", []).unwrap();
            ledger::append_on_connection(&conn, &draft(true, 1_000_000)).unwrap();
        }
        db.append_forwarded_event_if_unseen("first", &draft(true, 1_000_000)).unwrap();
        db.append_forwarded_event_if_unseen("second", &draft(true, 1_000_000)).unwrap();
        let rows = details(&db);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1]["forward_correlation"]["status"], "possible_match");
        assert_eq!(rows[2]["forward_correlation"]["status"], "unmatched");
    }

    #[test]
    fn orphan_legacy_marker_cannot_consume_any_live_or_history_occurrence() {
        let db = Database::open_in_memory().unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO forwarded_seen VALUES ('aa|bb|1000|7')", []).unwrap();
        for _ in 0..2 {
            assert!(db.append_observed_forward(&draft(false, 1_000_000)).unwrap());
        }
        for id in ["first", "second"] {
            assert!(db.append_forwarded_event_if_unseen(id, &draft(true, 1_000_000)).unwrap());
            assert!(!db.append_forwarded_event_if_unseen(id, &draft(true, 1_000_000)).unwrap());
        }
        assert_eq!(details(&db).len(), 4);
    }

    #[test]
    fn occurrence_insert_failure_rolls_back_event_marker_and_correlation() {
        let db = Database::open_in_memory().unwrap();
        db.append_observed_forward(&draft(false, 1_000_000)).unwrap();
        db.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER fail_forward_occurrence BEFORE INSERT ON forward_audit_occurrences
             BEGIN SELECT RAISE(ABORT, 'injected occurrence failure'); END;"
        ).unwrap();
        assert!(db.append_forwarded_event_if_unseen("retry", &draft(true, 1_000_000)).is_err());
        assert_eq!(details(&db).len(), 1, "the ledger insert preceding the marker also rolls back");
        db.conn.lock().unwrap().execute_batch("DROP TRIGGER fail_forward_occurrence").unwrap();
        assert!(db.append_forwarded_event_if_unseen("retry", &draft(true, 1_000_000)).unwrap());
        assert_eq!(details(&db)[1]["forward_correlation"]["candidate_event_id"], 1);
        assert!(!db.append_forwarded_event_if_unseen("retry", &draft(true, 1_000_000)).unwrap());
        assert_eq!(details(&db).len(), 2);
    }

    #[test]
    fn live_ledger_failure_is_retryable_without_consuming_the_history_match() {
        let db = Database::open_in_memory().unwrap();
        db.append_forwarded_event_if_unseen("h", &draft(true, 1_000_000)).unwrap();
        db.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER fail_live_forward BEFORE INSERT ON ledger_event_refs
             BEGIN SELECT RAISE(ABORT, 'injected ledger failure'); END;"
        ).unwrap();
        assert!(db.append_observed_forward(&draft(false, 1_000_000)).is_err());
        assert_eq!(details(&db).len(), 1);
        db.conn.lock().unwrap().execute_batch("DROP TRIGGER fail_live_forward").unwrap();
        assert!(db.append_observed_forward(&draft(false, 1_000_000)).unwrap());
        assert_eq!(details(&db)[1]["forward_correlation"]["candidate_event_id"], 1);
    }
}

fn init_schema(conn: &Connection) -> SqliteResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS forward_audit_occurrences (
            event_id INTEGER PRIMARY KEY REFERENCES ledger_events(id),
            history_id TEXT UNIQUE,
            fingerprint TEXT,
            origin TEXT NOT NULL,
            correlation_time_ms INTEGER,
            correlated_event_id INTEGER UNIQUE REFERENCES ledger_events(id)
         );
         CREATE INDEX IF NOT EXISTS idx_forward_audit_correlation
            ON forward_audit_occurrences(fingerprint, correlation_time_ms)
            WHERE correlated_event_id IS NULL;",
    )?;
    const MIGRATION: &str = "forward_audit_occurrences_v1";
    let migrated: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM ledger_metadata WHERE key = ?1)",
        [MIGRATION],
        |row| row.get(0),
    )?;
    if migrated {
        return Ok(());
    }

    // Recover IDs already stored in legacy event JSON, not lossy forwarded_seen
    // markers. A marker alone proves neither an occurrence count nor a history ID.
    let mut stmt = conn.prepare(
        "SELECT id, event_type, occurred_at_ms, detail_json FROM ledger_events
         WHERE event_type IN ('PAYMENT_FORWARDED', 'PAYMENT_FORWARDED_BACKFILL') ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?))
    })?;
    for row in rows {
        let (event_id, event_type, occurred_at_ms, raw) = row?;
        let detail: Value = serde_json::from_str(&raw).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
        })?;
        let is_history = event_type == "PAYMENT_FORWARDED_BACKFILL";
        let history_id = if is_history {
            detail.get("forwarded_payment_id").and_then(Value::as_str).filter(|id| !id.is_empty())
        } else {
            None
        };
        let time = if is_history {
            detail.get("occurred_at_ms").and_then(Value::as_i64)
        } else {
            Some(occurred_at_ms)
        };
        // Duplicate legacy JSON IDs keep their original rows; only the first is
        // the durable replay marker. No existing ledger rows are rewritten.
        conn.execute(
            "INSERT OR IGNORE INTO forward_audit_occurrences
                (event_id, history_id, fingerprint, origin, correlation_time_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event_id, history_id, fingerprint(&detail), if is_history { "legacy_history" } else { "legacy_live" }, time],
        )?;
    }
    conn.execute("INSERT INTO ledger_metadata (key, value) VALUES (?1, '1')", [MIGRATION])?;
    Ok(())
}

impl Database {
    /// Append one live stream occurrence. Equal attributes never suppress another
    /// occurrence. Without an upstream event ID, replay cannot safely be distinguished
    /// from another genuine payment; possible history matches preserve both sources.
    pub fn append_observed_forward(&self, draft: &LedgerEventDraft) -> SqliteResult<bool> {
        self.append_forward_occurrence(None, draft)
    }

    /// Record a durable LDK ForwardedPayment.id and its history row atomically.
    /// The argument is an upstream history ID, NEVER a route/amount fingerprint.
    /// Empty IDs are rejected: the caller must report incomplete history coverage.
    pub fn append_forwarded_event_if_unseen(
        &self,
        history_id: &str,
        draft: &LedgerEventDraft,
    ) -> SqliteResult<bool> {
        if history_id.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName("empty forwarded-payment history ID".into()));
        }
        self.append_forward_occurrence(Some(history_id), draft)
    }

    fn append_forward_occurrence(&self, history_id: Option<&str>, draft: &LedgerEventDraft) -> SqliteResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        init_schema(&tx)?;
        if let Some(id) = history_id {
            let existing: Option<i64> = tx.query_row(
                "SELECT event_id FROM forward_audit_occurrences WHERE history_id = ?1",
                [id], |row| row.get(0),
            ).optional()?;
            if existing.is_some() {
                tx.commit()?;
                return Ok(false);
            }
        }

        let fingerprint = fingerprint(&draft.detail);
        let time = if history_id.is_some() {
            // A history row without an upstream timestamp is not eligible for a
            // time-bounded match just because it was reconstructed now.
            draft.detail.get("occurred_at_ms").and_then(Value::as_i64)
        } else {
            Some(draft.occurred_at_ms)
        };
        let eligible = if history_id.is_some() {
            "(origin IN ('live', 'legacy_live') OR (origin = 'legacy_history' AND history_id IS NULL))"
        } else {
            "origin IN ('history', 'legacy_history')"
        };
        let candidates: Vec<i64> = if let (Some(key), Some(time)) = (fingerprint.as_ref(), time) {
            let mut stmt = tx.prepare(&format!(
                "SELECT event_id FROM forward_audit_occurrences
                 WHERE fingerprint = ?1 AND correlation_time_ms BETWEEN ?2 AND ?3
                    AND correlated_event_id IS NULL AND {eligible}
                 ORDER BY event_id LIMIT 2"
            ))?;
            let rows = stmt.query_map(params![key, time.saturating_sub(CORRELATION_WINDOW_MS), time.saturating_add(CORRELATION_WINDOW_MS)], |row| row.get(0))?;
            rows.collect::<SqliteResult<_>>()?
        } else {
            Vec::new()
        };
        let candidate = (candidates.len() == 1).then(|| candidates[0]);
        let mut draft = draft.clone();
        // Ignore caller-provided attribute dedup keys for live occurrences.
        draft.dedup_key = history_id.map(|id| format!("ldk:forwarded-payment:{id}"));
        let detail = draft.detail.as_object_mut().ok_or(rusqlite::Error::InvalidQuery)?;
        detail.remove("dedup_key");
        detail.insert("forward_provenance".into(), json!(if history_id.is_some() { "ldk_history" } else { "live_stream" }));
        detail.insert("forward_identity".into(), json!(if history_id.is_some() { "history_id" } else { "unidentified_live_occurrence" }));
        detail.insert("forward_correlation".into(), json!({
            "status": if time.is_none() || fingerprint.is_none() { "unavailable" } else if candidates.len() > 1 { "ambiguous" } else if candidate.is_some() { "possible_match" } else { "unmatched" },
            "candidate_event_id": candidate,
            "candidate_count_at_least": candidates.len(),
            "window_ms": CORRELATION_WINDOW_MS,
            "exact_identity": false,
            "both_observations_preserved": true,
        }));
        if let Some(id) = history_id {
            detail.insert("forwarded_payment_id".into(), json!(id));
            draft.refs.push(LedgerRef::new("forwarded_payment_id", id));
        }

        let outcome = ledger::append_on_connection(&tx, &draft)?;
        tx.execute(
            "INSERT INTO forward_audit_occurrences
                (event_id, history_id, fingerprint, origin, correlation_time_ms, correlated_event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![outcome.event_id, history_id, fingerprint, if history_id.is_some() { "history" } else { "live" }, time, candidate],
        )?;
        if let Some(candidate) = candidate {
            tx.execute("UPDATE forward_audit_occurrences SET correlated_event_id = ?1 WHERE event_id = ?2", params![outcome.event_id, candidate])?;
        }
        tx.commit()?;
        if outcome.inserted {
            crate::audit::mirror_committed_ledger_event(&draft, outcome.event_id);
        }
        Ok(outcome.inserted)
    }
}
