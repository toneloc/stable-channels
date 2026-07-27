//! Authoritative, append-only channel event ledger.
//!
//! SQLite is the source of truth. `audit_log.txt` is maintained by the audit
//! module as a best-effort JSONL mirror for operators and older tooling.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerCompleteness {
    Observed,
    Reconstructed,
    Legacy,
    Gap,
}

impl LedgerCompleteness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Reconstructed => "reconstructed",
            Self::Legacy => "legacy",
            Self::Gap => "gap",
        }
    }

    fn from_db(value: &str) -> Self {
        match value {
            "reconstructed" => Self::Reconstructed,
            "legacy" => Self::Legacy,
            "gap" => Self::Gap,
            _ => Self::Observed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRef {
    pub role: String,
    pub value: String,
}

impl LedgerRef {
    pub fn new(role: impl Into<String>, value: impl Into<String>) -> Self {
        Self { role: role.into(), value: value.into() }
    }
}

/// Accounting truth captured around a money- or state-moving event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountingSnapshot {
    pub expected_usd: Option<f64>,
    pub backing_sats: Option<u64>,
    pub native_sats: Option<u64>,
    pub live_receiver_sats: Option<u64>,
    pub btc_price: Option<f64>,
    pub amount_sats: Option<u64>,
    pub amount_msat: Option<u64>,
    pub amount_usd: Option<f64>,
    pub fee_sats: Option<u64>,
    pub fee_msat: Option<u64>,
}

impl AccountingSnapshot {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Typed input accepted by the ledger recorder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEventDraft {
    pub event_type: String,
    pub category: String,
    pub severity: String,
    pub status: String,
    pub source: String,
    pub completeness: LedgerCompleteness,
    pub occurred_at_ms: i64,
    pub dedup_key: Option<String>,
    pub before: Option<AccountingSnapshot>,
    pub after: Option<AccountingSnapshot>,
    pub detail: Value,
    pub refs: Vec<LedgerRef>,
}

impl LedgerEventDraft {
    pub fn from_audit_event(event_type: &str, detail: Value) -> Self {
        let upper = event_type.to_ascii_uppercase();
        let completeness = if upper.contains("GAP") {
            LedgerCompleteness::Gap
        } else if upper.contains("BACKFILL") || upper.contains("RECONSTRUCTED") {
            LedgerCompleteness::Reconstructed
        } else {
            LedgerCompleteness::Observed
        };
        let refs = extract_refs(&detail);
        let before = extract_snapshot(&detail, true);
        let after = extract_snapshot(&detail, false);
        let occurred_at_ms = detail
            .get("occurred_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| Utc::now().timestamp_millis());
        let mut draft = Self {
            event_type: event_type.to_owned(),
            category: category_for(&upper).to_owned(),
            severity: severity_for(&upper).to_owned(),
            status: detail
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_else(|| status_for(&upper))
                .to_owned(),
            source: detail
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("lsp")
                .to_owned(),
            completeness,
            occurred_at_ms,
            dedup_key: detail.get("dedup_key").and_then(Value::as_str).map(str::to_owned),
            before,
            after,
            detail,
            refs,
        };
        if draft.dedup_key.is_none() {
            draft.dedup_key = natural_dedup_key(&draft, &upper);
        }
        draft
    }

    pub fn with_ref(mut self, role: impl Into<String>, value: impl Into<String>) -> Self {
        self.refs.push(LedgerRef::new(role, value));
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEvent {
    pub id: i64,
    pub event_type: String,
    pub category: String,
    pub severity: String,
    pub status: String,
    pub source: String,
    pub completeness: LedgerCompleteness,
    pub occurred_at_ms: i64,
    pub recorded_at_ms: i64,
    pub dedup_key: Option<String>,
    pub before: Option<AccountingSnapshot>,
    pub after: Option<AccountingSnapshot>,
    pub detail: Value,
    pub refs: Vec<LedgerRef>,
}

#[derive(Debug, Clone, Default)]
pub struct LedgerQuery {
    pub identifier: Option<String>,
    pub category: Option<String>,
    pub status: Option<String>,
    pub completeness: Option<String>,
    /// Return rows with an id lower than this cursor.
    pub before_id: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct LedgerPage {
    /// Chronological within the page. Pages themselves are selected newest-first.
    pub events: Vec<LedgerEvent>,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendOutcome {
    pub event_id: i64,
    pub inserted: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyImportReport {
    pub imported: usize,
    pub skipped: usize,
    pub already_imported: bool,
}

pub(crate) fn init_schema(conn: &Connection) -> SqliteResult<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS ledger_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type TEXT NOT NULL,
            category TEXT NOT NULL,
            severity TEXT NOT NULL,
            status TEXT NOT NULL,
            source TEXT NOT NULL,
            completeness TEXT NOT NULL CHECK (completeness IN ('observed','reconstructed','legacy','gap')),
            occurred_at_ms INTEGER NOT NULL,
            recorded_at_ms INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000),
            dedup_key TEXT UNIQUE,
            before_json TEXT,
            after_json TEXT,
            detail_json TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS ledger_event_refs (
            event_id INTEGER NOT NULL REFERENCES ledger_events(id) ON DELETE CASCADE,
            role TEXT NOT NULL,
            value TEXT NOT NULL,
            PRIMARY KEY (event_id, role, value)
         );
         CREATE TABLE IF NOT EXISTS ledger_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at_ms INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000)
         );
         CREATE INDEX IF NOT EXISTS idx_ledger_events_page ON ledger_events(id DESC);
         CREATE INDEX IF NOT EXISTS idx_ledger_events_filters
            ON ledger_events(category, status, completeness, id DESC);
         CREATE INDEX IF NOT EXISTS idx_ledger_refs_exact
            ON ledger_event_refs(value, role, event_id DESC);",
    )?;
    backfill_recognized_refs(conn)
}

/// Re-run reference extraction once when the recognized reference contract
/// expands. This updates only the index rows; the immutable event detail and
/// original timestamps remain unchanged.
fn backfill_recognized_refs(conn: &Connection) -> SqliteResult<()> {
    const METADATA_KEY: &str = "ledger_ref_backfill_v2_plural_ids";

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        let completed: Option<String> = conn
            .query_row(
                "SELECT value FROM ledger_metadata WHERE key = ?1",
                params![METADATA_KEY],
                |row| row.get(0),
            )
            .optional()?;
        if completed.is_some() {
            return Ok(());
        }

        let events = {
            let mut stmt = conn.prepare("SELECT id, detail_json FROM ledger_events ORDER BY id")?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
                .collect::<SqliteResult<Vec<_>>>()?;
            rows
        };
        let mut added = 0usize;
        for (event_id, detail_json) in events {
            let detail: Value = serde_json::from_str(&detail_json).map_err(json_err)?;
            for reference in extract_refs(&detail) {
                added += conn.execute(
                    "INSERT OR IGNORE INTO ledger_event_refs (event_id, role, value)
                     VALUES (?1, ?2, ?3)",
                    params![event_id, reference.role, reference.value],
                )?;
            }
        }
        let metadata = serde_json::json!({
            "added": added,
            "completed_at": Utc::now().to_rfc3339(),
        });
        conn.execute(
            "INSERT INTO ledger_metadata (key, value) VALUES (?1, ?2)",
            params![METADATA_KEY, metadata.to_string()],
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT"),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        },
    }
}

pub(crate) fn append_on_connection(
    conn: &Connection,
    draft: &LedgerEventDraft,
) -> SqliteResult<AppendOutcome> {
    let before_json = draft.before.as_ref().map(serde_json::to_string).transpose().map_err(json_err)?;
    let after_json = draft.after.as_ref().map(serde_json::to_string).transpose().map_err(json_err)?;
    let detail_json = serde_json::to_string(&draft.detail).map_err(json_err)?;
    let inserted = conn.execute(
        "INSERT INTO ledger_events
            (event_type, category, severity, status, source, completeness,
             occurred_at_ms, dedup_key, before_json, after_json, detail_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(dedup_key) DO NOTHING",
        params![
            draft.event_type,
            draft.category,
            draft.severity,
            draft.status,
            draft.source,
            draft.completeness.as_str(),
            draft.occurred_at_ms,
            draft.dedup_key,
            before_json,
            after_json,
            detail_json,
        ],
    )? != 0;
    let event_id = if inserted {
        conn.last_insert_rowid()
    } else {
        conn.query_row(
            "SELECT id FROM ledger_events WHERE dedup_key = ?1",
            params![draft.dedup_key],
            |row| row.get(0),
        )?
    };
    if inserted {
        let mut unique = BTreeSet::new();
        for reference in &draft.refs {
            let role = reference.role.trim();
            let value = reference.value.trim();
            if role.is_empty() || value.is_empty() || !unique.insert((role, value)) {
                continue;
            }
            conn.execute(
                "INSERT OR IGNORE INTO ledger_event_refs (event_id, role, value) VALUES (?1, ?2, ?3)",
                params![event_id, role, value],
            )?;
        }
    }
    Ok(AppendOutcome { event_id, inserted })
}

pub(crate) fn list_on_connection(conn: &Connection, query: &LedgerQuery) -> SqliteResult<LedgerPage> {
    let identifier = query.identifier.as_deref().unwrap_or("").trim();
    let category = query.category.as_deref().unwrap_or("").trim();
    let status = query.status.as_deref().unwrap_or("").trim();
    let completeness = query.completeness.as_deref().unwrap_or("").trim();
    let before_id = query.before_id.unwrap_or(0);
    let limit = if query.limit == 0 { 50 } else { query.limit.min(200) };
    let mut stmt = conn.prepare(
        "SELECT id, event_type, category, severity, status, source, completeness,
                occurred_at_ms, recorded_at_ms, dedup_key, before_json, after_json, detail_json
         FROM ledger_events e
         WHERE (?1 = '' OR EXISTS (
                  SELECT 1 FROM ledger_event_refs r WHERE r.event_id = e.id AND r.value = ?1
               ))
           AND (?2 = '' OR e.category = ?2)
           AND (?3 = '' OR e.status = ?3)
           AND (?4 = '' OR e.completeness = ?4)
           AND (?5 = 0 OR e.id < ?5)
         ORDER BY e.id DESC
         LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![identifier, category, status, completeness, before_id, (limit + 1) as i64],
        |row| {
            let before_json: Option<String> = row.get(10)?;
            let after_json: Option<String> = row.get(11)?;
            let detail_json: String = row.get(12)?;
            let completeness: String = row.get(6)?;
            Ok(LedgerEvent {
                id: row.get(0)?,
                event_type: row.get(1)?,
                category: row.get(2)?,
                severity: row.get(3)?,
                status: row.get(4)?,
                source: row.get(5)?,
                completeness: LedgerCompleteness::from_db(&completeness),
                occurred_at_ms: row.get(7)?,
                recorded_at_ms: row.get(8)?,
                dedup_key: row.get(9)?,
                before: decode_optional_json(before_json)?,
                after: decode_optional_json(after_json)?,
                detail: serde_json::from_str(&detail_json).map_err(json_err)?,
                refs: Vec::new(),
            })
        },
    )?;
    let mut events = rows.collect::<SqliteResult<Vec<_>>>()?;
    drop(stmt);
    let has_more = events.len() > limit;
    if has_more {
        events.truncate(limit);
    }
    let next_cursor = has_more.then(|| events.last().map(|e| e.id)).flatten();
    let mut refs_stmt = conn.prepare(
        "SELECT role, value FROM ledger_event_refs WHERE event_id = ?1 ORDER BY role, value",
    )?;
    for event in &mut events {
        event.refs = refs_stmt
            .query_map(params![event.id], |row| {
                Ok(LedgerRef { role: row.get(0)?, value: row.get(1)? })
            })?
            .collect::<SqliteResult<Vec<_>>>()?;
    }
    events.reverse();
    Ok(LedgerPage { events, next_cursor })
}

pub(crate) fn import_legacy_jsonl(conn: &Connection, path: &Path) -> SqliteResult<LegacyImportReport> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        let done: Option<String> = conn
            .query_row(
                "SELECT value FROM ledger_metadata WHERE key = 'legacy_audit_import_v1'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if done.is_some() {
            return Ok(LegacyImportReport { already_imported: true, ..Default::default() });
        }

        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
        };
        let mut report = LegacyImportReport::default();
        for (line_no, line) in content.lines().enumerate() {
            let parsed: Value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(_) => {
                    report.skipped += 1;
                    continue;
                },
            };
            let Some(event_type) = parsed.get("event").and_then(Value::as_str) else {
                report.skipped += 1;
                continue;
            };
            let detail = parsed.get("data").cloned().unwrap_or(Value::Null);
            let mut draft = LedgerEventDraft::from_audit_event(event_type, detail);
            draft.completeness = LedgerCompleteness::Legacy;
            draft.source = "legacy_jsonl".to_owned();
            if let Some(ts) = parsed.get("ts").and_then(Value::as_str) {
                if let Ok(ts) = DateTime::parse_from_rfc3339(ts) {
                    draft.occurred_at_ms = ts.timestamp_millis();
                }
            }
            draft.dedup_key = Some(format!(
                "legacy:{}:{}:{}",
                line_no + 1,
                draft.occurred_at_ms,
                event_type
            ));
            if append_on_connection(conn, &draft)?.inserted {
                report.imported += 1;
            }
        }
        let metadata = serde_json::json!({
            "path": path.display().to_string(),
            "imported": report.imported,
            "skipped": report.skipped,
            "completed_at": Utc::now().to_rfc3339(),
        });
        conn.execute(
            "INSERT INTO ledger_metadata (key, value) VALUES ('legacy_audit_import_v1', ?1)",
            params![metadata.to_string()],
        )?;
        Ok(report)
    })();
    match result {
        Ok(report) => {
            conn.execute_batch("COMMIT")?;
            Ok(report)
        },
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        },
    }
}

fn json_err(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn decode_optional_json<T: for<'de> Deserialize<'de>>(raw: Option<String>) -> SqliteResult<Option<T>> {
    raw.map(|value| serde_json::from_str(&value).map_err(json_err)).transpose()
}

fn category_for(event: &str) -> &'static str {
    if event.contains("SPLICE") || event.contains("CHANNEL") {
        "channel"
    } else if event.contains("FORWARD") {
        "forwarding"
    } else if event.contains("PAYMENT") {
        "payment"
    } else if event.contains("TRADE") {
        "trade"
    } else if event.contains("STABILITY") || event.contains("STABLE") || event.contains("SYNC") {
        "stability"
    } else if event.contains("PEER") {
        "peer"
    } else if event.contains("SWEEP") || event.contains("CLOSURE") {
        "sweep"
    } else if event.contains("RECONCIL") || event.contains("BACKFILL") || event.contains("EVENT_STREAM") || event.contains("GAP") {
        "reconciliation"
    } else if event.contains("EDIT") || event.contains("CONFIG") || event.contains("OPERATOR") {
        "operator"
    } else {
        "system"
    }
}

fn severity_for(event: &str) -> &'static str {
    if event.contains("FAILED") || event.contains("ERROR") || event.contains("REJECTED") {
        "error"
    } else if event.contains("GAP") || event.contains("CLAMP") || event.contains("DEFERRED") {
        "warning"
    } else {
        "info"
    }
}

fn status_for(event: &str) -> &'static str {
    if event.contains("FAILED") || event.contains("ERROR") || event.contains("REJECTED") {
        "failed"
    } else if event.contains("SKIPPED") || event.contains("COOLDOWN") {
        "skipped"
    } else if event.contains("PENDING") || event.contains("STARTED") || event.contains("CLAIMABLE") {
        "pending"
    } else if event.contains("SUCCESS")
        || event.contains("SETTLED")
        || event.contains("COMPLETED")
        || event.contains("APPLIED")
        || event.contains("RECONCILED")
        || event.contains("READY")
        || event.contains("CLOSED")
    {
        "completed"
    } else {
        "observed"
    }
}

fn natural_dedup_key(draft: &LedgerEventDraft, event: &str) -> Option<String> {
    let stable = event.contains("RECONSTRUCTED")
        || event.contains("BACKFILL")
        || event.contains("SETTLED")
        || event.contains("SUCCESS")
        || event.contains("FAILED")
        || event.contains("APPLIED")
        || event.contains("READY")
        || event.contains("CLOSED");
    let throttled = event.contains("COOLDOWN") || event.contains("SKIPPED_UNCHANGED");
    if !stable && !throttled {
        return None;
    }
    let mut refs = draft
        .refs
        .iter()
        .map(|r| format!("{}={}", r.role, r.value))
        .collect::<Vec<_>>();
    refs.sort();
    if refs.is_empty() {
        return None;
    }
    let version = draft.detail.get("sync_version").and_then(Value::as_u64).unwrap_or(0);
    let bucket = if throttled { draft.occurred_at_ms / 300_000 } else { 0 };
    Some(format!("event:{}:{}:v{}:b{}", draft.event_type, refs.join("|"), version, bucket))
}

fn extract_refs(detail: &Value) -> Vec<LedgerRef> {
    fn insert_values(role: &str, value: &Value, refs: &mut BTreeSet<(String, String)>) {
        match value {
            Value::String(value) if !value.is_empty() => {
                refs.insert((role.to_owned(), value.to_owned()));
            },
            Value::Number(value) => {
                refs.insert((role.to_owned(), value.to_string()));
            },
            Value::Array(values) => {
                for value in values {
                    insert_values(role, value, refs);
                }
            },
            _ => {},
        }
    }

    fn visit(value: &Value, refs: &mut BTreeSet<(String, String)>) {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    let role = match key.as_str() {
                        "user_channel_id" | "user_channel_ids" | "prev_user_channel_id"
                        | "next_user_channel_id" => Some("user_channel_id"),
                        "channel_id" | "channel_ids" | "prev_channel_id" | "next_channel_id" => {
                            Some("channel_id")
                        },
                        "payment_id" | "payment_ids" => Some("payment_id"),
                        "payment_hash" | "payment_hashes" => Some("payment_hash"),
                        "txid" | "txids" | "transaction_id" | "transaction_ids"
                        | "funding_txo" => Some("transaction_id"),
                        "node_id" | "node_ids" | "counterparty_node_id" | "prev_node_id"
                        | "next_node_id" => Some("node_id"),
                        "correlation_id" | "correlation_ids" => Some("correlation_id"),
                        _ => None,
                    };
                    if let Some(role) = role {
                        insert_values(role, value, refs);
                    }
                    visit(value, refs);
                }
            },
            Value::Array(values) => values.iter().for_each(|value| visit(value, refs)),
            _ => {},
        }
    }
    let mut refs = BTreeSet::new();
    visit(detail, &mut refs);
    refs.into_iter().map(|(role, value)| LedgerRef { role, value }).collect()
}

fn extract_snapshot(detail: &Value, before: bool) -> Option<AccountingSnapshot> {
    let object = detail.as_object()?;
    let number = |keys: &[&str]| {
        keys.iter().find_map(|key| object.get(*key).and_then(Value::as_f64))
    };
    let unsigned = |keys: &[&str]| {
        keys.iter().find_map(|key| object.get(*key).and_then(|v| v.as_u64().or_else(|| v.as_i64().and_then(|v| u64::try_from(v).ok()))))
    };
    let snapshot = if before {
        AccountingSnapshot {
            expected_usd: number(&["before_expected_usd", "old_expected_usd", "pre_expected_usd"]),
            backing_sats: unsigned(&["before_backing_sats", "old_backing_sats", "pre_backing_sats"]),
            native_sats: unsigned(&["before_native_sats", "old_native_sats", "pre_native_sats"]),
            live_receiver_sats: unsigned(&["before_live_receiver_sats", "pre_live_receiver_sats", "receiver_sats_at_start"]),
            btc_price: number(&["before_btc_price", "old_btc_price"]),
            amount_sats: unsigned(&["before_amount_sats"]),
            amount_msat: unsigned(&["before_amount_msat"]),
            amount_usd: number(&["before_amount_usd"]),
            fee_sats: unsigned(&["before_fee_sats"]),
            fee_msat: unsigned(&["before_fee_msat"]),
        }
    } else {
        AccountingSnapshot {
            expected_usd: number(&["after_expected_usd", "new_expected_usd", "expected_usd"]),
            backing_sats: unsigned(&["after_backing_sats", "new_backing_sats", "backing_sats", "stable_sats"]),
            native_sats: unsigned(&["after_native_sats", "new_native_sats", "native_sats"]),
            live_receiver_sats: unsigned(&["after_live_receiver_sats", "live_receiver_sats", "receiver_sats"]),
            btc_price: number(&["btc_price", "price", "after_btc_price"]),
            amount_sats: unsigned(&["amount_sats", "outbound_amount_sats", "splice_out_sats"]),
            amount_msat: unsigned(&["amount_msat", "outbound_amount_msat", "outbound_amount_forwarded_msat"]),
            amount_usd: number(&["amount_usd", "usd_deducted"]),
            fee_sats: unsigned(&["fee_sats"]),
            fee_msat: unsigned(&["fee_msat", "fee_paid_msat", "total_fee_msat"]),
        }
    };
    (!snapshot.is_empty()).then_some(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_all_recognized_references_without_fabrication() {
        let draft = LedgerEventDraft::from_audit_event(
            "PAYMENT_SETTLED",
            serde_json::json!({
                "payment_id": "pay",
                "payment_hash": "hash",
                "user_channel_id": "stable-id",
                "user_channel_ids": ["stable-a", "stable-b"],
                "channel_id": "physical-id",
                "txid": "tx",
                "counterparty_node_id": "node",
                "correlation_id": "corr"
            }),
        );
        assert_eq!(draft.refs.len(), 9);
        for value in ["stable-a", "stable-b"] {
            assert!(draft.refs.contains(&LedgerRef::new("user_channel_id", value)));
        }
        let unassociated = LedgerEventDraft::from_audit_event(
            "PAYMENT_CLAIMABLE",
            serde_json::json!({"payment_id": "mpp"}),
        );
        assert!(!unassociated.refs.iter().any(|r| r.role.contains("channel")));
    }

    #[test]
    fn snapshots_capture_auditable_allocation() {
        let draft = LedgerEventDraft::from_audit_event(
            "SYNC_V1_APPLIED",
            serde_json::json!({
                "old_expected_usd": 9.0,
                "new_expected_usd": 10.0,
                "new_backing_sats": 12_000,
                "native_sats": 3_000,
                "live_receiver_sats": 15_000,
                "btc_price": 80_000.0
            }),
        );
        assert_eq!(draft.before.unwrap().expected_usd, Some(9.0));
        let after = draft.after.unwrap();
        assert_eq!(after.backing_sats.unwrap() + after.native_sats.unwrap(), after.live_receiver_sats.unwrap());
    }

    #[test]
    fn reconnect_markers_preserve_gap_and_result_status() {
        let gap = LedgerEventDraft::from_audit_event(
            "EVENT_STREAM_GAP_STARTED",
            serde_json::json!({"correlation_id": "gap-1"}),
        );
        assert_eq!(gap.completeness, LedgerCompleteness::Gap);
        assert_eq!(gap.status, "pending");
        assert!(gap.refs.iter().any(|reference| {
            reference.role == "correlation_id" && reference.value == "gap-1"
        }));

        let result = LedgerEventDraft::from_audit_event(
            "RECONCILIATION_RESULT",
            serde_json::json!({"correlation_id": "gap-1", "status": "completed"}),
        );
        assert_eq!(result.status, "completed");
    }
}
