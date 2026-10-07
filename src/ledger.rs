//! Authoritative, append-only channel event ledger.
//!
//! SQLite is the source of truth. `audit_log.txt` is maintained by the audit
//! module as a best-effort JSONL mirror for operators and older tooling.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;
use std::io::{BufRead, Cursor};
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

    fn is_complete(&self) -> bool {
        self.expected_usd.is_some()
            && self.backing_sats.is_some()
            && self.native_sats.is_some()
            && self.live_receiver_sats.is_some()
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
        Self {
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
        }
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
    /// Return rows chronologically before this opaque timeline position.
    pub before: Option<LedgerCursor>,
    pub limit: usize,
    /// Also include rows sharing a payment, trade or settlement id with the identifier's rows.
    pub include_linked: bool,
    /// Only channel state changes (see `CHANNEL_STATE_EVENTS` and `DIRECT_STATE_EVENTS`).
    pub state_changes_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerCursor {
    pub occurred_at_ms: i64,
    pub id: i64,
}

#[derive(Debug, Clone)]
pub struct LedgerPage {
    /// Chronological within the page. Pages themselves are selected newest-first.
    pub events: Vec<LedgerEvent>,
    pub next_cursor: Option<LedgerCursor>,
    pub overview: LedgerOverview,
}

/// Aggregate facts for one exact ledger identifier. Coverage and time bounds
/// deliberately ignore the presentation filters, while `matching_events`
/// reflects them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LedgerOverview {
    pub total_events: u64,
    pub matching_events: u64,
    pub oldest_occurred_at_ms: Option<i64>,
    pub newest_occurred_at_ms: Option<i64>,
    pub observed_events: u64,
    pub reconstructed_events: u64,
    pub legacy_events: u64,
    pub gap_events: u64,
    pub latest_accounting: Option<AccountingSnapshot>,
    pub latest_accounting_at_ms: Option<i64>,
    /// `channels` for an exact user_channel_id, `ledger` for the newest
    /// complete snapshot attached to any other exact reference.
    pub latest_accounting_source: Option<String>,
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
    /// Valid lines left in the JSONL file because they are not channel state changes.
    pub operational: usize,
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
         CREATE TABLE IF NOT EXISTS ledger_reconstruction_state (
            scope TEXT NOT NULL,
            identity TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            updated_at_ms INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000),
            PRIMARY KEY (scope, identity)
         );
         CREATE INDEX IF NOT EXISTS idx_ledger_events_page ON ledger_events(id DESC);
         CREATE INDEX IF NOT EXISTS idx_ledger_events_timeline
            ON ledger_events(occurred_at_ms DESC, id DESC);
         CREATE INDEX IF NOT EXISTS idx_ledger_events_filters
            ON ledger_events(category, status, completeness, id DESC);
         CREATE INDEX IF NOT EXISTS idx_ledger_refs_exact
            ON ledger_event_refs(value, role, event_id DESC);",
    )?;
    backfill_recognized_refs(conn)?;
    backfill_links(conn)
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

/// Adds flow-id refs and resolved stable ids to rows written before those refs existed. Runs once.
fn backfill_links(conn: &Connection) -> SqliteResult<()> {
    const METADATA_KEY: &str = "ledger_ref_backfill_v3_links";

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
        for (event_id, detail_json) in &events {
            let detail: Value = serde_json::from_str(detail_json).map_err(json_err)?;
            for reference in extract_refs(&detail) {
                added += conn.execute(
                    "INSERT OR IGNORE INTO ledger_event_refs (event_id, role, value)
                     VALUES (?1, ?2, ?3)",
                    params![event_id, reference.role, reference.value],
                )?;
            }
        }
        let unlinked = {
            let mut stmt = conn.prepare(
                "SELECT c.event_id, c.value FROM ledger_event_refs c
                 WHERE c.role = 'channel_id'
                   AND NOT EXISTS (
                       SELECT 1 FROM ledger_event_refs u WHERE u.event_id = c.event_id AND u.role = 'user_channel_id'
                   )",
            )?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
                .collect::<SqliteResult<Vec<_>>>()?;
            rows
        };
        let mut by_event: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        for (event_id, channel_id) in unlinked {
            by_event.entry(event_id).or_default().push(channel_id);
        }
        for (event_id, channel_ids) in by_event {
            let ids: Vec<&str> = channel_ids.iter().map(String::as_str).collect();
            add_resolved_user_channel_id(conn, event_id, &ids)?;
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

/// Stable ids a physical channel id maps to: the node's own channels row when it has one (a peer's own
/// user_channel_id also appears in payload rows), otherwise earlier single-channel rows (pre-splice ids).
fn resolve_user_channel_ids(conn: &Connection, channel_id: &str) -> SqliteResult<Vec<String>> {
    let mut found = BTreeSet::new();
    let has_channels: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'channels')",
        [],
        |row| row.get(0),
    )?;
    if has_channels {
        let mut stmt = conn.prepare("SELECT user_channel_id FROM channels WHERE channel_id = ?1 AND user_channel_id IS NOT NULL AND user_channel_id != ''")?;
        for uid in stmt.query_map(params![channel_id], |row| row.get::<_, String>(0))? {
            found.insert(uid?);
        }
        if !found.is_empty() {
            return Ok(found.into_iter().collect());
        }
    }
    let mut stmt = conn.prepare(
        "SELECT DISTINCT u.value FROM ledger_event_refs c
         JOIN ledger_event_refs u ON u.event_id = c.event_id AND u.role = 'user_channel_id'
         WHERE c.role = 'channel_id' AND c.value = ?1
           AND (SELECT COUNT(*) FROM ledger_event_refs x WHERE x.event_id = c.event_id AND x.role = 'user_channel_id') = 1
           AND (SELECT COUNT(*) FROM ledger_event_refs y WHERE y.event_id = c.event_id AND y.role = 'channel_id') = 1",
    )?;
    for uid in stmt.query_map(params![channel_id], |row| row.get::<_, String>(0))? {
        found.insert(uid?);
    }
    Ok(found.into_iter().collect())
}

/// Adds the stable id to a channel-only event when exactly one stable id matches its channel ids.
fn add_resolved_user_channel_id(conn: &Connection, event_id: i64, channel_ids: &[&str]) -> SqliteResult<()> {
    let mut uids = BTreeSet::new();
    for channel_id in channel_ids {
        uids.extend(resolve_user_channel_ids(conn, channel_id)?);
    }
    if uids.len() == 1 {
        conn.execute(
            "INSERT OR IGNORE INTO ledger_event_refs (event_id, role, value) VALUES (?1, 'user_channel_id', ?2)",
            params![event_id, uids.pop_first()],
        )?;
    }
    Ok(())
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
        if !draft.refs.iter().any(|r| r.role == "user_channel_id") {
            let channel_ids: Vec<&str> =
                draft.refs.iter().filter(|r| r.role == "channel_id").map(|r| r.value.trim()).collect();
            if !channel_ids.is_empty() {
                add_resolved_user_channel_id(conn, event_id, &channel_ids)?;
            }
        }
    }
    Ok(AppendOutcome { event_id, inserted })
}

fn state_types_sql() -> String {
    CHANNEL_STATE_EVENTS
        .iter()
        .chain(DIRECT_STATE_EVENTS)
        .map(|event| format!("'{event}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

// Identifier filters (optionally widened one hop through flow ids, matched by role and value) resolve to an id set once; a per-event EXISTS rescans refs for every row.
fn identifier_filter(identifier: &str, linked: &str) -> String {
    format!(
        "({identifier} = '' OR e.id IN (
            SELECT r.event_id FROM ledger_event_refs r WHERE r.value = {identifier}
            UNION SELECT r.event_id FROM ledger_event_refs r
                WHERE {linked} = 1 AND (r.value, r.role) IN (
                    SELECT l.value, l.role FROM ledger_event_refs o
                    JOIN ledger_event_refs l ON l.event_id = o.event_id
                    WHERE o.value = {identifier}
                      AND l.role IN ('payment_id', 'trade_id', 'settlement_id'))))"
    )
}

static LIST_EVENTS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT id, event_type, category, severity, status, source, completeness,
            occurred_at_ms, recorded_at_ms, dedup_key, before_json, after_json, detail_json
    FROM ledger_events e
    WHERE {}
      AND (?2 = '' OR e.category = ?2)
      AND (?3 = '' OR e.status = ?3)
      AND (?4 = '' OR e.completeness = ?4)
      AND (?5 = 0 OR e.occurred_at_ms < ?5
           OR (e.occurred_at_ms = ?5 AND e.id < ?6))
      AND (?9 = 0 OR e.event_type IN ({}))
    ORDER BY e.occurred_at_ms DESC, e.id DESC
    LIMIT ?7",
        identifier_filter("?1", "?8"),
        state_types_sql(),
    )
});

// The cross-channel feed: no identifier, state rows only, walked straight down the timeline index.
static RECENT_STATE_EVENTS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT id, event_type, category, severity, status, source, completeness,
            occurred_at_ms, recorded_at_ms, dedup_key, before_json, after_json, detail_json
    FROM ledger_events e
    WHERE (?1 = '' OR e.category = ?1)
      AND (?2 = '' OR e.status = ?2)
      AND (?3 = '' OR e.completeness = ?3)
      AND (?4 = 0 OR e.occurred_at_ms < ?4
           OR (e.occurred_at_ms = ?4 AND e.id < ?5))
      AND e.event_type IN ({})
    ORDER BY e.occurred_at_ms DESC, e.id DESC
    LIMIT ?6",
        state_types_sql(),
    )
});

static OVERVIEW_TOTALS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT COUNT(*), MIN(e.occurred_at_ms), MAX(e.occurred_at_ms),
        COALESCE(SUM(e.completeness = 'observed'), 0),
        COALESCE(SUM(e.completeness = 'reconstructed'), 0),
        COALESCE(SUM(e.completeness = 'legacy'), 0),
        COALESCE(SUM(e.completeness = 'gap'), 0)
    FROM ledger_events e
    WHERE {}",
        identifier_filter("?1", "?2"),
    )
});

static OVERVIEW_MATCHING_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT COUNT(*)
    FROM ledger_events e
    WHERE {}
      AND (?2 = '' OR e.category = ?2)
      AND (?3 = '' OR e.status = ?3)
      AND (?4 = '' OR e.completeness = ?4)
      AND (?6 = 0 OR e.event_type IN ({}))",
        identifier_filter("?1", "?5"),
        state_types_sql(),
    )
});

const LATEST_ACCOUNTING_SQL: &str = "SELECT e.occurred_at_ms, e.before_json, e.after_json
    FROM ledger_events e
    WHERE e.id IN (SELECT r.event_id FROM ledger_event_refs r WHERE r.value = ?1)
    ORDER BY e.occurred_at_ms DESC, e.id DESC";

pub(crate) fn list_on_connection(conn: &Connection, query: &LedgerQuery) -> SqliteResult<LedgerPage> {
    let identifier = query.identifier.as_deref().unwrap_or("").trim();
    let category = query.category.as_deref().unwrap_or("").trim();
    let status = query.status.as_deref().unwrap_or("").trim();
    let completeness = query.completeness.as_deref().unwrap_or("").trim();
    let before = query.before.unwrap_or(LedgerCursor { occurred_at_ms: 0, id: 0 });
    let limit = if query.limit == 0 { 50 } else { query.limit.min(200) };
    let mut stmt = conn.prepare(
        LIST_EVENTS_SQL.as_str(),
    )?;
    let rows = stmt.query_map(
        params![
            identifier,
            category,
            status,
            completeness,
            before.occurred_at_ms,
            before.id,
            (limit + 1) as i64,
            query.include_linked as i64,
            query.state_changes_only as i64,
        ],
        event_from_row,
    )?;
    let events = rows.collect::<SqliteResult<Vec<_>>>()?;
    drop(stmt);
    let (events, next_cursor) = finish_page(conn, events, limit)?;
    let overview = overview_on_connection(conn, query)?;
    Ok(LedgerPage { events, next_cursor, overview })
}

/// Newest channel-state events across every channel, cursor-paged like `list_on_connection`; the
/// identifier and linking are ignored and the overview stays empty (no per-identifier counts run).
pub(crate) fn list_recent_state_events(conn: &Connection, query: &LedgerQuery) -> SqliteResult<LedgerPage> {
    let category = query.category.as_deref().unwrap_or("").trim();
    let status = query.status.as_deref().unwrap_or("").trim();
    let completeness = query.completeness.as_deref().unwrap_or("").trim();
    let before = query.before.unwrap_or(LedgerCursor { occurred_at_ms: 0, id: 0 });
    let limit = if query.limit == 0 { 100 } else { query.limit.min(200) };
    let mut stmt = conn.prepare(RECENT_STATE_EVENTS_SQL.as_str())?;
    let rows = stmt.query_map(
        params![category, status, completeness, before.occurred_at_ms, before.id, (limit + 1) as i64],
        event_from_row,
    )?;
    let events = rows.collect::<SqliteResult<Vec<_>>>()?;
    drop(stmt);
    let (events, next_cursor) = finish_page(conn, events, limit)?;
    Ok(LedgerPage { events, next_cursor, overview: LedgerOverview::default() })
}

fn event_from_row(row: &rusqlite::Row<'_>) -> SqliteResult<LedgerEvent> {
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
}

/// Trims the extra row that signals another page, loads refs and returns the page chronological.
fn finish_page(
    conn: &Connection,
    mut events: Vec<LedgerEvent>,
    limit: usize,
) -> SqliteResult<(Vec<LedgerEvent>, Option<LedgerCursor>)> {
    let has_more = events.len() > limit;
    if has_more {
        events.truncate(limit);
    }
    let next_cursor = has_more
        .then(|| {
            events.last().map(|event| LedgerCursor {
                occurred_at_ms: event.occurred_at_ms,
                id: event.id,
            })
        })
        .flatten();
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
    Ok((events, next_cursor))
}

fn overview_on_connection(conn: &Connection, query: &LedgerQuery) -> SqliteResult<LedgerOverview> {
    let identifier = query.identifier.as_deref().unwrap_or("").trim();
    let category = query.category.as_deref().unwrap_or("").trim();
    let status = query.status.as_deref().unwrap_or("").trim();
    let completeness = query.completeness.as_deref().unwrap_or("").trim();

    let (
        total_events,
        oldest_occurred_at_ms,
        newest_occurred_at_ms,
        observed_events,
        reconstructed_events,
        legacy_events,
        gap_events,
    ): (i64, Option<i64>, Option<i64>, i64, i64, i64, i64) = conn.query_row(
        OVERVIEW_TOTALS_SQL.as_str(),
        params![identifier, query.include_linked as i64],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    let matching_events: i64 = conn.query_row(
        OVERVIEW_MATCHING_SQL.as_str(),
        params![identifier, category, status, completeness, query.include_linked as i64, query.state_changes_only as i64],
        |row| row.get(0),
    )?;

    let (latest_accounting, latest_accounting_at_ms, latest_accounting_source) =
        latest_accounting_on_connection(conn, identifier)?;
    Ok(LedgerOverview {
        total_events: total_events.max(0) as u64,
        matching_events: matching_events.max(0) as u64,
        oldest_occurred_at_ms,
        newest_occurred_at_ms,
        observed_events: observed_events.max(0) as u64,
        reconstructed_events: reconstructed_events.max(0) as u64,
        legacy_events: legacy_events.max(0) as u64,
        gap_events: gap_events.max(0) as u64,
        latest_accounting,
        latest_accounting_at_ms,
        latest_accounting_source,
    })
}

fn latest_accounting_on_connection(
    conn: &Connection,
    identifier: &str,
) -> SqliteResult<(Option<AccountingSnapshot>, Option<i64>, Option<String>)> {
    if identifier.is_empty() {
        return Ok((None, None, None));
    }

    // Only this exact lookup is allowed to use the mutable current-state row.
    // A channel_id/payment_id/node_id must not be guessed into a channel row.
    let channel_state: Option<(f64, i64, i64, i64)> = conn
        .query_row(
            "SELECT expected_usd, stable_sats, native_sats, updated_at
             FROM channels
             WHERE user_channel_id = ?1
             ORDER BY updated_at DESC
             LIMIT 1",
            params![identifier],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    if let Some((expected_usd, backing_sats, native_sats, updated_at)) = channel_state {
        let backing_sats = u64::try_from(backing_sats).ok();
        let native_sats = u64::try_from(native_sats).ok();
        let snapshot = AccountingSnapshot {
            expected_usd: Some(expected_usd),
            backing_sats,
            native_sats,
            live_receiver_sats: backing_sats
                .zip(native_sats)
                .map(|(backing, native)| backing.saturating_add(native)),
            ..Default::default()
        };
        return Ok((
            Some(snapshot),
            Some(updated_at.saturating_mul(1_000)),
            Some("channels".to_owned()),
        ));
    }

    let mut stmt = conn.prepare(
        LATEST_ACCOUNTING_SQL,
    )?;
    let rows = stmt.query_map(params![identifier], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (occurred_at_ms, before_json, after_json) = row?;
        let after: Option<AccountingSnapshot> = decode_optional_json(after_json)?;
        let before: Option<AccountingSnapshot> = decode_optional_json(before_json)?;
        if let Some(snapshot) = after.filter(AccountingSnapshot::is_complete) {
            return Ok((Some(snapshot), Some(occurred_at_ms), Some("ledger".to_owned())));
        }
        if let Some(snapshot) = before.filter(AccountingSnapshot::is_complete) {
            return Ok((Some(snapshot), Some(occurred_at_ms), Some("ledger".to_owned())));
        }
    }
    Ok((None, None, None))
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

        let content = match std::fs::read(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
        };
        let mut report = LegacyImportReport::default();
        for (line_no, line) in Cursor::new(content).split(b'\n').enumerate() {
            let line = line.map_err(|error| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(error))
            })?;
            let line = match std::str::from_utf8(&line) {
                Ok(line) => line,
                Err(_) => {
                    report.skipped += 1;
                    continue;
                }
            };
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
            if !records_channel_state(event_type) {
                report.operational += 1;
                continue;
            }
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
            "operational": report.operational,
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

/// Audit event types that record a change to a channel's state or the final outcome of an
/// attempt on it. Everything else (retries, internal steps, validation and I/O diagnostics) is
/// operational detail that belongs only in the JSONL audit file, never in the channel ledger.
/// Events committed inside a `db.rs` transaction bypass this and are always recorded.
pub const CHANNEL_STATE_EVENTS: &[&str] = &[
    // Channel lifecycle, splices and sweeps.
    "CHANNEL_PENDING",
    "CHANNEL_READY",
    "CHANNEL_READY_SPLICE",
    "CHANNEL_READY_TRACKED",
    "CHANNEL_OPEN_FAILED",
    "CHANNEL_CLOSED",
    "CHANNEL_SHUTDOWN_STATE_CHANGED",
    "CHANNEL_MARKED_CLOSED_AT_STARTUP",
    "CHANNEL_ID_UPDATED_SPLICE",
    "CHANNEL_RECONSTRUCTED",
    "CHANNEL_STATE_UNKNOWN",
    "CHANNEL_ONCHAIN_TX",
    "SPLICE_PENDING",
    "SPLICE_NEGOTIATED",
    "SPLICE_NEGOTIATION_FAILED",
    "SPLICE_FAILED",
    "SPLICE_IN_RECONCILED",
    "SPLICE_OUT_STABLE_DEDUCTED",
    "SPLICE_OUT_STABLE_RECONCILED",
    "SPLICE_RECONSTRUCTED",
    "AUTO_SPLICE_CONFIRMED",
    "SWEEP_TO_CHANNEL",
    "SWEEP_PROGRESS",
    "SWEEP_RECONSTRUCTED",
    // Payments and their final outcomes.
    "PAYMENT_CLAIMABLE",
    "PAYMENT_RECEIVED",
    "PAYMENT_SUCCESSFUL",
    "PAYMENT_SETTLED",
    "PAYMENT_FAILED",
    "PAYMENT_FORWARDED",
    "PAYMENT_FORWARDED_BACKFILL",
    "PAYMENT_RECONSTRUCTED",
    "PAYMENT_BACKING_CLAMPED",
    "ONCHAIN_DEPOSIT_DETECTED",
    "WEBSOCKET_INSTANT_PAYMENT_RECORDED",
    "WEBSOCKET_RBF_FAILED_PAYMENT",
    // Stable allocation changes and operator edits.
    "BACKSTOP_STABLE_DEDUCTED",
    "OUTGOING_STABLE_DEDUCTED",
    "STABLE_SPEND_DEDUCTED",
    "OVERBACKED_ALLOCATION_REPAIRED",
    "MAX_STABILIZATION_REJECTED",
    "STABLE_EDITED",
    "OPERATOR_NOTE_EDITED",
    // Stability settlements, wake-ups and their outcomes.
    "STABILITY_PAYMENT_V1_SENT",
    "STABILITY_PAYMENT_V1_APPLIED",
    "STABILITY_PAYMENT_FAILED",
    "STABILITY_RECEIVED_RECONCILED",
    "STABILITY_RECEIVE_UNATTRIBUTED",
    "STABILITY_PUSH_QUEUED",
    "STABILITY_CHECK_ONLY",
    "STABILITY_WAKE_POLL_ONLINE",
    // SYNC publications and their final outcomes (never each retry).
    "SYNC_MESSAGE_SENT",
    "SYNC_RETRY_EXHAUSTED",
    "SYNC_RETRY_BLOCKED",
    "SYNC_PENDING_ABANDONED",
    "SYNC_V1_APPLIED",
    "SYNC_V1_ALLOCATION_REJECTED",
    // Trades and their final outcomes.
    "TRADE_MESSAGE_SENT",
    "TRADE_ACCEPTED",
    "TRADE_APPLIED",
    "TRADE_REJECTION_QUEUED",
    "TRADE_REJECTED_BY_LSP",
    "TRADE_FAILED",
    "TRADE_FEE_CONFIRMED_AFTER_SYNC",
    "TRADE_FEE_CONFIRMED_AWAITING_SYNC",
    "TRADE_ID_REUSED",
    // Peer reachability.
    "PEER_CONNECTED",
    "PEER_DISCONNECTED",
    "PEER_RECONSTRUCTED",
    // Ledger completeness markers.
    "EVENT_STREAM_GAP_STARTED",
    "EVENT_STREAM_GAP_CLOSED",
    "RECONCILIATION_GAP_DETECTED",
    "RECONCILIATION_RESULT",
    "RECONCILIATION_SCOPE_FAILED",
    // Operator refunds of rejected trade fees: money left the node, so the record must survive.
    "TRADE_FEE_REFUND_SENT",
    "TRADE_FEE_REFUND_OUTCOME_UNKNOWN",
    // Integrity alarms: a store or replay that went wrong must survive a restart.
    "STABILITY_PAYMENT_STATE_DIVERGENCE",
    "STABILITY_PAYMENT_REPLAY_CONFLICT",
    "STABILITY_PAYMENT_PERSIST_FAILED",
    "TRADE_RESPONSE_PAYMENT_ID_PERSIST_FAILED",
    "DB_WRITE_FAILED",
    "DB_READ_FAILED",
    "STABILITY_PAYMENT_REPLAY_IGNORED",
    "STABILITY_PAYMENT_AMOUNT_MISMATCH",
    "STABILITY_PAYMENT_CHANNEL_MISMATCH",
    "SYNC_V1_CHANNEL_MISMATCH",
    "TRADE_PAYMENT_UNATTRIBUTABLE",
    "ONCHAIN_DEPOSIT_PERSIST_FAILED",
    "OUTGOING_RECONCILE_PERSIST_FAILED",
    "OVERBACKED_REPAIR_PERSIST_FAILED",
    "PAYMENT_PERSIST_FAILED",
    "STABILITY_PAYMENT_FAILURE_PERSIST_FAILED",
    "STABILITY_PAYMENT_SUCCESS_PERSIST_FAILED",
    "TRADE_FEE_STATUS_PERSIST_FAILED",
    "TRADE_INTENT_PERSIST_FAILED",
    "TRADE_PAYMENT_FAILURE_PERSIST_FAILED",
    "TRADE_PAYMENT_ID_PERSIST_FAILED",
    // Failure records the operator needs next to the money they concern.
    "ONCHAIN_DEPOSIT_COMPLETION_FAILED",
    "OUTGOING_PAYMENT_CLASSIFICATION_FAILED",
    "TRADE_PAYMENT_CLASSIFICATION_FAILED",
    "CHANNEL_READY_UID_UNPARSEABLE",
    "TRADE_PARSE_PAYLOAD_FAILED",
    "TRADE_PARSE_SIGNED_FAILED",
    "TRADE_CHANNEL_UID_UNPARSEABLE",
    // --- Added 2026-10-02: every emitted name is classified; these record a money or channel
    // outcome, or the reason one did not happen, and were in SQLite on main.
    // Shared accounting (src/stable.rs)
    "BALANCE_UPDATE", "BALANCE_UPDATE_FAILED", "OVERBACKED_REPAIR_SKIPPED_PENDING_HTLC",
    "STABILITY_SKIP", "STABILITY_SKIP_HTLC_SAFETY", "STABILITY_PAYMENT_SERIALIZE_FAILED",
    // Stability settlement decisions and failures (daemon and client)
    "STABILITY_SKIP_HIGH_RISK", "STABILITY_PAYMENT_BINDING_INVALID", "STABILITY_PAYMENT_CHANNEL_LOOKUP_FAILED",
    "STABILITY_PAYMENT_CHANNEL_UNAVAILABLE", "STABILITY_PAYMENT_EXPIRED", "STABILITY_PAYMENT_PAYLOAD_INVALID",
    "STABILITY_PAYMENT_PRICE_UNAVAILABLE", "STABILITY_PAYMENT_SIGNATURE_CHECK_FAILED", "STABILITY_PAYMENT_SIGNATURE_INVALID",
    "STABILITY_PAYMENT_SIGN_FAILED", "STABILITY_PAYMENT_ALLOCATION_INVALID", "STABILITY_PAYMENT_ALLOCATION_RETRY_DEFERRED",
    "LEGACY_STABILITY_MARKER_INVALID", "LEGACY_STABILITY_MARKER_UNAUTHENTICATED",
    // Trade decisions: why a trade was or was not applied, and whether the answer reached the user
    "MESSAGE_RECEIVED", "TRADE_PARSED_PAYLOAD_OK", "TRADE_SIGNATURE_VALID", "TRADE_SIGNATURE_INVALID",
    "TRADE_ALLOCATION_REJECTED", "TRADE_CHANNEL_NOT_FOUND", "TRADE_CORRELATION_INVALID", "TRADE_EXCEEDS_BALANCE",
    "TRADE_FEE_INVALID", "TRADE_INVALID_AMOUNT", "TRADE_INVALID_QUOTE", "TRADE_QUOTE_DEVIATION_EXCEEDED",
    "TRADE_STABLE_ENTRY_NOT_FOUND", "TRADE_STALE", "TRADE_UNHANDLED_TYPE", "TRADE_REJECTION_SIGN_FAILED",
    "TRADE_RESPONSE_SENT", "TRADE_RESPONSE_SEND_FAILED", "LDK_CALL_FAILED",
    // Client-side reconciliation of received money (src/user.rs)
    "ONCHAIN_DEPOSIT_DEFERRED", "ONCHAIN_OUTBOUND_CONFIRMATION_FAILED", "OUTGOING_RECONCILE_DEFERRED_NO_PRICE",
    "SPLICE_OUT_RECONCILE_DEFERRED", "SPLICE_OUT_RECONCILE_DEFERRED_NO_PRICE", "SPLICE_OUT_LOOKUP_STATE_INVALID",
    "SPLICE_RECONCILE_SKIPPED_ALREADY_DEDUCTED", "SPLICE_PENDING_LOOKUP_FAILED", "PAYMENT_RECEIVED_IGNORED",
    "LIGHTNING_RECEIVE_FAILED", "JIT_INVOICE_FAILED", "INVOICE_GENERATION_FAILED", "INVOICE_INPUT_INVALID",
    "SYNC_V1_PROCESSED", "SYNC_V1_PAYLOAD_INVALID", "SYNC_V1_CORRELATION_INVALID", "SYNC_V1_CORRELATED_AMOUNT_INVALID",
    "TRADE_LOCAL_ALLOCATION_REJECTED", "TRADE_MESSAGE_FAILED", "TRADE_RESULT_SIGNATURE_INVALID",
    "TRADE_REJECTED_V1_CONTEXT_INVALID", "TRADE_REJECTED_V1_PAYLOAD_INVALID", "TRADE_REJECTED_V1_UNMATCHED",
    // Integrity and authentication
    "EVENT_STREAM_COVERAGE_UNKNOWN", "REGISTER_PUSH_LEGACY_INVALID", "REGISTER_PUSH_SIGNATURE_INVALID",
];

/// True when an audit event belongs in the channel ledger.
pub fn records_channel_state(event_type: &str) -> bool {
    CHANNEL_STATE_EVENTS.contains(&event_type)
}

/// State rows written straight into the ledger by db.rs transactions and reconnect reconstruction, plus older-daemon names.
const DIRECT_STATE_EVENTS: &[&str] = &[
    "CHANNEL_ACCOUNTING_STATE_COMMITTED",
    "CHANNEL_CLOSED_COMMITTED",
    "SYNC_V1_APPLIED",
    "PAYMENT_OUTGOING_RECONCILED",
    "PAYMENT_RECORDED",
    "STABILITY_PAYMENT_SENT",
    "STABILITY_PAYMENT_SETTLED",
    "STABILITY_PAYMENT_RECORDED",
    "STABILITY_PAYMENT_FAILED_RECONCILED",
    "STABILITY_PAYMENT_ROLLED_BACK",
    "SPLICE_RECONCILED",
    "TRADE_RESERVED",
];

/// Events that are deliberately JSONL-only: high-volume or transport noise whose durable signal
/// is carried by another record. Every emitted event name must be in exactly one of the three lists.
pub const OPERATIONAL_EVENTS: &[&str] = &[
    // One row per sync keysend attempt, hundreds an hour against offline phones; SYNC_RETRY_BLOCKED
    // and SYNC_RETRY_EXHAUSTED are the durable records of a channel that cannot be reached.
    "SYNC_MESSAGE_FAILED",
    // A wake watch starting or expiring is diagnostic; only the reconnect reaches the channel ledger.
    "STABILITY_WAKE_POLL_STARTED",
    "STABILITY_WAKE_POLL_TIMEOUT",
    // Event-stream transport; EVENT_STREAM_GAP_STARTED, EVENT_STREAM_GAP_CLOSED and RECONCILIATION_RESULT are the durable records.
    "EVENT_STREAM_CONNECTED", "EVENT_STREAM_CONNECT_FAILED", "EVENT_STREAM_DISCONNECTED", "RECONCILIATION_STARTED",
    // Price-feed transport.
    "WEBSOCKET_DISCONNECTED",
    // Per-tick and per-attempt traces whose outcome is recorded elsewhere, transport, and UI.
    "STABILITY_CHECK", "STABILITY_COOLDOWN", "STABILITY_PAY_COOLDOWN_CHECK", "RECONCILE_FORWARDED_COOLDOWN_SET",
    "TRADE_PROTOCOL_PATH", "CHANNEL_EXISTS_CHECK", "REGISTER_PUSH_OK", "REGISTER_PUSH_LEGACY_OK",
    "EVENT_IGNORED", "INVOICE_GENERATED", "JIT_INVOICE_ATTEMPT", "JIT_INVOICE_GENERATED", "LIGHTNING_RECEIVE_INVOICE",
    "QR_GENERATION_FAILED", "SPLICE_PENDING_LOOKUP",
    "WEBSOCKET_CONNECTED", "WEBSOCKET_CONNECT_FAILED", "WEBSOCKET_TRACKING_FAILED",
];

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
    if event.contains("FAILED") || event.contains("ERROR") || event.contains("REJECTED") || event.contains("CONFLICT") || event.contains("DIVERGENCE") {
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
                        "payment_id" | "payment_ids" | "trade_payment_id" => Some("payment_id"),
                        "trade_id" | "trade_ids" => Some("trade_id"),
                        "settlement_id" | "settlement_ids" => Some("settlement_id"),
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
    fn legacy_import_keeps_operational_lines_out_of_the_ledger() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audit_log.txt");
        std::fs::write(
            &path,
            concat!(
                r#"{"ts":"2026-07-01T00:00:00Z","event":"SYNC_MESSAGE_FAILED","data":{"user_channel_id":"u"}}"#, "\n",
                r#"{"ts":"2026-07-01T00:01:00Z","event":"PAYMENT_SETTLED","data":{"user_channel_id":"u","payment_id":"p"}}"#, "\n",
            ),
        )
        .unwrap();
        let report = import_legacy_jsonl(&conn, &path).unwrap();
        assert_eq!(report.imported, 1);
        let types: Vec<String> = conn
            .prepare("SELECT event_type FROM ledger_events")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<SqliteResult<_>>()
            .unwrap();
        assert_eq!(types, ["PAYMENT_SETTLED"]);
    }

    fn refs_of(conn: &Connection, event_id: i64) -> BTreeSet<(String, String)> {
        conn.prepare("SELECT role, value FROM ledger_event_refs WHERE event_id = ?1")
            .unwrap()
            .query_map(params![event_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<SqliteResult<_>>()
            .unwrap()
    }

    #[test]
    fn flow_ids_become_refs() {
        let draft = LedgerEventDraft::from_audit_event(
            "TRADE_ACCEPTED",
            serde_json::json!({"trade_id": "t-1", "trade_payment_id": "in-1", "settlement_id": "s-1"}),
        );
        for (role, value) in [("trade_id", "t-1"), ("payment_id", "in-1"), ("settlement_id", "s-1")] {
            assert!(draft.refs.contains(&LedgerRef::new(role, value)), "{role}");
        }
    }

    #[test]
    fn channel_only_events_gain_their_stable_identity() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE channels (channel_id TEXT, user_channel_id TEXT);
             INSERT INTO channels VALUES ('live-chan', 'uid-7');",
        )
        .unwrap();
        // A pre-splice id is known only from an earlier single-channel row.
        append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "CHANNEL_ACCOUNTING_STATE_COMMITTED",
            serde_json::json!({"user_channel_id": "uid-42", "channel_id": "old-chan"}),
        ))
        .unwrap();
        // A forward names two channels and two stable ids; it must not become a mapping.
        append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "PAYMENT_FORWARDED",
            serde_json::json!({"prev_channel_id": "old-chan", "next_channel_id": "fwd-chan",
                "prev_user_channel_id": "uid-42", "next_user_channel_id": "uid-99"}),
        ))
        .unwrap();
        let append = |channel: &str, payment: &str| {
            append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
                "STABILITY_PAYMENT_V1_APPLIED",
                serde_json::json!({"channel_id": channel, "payment_id": payment}),
            ))
            .unwrap()
            .event_id
        };
        let (live, old, ambiguous) = (append("live-chan", "p-1"), append("old-chan", "p-2"), append("fwd-chan", "p-3"));
        let uid = |id| {
            refs_of(&conn, id)
                .into_iter()
                .filter(|(role, _)| role == "user_channel_id")
                .map(|(_, value)| value)
                .collect::<Vec<_>>()
        };
        assert_eq!(uid(live), ["uid-7"]);
        assert_eq!(uid(old), ["uid-42"]);
        assert!(uid(ambiguous).is_empty());
    }

    #[test]
    fn the_live_channels_row_outranks_the_peer_stable_id_seen_in_payloads() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE channels (channel_id TEXT, user_channel_id TEXT);
             INSERT INTO channels VALUES ('shared-chan', 'lsp-uid');",
        )
        .unwrap();
        // Each peer names the channel with its own user_channel_id; the wallet's arrives in trade payloads.
        append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "TRADE_SIGNATURE_VALID",
            serde_json::json!({"user_channel_id": "wallet-uid", "channel_id": "shared-chan"}),
        ))
        .unwrap();
        let applied = append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "STABILITY_PAYMENT_V1_APPLIED",
            serde_json::json!({"channel_id": "shared-chan", "payment_id": "p-9"}),
        ))
        .unwrap()
        .event_id;
        assert!(refs_of(&conn, applied).contains(&("user_channel_id".to_owned(), "lsp-uid".to_owned())));
    }

    #[test]
    fn a_channels_row_without_a_stable_id_never_fails_the_write() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE channels (channel_id TEXT, user_channel_id TEXT);
             INSERT INTO channels VALUES ('legacy-chan', NULL);",
        )
        .unwrap();
        let outcome = append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "STABILITY_PAYMENT_V1_APPLIED",
            serde_json::json!({"channel_id": "legacy-chan"}),
        ))
        .unwrap();
        assert!(refs_of(&conn, outcome.event_id).iter().all(|(role, _)| role != "user_channel_id"));
    }

    #[test]
    fn resolution_is_skipped_without_a_channels_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let outcome = append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "STABILITY_PAYMENT_V1_APPLIED",
            serde_json::json!({"channel_id": "unknown"}),
        ))
        .unwrap();
        assert!(outcome.inserted);
        assert!(refs_of(&conn, outcome.event_id).iter().all(|(role, _)| role != "user_channel_id"));
    }

    #[test]
    fn links_backfill_adds_refs_to_existing_rows_once() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "CHANNEL_ACCOUNTING_STATE_COMMITTED",
            serde_json::json!({"user_channel_id": "uid-1", "channel_id": "chan-1"}),
        ))
        .unwrap();
        let applied = append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "STABILITY_PAYMENT_V1_APPLIED",
            serde_json::json!({"channel_id": "chan-1", "settlement_id": "s-1"}),
        ))
        .unwrap()
        .event_id;
        // Simulate a row written before this change: strip the new refs and the completion marker.
        conn.execute(
            "DELETE FROM ledger_event_refs WHERE event_id = ?1 AND role IN ('user_channel_id', 'settlement_id')",
            params![applied],
        )
        .unwrap();
        conn.execute("DELETE FROM ledger_metadata WHERE key = 'ledger_ref_backfill_v3_links'", []).unwrap();

        backfill_links(&conn).unwrap();
        let refs = refs_of(&conn, applied);
        assert!(refs.contains(&("user_channel_id".to_owned(), "uid-1".to_owned())));
        assert!(refs.contains(&("settlement_id".to_owned(), "s-1".to_owned())));

        conn.execute("DELETE FROM ledger_event_refs WHERE event_id = ?1 AND role = 'settlement_id'", params![applied]).unwrap();
        backfill_links(&conn).unwrap();
        assert!(!refs_of(&conn, applied).iter().any(|(role, _)| role == "settlement_id"), "runs once");
    }

    #[test]
    fn linked_history_reaches_flow_rows_and_state_filter_hides_noise() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // Listing by identifier also reads the live accounting row, as it does against the full schema.
        conn.execute_batch(
            "CREATE TABLE channels (channel_id TEXT, user_channel_id TEXT, expected_usd REAL,
                stable_sats INTEGER, native_sats INTEGER, updated_at INTEGER);",
        )
        .unwrap();
        for (event, detail) in [
            ("SYNC_MESSAGE_SENT", serde_json::json!({"user_channel_id": "uid-1", "payment_id": "sync-pay"})),
            ("PAYMENT_SETTLED", serde_json::json!({"payment_id": "sync-pay", "amount_msat": 1})),
            ("SYNC_MESSAGE_FAILED", serde_json::json!({"user_channel_id": "uid-1", "stage": "send"})),
            ("TRADE_ACCEPTED", serde_json::json!({"trade_id": "t-9", "trade_payment_id": "in-9"})),
            ("TRADE_APPLIED", serde_json::json!({"user_channel_id": "uid-1", "trade_id": "t-9"})),
        ] {
            append_on_connection(&conn, &LedgerEventDraft::from_audit_event(event, detail)).unwrap();
        }
        let types = |linked: bool, state_only: bool| {
            let page = list_on_connection(&conn, &LedgerQuery {
                identifier: Some("uid-1".to_owned()),
                include_linked: linked,
                state_changes_only: state_only,
                limit: 50,
                ..Default::default()
            })
            .unwrap();
            assert_eq!(page.overview.matching_events as usize, page.events.len());
            page.events.into_iter().map(|e| e.event_type).collect::<BTreeSet<_>>()
        };
        let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(types(false, false), set(&["SYNC_MESSAGE_SENT", "SYNC_MESSAGE_FAILED", "TRADE_APPLIED"]));
        assert_eq!(
            types(true, false),
            set(&["SYNC_MESSAGE_SENT", "PAYMENT_SETTLED", "SYNC_MESSAGE_FAILED", "TRADE_ACCEPTED", "TRADE_APPLIED"])
        );
        assert_eq!(types(true, true), set(&["SYNC_MESSAGE_SENT", "PAYMENT_SETTLED", "TRADE_ACCEPTED", "TRADE_APPLIED"]));
    }

    #[test]
    fn recent_state_feed_spans_channels_newest_first_and_pages() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        for (event, detail) in [
            ("CHANNEL_PENDING", serde_json::json!({"user_channel_id": "uid-1"})),
            ("SYNC_MESSAGE_FAILED", serde_json::json!({"user_channel_id": "uid-1", "stage": "send"})),
            ("TRADE_APPLIED", serde_json::json!({"user_channel_id": "uid-2", "trade_id": "t-1"})),
            ("PAYMENT_FAILED", serde_json::json!({"user_channel_id": "uid-1", "payment_id": "p-1"})),
        ] {
            append_on_connection(&conn, &LedgerEventDraft::from_audit_event(event, detail)).unwrap();
        }
        let query = |before: Option<LedgerCursor>, limit: usize| LedgerQuery {
            identifier: Some("ignored".to_owned()),
            before,
            limit,
            ..Default::default()
        };
        let all = list_recent_state_events(&conn, &query(None, 50)).unwrap();
        let types: Vec<&str> = all.events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(types, ["CHANNEL_PENDING", "TRADE_APPLIED", "PAYMENT_FAILED"], "operational rows stay out, page is chronological");
        assert!(all.events.iter().all(|e| !e.refs.is_empty()), "refs are loaded so readers can split by channel");
        assert_eq!(all.overview, LedgerOverview::default());
        assert_eq!(all.next_cursor, None);

        let first = list_recent_state_events(&conn, &query(None, 2)).unwrap();
        assert_eq!(first.events.iter().map(|e| e.event_type.as_str()).collect::<Vec<_>>(), ["TRADE_APPLIED", "PAYMENT_FAILED"]);
        let cursor = first.next_cursor.expect("an older page remains");
        let second = list_recent_state_events(&conn, &query(Some(cursor), 2)).unwrap();
        assert_eq!(second.events.iter().map(|e| e.event_type.as_str()).collect::<Vec<_>>(), ["CHANNEL_PENDING"]);
        assert_eq!(second.next_cursor, None);
    }

    #[test]
    fn recent_state_feed_clamps_the_page_size() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        for i in 0..205 {
            append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
                "PEER_CONNECTED",
                serde_json::json!({"user_channel_id": format!("uid-{i}"), "n": i}),
            ))
            .unwrap();
        }
        let page = |limit: usize| list_recent_state_events(&conn, &LedgerQuery { limit, ..Default::default() }).unwrap().events.len();
        assert_eq!(page(0), 100, "default page");
        assert_eq!(page(500), 200, "hard cap");
        assert_eq!(page(3), 3);
    }

    #[test]
    fn linked_history_matches_flow_ids_by_role() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE channels (channel_id TEXT, user_channel_id TEXT, expected_usd REAL,
                stable_sats INTEGER, native_sats INTEGER, updated_at INTEGER);",
        )
        .unwrap();
        // A wallet chooses its settlement id; one equal to another channel's funding txid must not link them.
        append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "CHANNEL_PENDING",
            serde_json::json!({"user_channel_id": "other-uid", "funding_txo": "abc123"}),
        ))
        .unwrap();
        append_on_connection(&conn, &LedgerEventDraft::from_audit_event(
            "STABILITY_PAYMENT_V1_SENT",
            serde_json::json!({"user_channel_id": "uid-1", "settlement_id": "abc123"}),
        ))
        .unwrap();
        let page = list_on_connection(&conn, &LedgerQuery {
            identifier: Some("uid-1".to_owned()),
            include_linked: true,
            limit: 50,
            ..Default::default()
        })
        .unwrap();
        let types: Vec<String> = page.events.into_iter().map(|e| e.event_type).collect();
        assert_eq!(types, ["STABILITY_PAYMENT_V1_SENT"]);
    }

    #[test]
    fn identifier_filters_resolve_refs_once_not_per_event() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // A per-event correlated lookup walks every ref of a busy identifier for each row (minutes on a live LSP).
        for sql in [LIST_EVENTS_SQL.as_str(), OVERVIEW_TOTALS_SQL.as_str(), OVERVIEW_MATCHING_SQL.as_str(), LATEST_ACCOUNTING_SQL] {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let mut rows = stmt.raw_query();
            let mut plan = Vec::new();
            while let Some(row) = rows.next().unwrap() {
                plan.push(row.get::<_, String>(3).unwrap());
            }
            assert!(!plan.iter().any(|step| step.contains("CORRELATED")), "{plan:?}");
        }
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
    fn repeated_business_events_require_an_explicit_replay_identity_to_deduplicate() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let event = LedgerEventDraft::from_audit_event(
            "PAYMENT_FAILED",
            serde_json::json!({"payment_id": "retryable-payment"}),
        );

        assert!(append_on_connection(&conn, &event).unwrap().inserted);
        assert!(append_on_connection(&conn, &event).unwrap().inserted);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE event_type = 'PAYMENT_FAILED'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
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

    #[test]
    fn splice_ready_dedup_uses_funding_outpoint_as_event_identity() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let splice = |funding_txo: &str| {
            LedgerEventDraft::from_audit_event(
                "CHANNEL_READY_SPLICE",
                serde_json::json!({
                    "user_channel_id": "stable-channel",
                    "channel_id": "logical-channel",
                    "funding_txo": funding_txo,
                    "dedup_key": format!("lsp:channel-ready-splice:stable-channel:{funding_txo}"),
                    "deducted": false,
                }),
            )
        };

        let first = append_on_connection(&conn, &splice("funding-one:0")).unwrap();
        let replay = append_on_connection(&conn, &splice("funding-one:0")).unwrap();
        let second = append_on_connection(&conn, &splice("funding-two:0")).unwrap();

        assert!(first.inserted, "the first splice must be recorded");
        assert!(!replay.inserted, "a replay of the same funding outpoint must deduplicate");
        assert!(second.inserted, "a new funding outpoint is a distinct splice");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE event_type = 'CHANNEL_READY_SPLICE'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn splice_ready_extracts_direction_amount_and_balance_change() {
        let draft = LedgerEventDraft::from_audit_event(
            "CHANNEL_READY_SPLICE",
            serde_json::json!({
                "user_channel_id": "stable-channel",
                "channel_id": "logical-channel",
                "funding_txo": "funding:0",
                "direction": "in",
                "amount_sats": 9_769,
                "before_live_receiver_sats": 154_516,
                "after_live_receiver_sats": 164_285,
                "before_btc_price": 65_000.0,
                "btc_price": 65_000.0,
            }),
        );

        assert_eq!(draft.detail["direction"], "in");
        assert_eq!(draft.before.as_ref().and_then(|state| state.live_receiver_sats), Some(154_516));
        assert_eq!(draft.after.as_ref().and_then(|state| state.live_receiver_sats), Some(164_285));
        assert_eq!(draft.after.as_ref().and_then(|state| state.amount_sats), Some(9_769));
    }

    /// The source above a file's first test-gated module; a test-gated item among production code does not end it.
    fn production_source(text: &str) -> &str {
        let gate = "#[cfg(test)]";
        let mut searched = 0;
        while let Some(found) = text[searched..].find(gate) {
            let start = searched + found;
            searched = start + gate.len();
            let item = text[searched..].trim_start();
            let item = item.strip_prefix("pub(crate) ").or_else(|| item.strip_prefix("pub ")).unwrap_or(item);
            if item.starts_with("mod ") {
                return &text[..start];
            }
        }
        text
    }

    #[test]
    fn production_source_ends_at_the_test_module_not_at_a_test_gated_item() {
        let source = "fn a() {}\n#[cfg(test)]\npub(crate) fn helper() {}\nfn b() {}\n#[cfg(test)]\npub(crate) mod testing {}\nfn c() {}\n";
        assert_eq!(production_source(source), "fn a() {}\n#[cfg(test)]\npub(crate) fn helper() {}\nfn b() {}\n");
        assert_eq!(production_source("fn a() {}\n"), "fn a() {}\n");
    }

    #[test]
    fn every_emitted_event_is_explicitly_classified() {
        // Every string literal handed to audit_event / record_event in non-test code must sit in
        // exactly one of the three lists, so a new money record cannot default to "dropped" and a
        // name deleted from CHANNEL_STATE_EVENTS fails here rather than silently going JSONL-only.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut dirs = vec![root.join("src"), root.join("server/stable-channels-lsp/src")];
        let mut unclassified = Vec::new();
        let mut seen = 0;
        while let Some(dir) = dirs.pop() {
            for path in std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().path()) {
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let production = production_source(&text);
                for call in ["audit_event(", "record_event("] {
                    for (idx, _) in production.match_indices(call) {
                        let rest = production[idx + call.len()..].trim_start();
                        let Some(literal) = rest.strip_prefix('"') else { continue };
                        let Some(end) = literal.find('"') else { continue };
                        let name = &literal[..end];
                        seen += 1;
                        let lists = [CHANNEL_STATE_EVENTS, DIRECT_STATE_EVENTS, OPERATIONAL_EVENTS];
                        let hits = lists.iter().filter(|list| list.contains(&name)).count();
                        if hits != 1 {
                            unclassified.push(format!("{name} x{hits} ({})", path.display()));
                        }
                    }
                }
            }
        }
        unclassified.sort();
        unclassified.dedup();
        // Keep the floor just under the real count, so a scan that stops reaching part of the tree fails here.
        assert!(seen >= 285, "the scan found only {seen} emitted events; it no longer reaches every production call site");
        assert!(unclassified.is_empty(), "events no list (or two lists) classify: {unclassified:?}");
        for name in OPERATIONAL_EVENTS {
            assert!(!records_channel_state(name), "{name} is operational and must not reach the ledger");
        }
    }

    #[test]
    fn every_integrity_alarm_in_the_tree_reaches_the_ledger() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut dirs = vec![root.join("src"), root.join("server/stable-channels-lsp/src")];
        let mut missing = Vec::new();
        while let Some(dir) = dirs.pop() {
            for path in std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().path()) {
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    // Every segment between quotes, so an escaped quote cannot hide a literal.
                    for word in std::fs::read_to_string(&path).unwrap().split('"') {
                        let alarm = word.ends_with("_PERSIST_FAILED")
                            || word.ends_with("_UNATTRIBUTABLE")
                            || (word.starts_with("DB_") && word.ends_with("_FAILED"))
                            || word.contains("_REPLAY_")
                            || word.ends_with("_DIVERGENCE")
                            || word.ends_with("_MISMATCH")
                            || word == "SYNC_RETRY_BLOCKED";
                        let identifier = word.starts_with(|c: char| c.is_ascii_uppercase())
                            && word.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
                        if alarm && identifier && !CHANNEL_STATE_EVENTS.contains(&word) && !DIRECT_STATE_EVENTS.contains(&word) {
                            missing.push(format!("{word} ({})", path.display()));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "alarms the ledger would drop: {missing:?}");
    }
}
