//! SQLite database layer for Stable Channels user data.
//!
//! This module provides isolated database operations for storing:
//! - Channel settings (expected_usd, notes)
//! - Trade history
//! - Price history (for charts and analytics)

use chrono::{Duration as ChronoDuration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::ledger::{
    self, AccountingSnapshot, AppendOutcome, LedgerCompleteness, LedgerEventDraft, LedgerPage,
    LedgerQuery, LedgerRef, LegacyImportReport,
};

/// Outcome of `record_payment_and_maybe_update_backing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentPersistence {
    /// True if the payment row was newly inserted, false if it was a duplicate.
    pub is_new: bool,
    /// Authoritative `stable_sats` value committed to the DB, when a backing
    /// update was requested and applied. Callers should sync in-memory state
    /// from this rather than re-applying the delta themselves.
    pub new_backing: Option<i64>,
    /// True if `current + delta` went below zero and was clamped to 0.
    pub clamped: bool,
}

/// Result of consuming a failed outbound stability settlement.
#[derive(Debug, Clone, PartialEq)]
pub struct StabilityRollback {
    pub user_channel_id: String,
    pub backing_sats_before: u64,
    pub backing_sats_after: u64,
    pub native_sats_before: u64,
    pub expected_usd: f64,
    pub last_stability_payment_before: i64,
    /// False means the settlement was marked failed, but a newer allocation had already replaced
    /// the optimistic state, so the channel row was intentionally left untouched.
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StabilityPaymentRollback {
    pub user_channel_id: Option<String>,
    pub backing_sats_before: Option<u64>,
    pub backing_sats_after: Option<u64>,
    /// True only when the channel still held this payment's optimistic allocation and was restored.
    pub restored: bool,
}

/// Returns true if `err` is the distinct missing-channel-row condition from
/// `record_payment_and_maybe_update_backing` — i.e. a backing update was
/// requested but no `channels` row exists for the user_channel_id. Callers
/// can recreate the row and retry.
pub fn is_missing_channel_row(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::QueryReturnedNoRows)
}

/// Database file name
pub const DB_FILENAME: &str = "stablechannels.db";

/// Thread-safe database handle
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

/// Stable dedup key for a forwarded payment (the proto gives forwards no unique id).
pub fn forward_fingerprint(
    prev_channel_id: &str,
    next_channel_id: &str,
    outbound_amount_msat: Option<u64>,
    total_fee_msat: Option<u64>,
) -> String {
    format!(
        "{}|{}|{}|{}",
        prev_channel_id,
        next_channel_id,
        outbound_amount_msat.unwrap_or(0),
        total_fee_msat.unwrap_or(0)
    )
}

/// A still-pending trade recoverable after a restart (the in-memory pending-trade map is empty on launch).
pub struct PendingTradeRow {
    pub id: i64,
    pub channel_id: String,
    pub trade_id: Option<String>,
    pub payment_id: Option<String>,
    pub fee_msat: u64,
    pub new_expected_usd: f64,
    pub btc_price: f64,
    pub new_backing_sats: Option<u64>,
    pub action: String,
    pub status: String,
}

fn pending_trade_from_row(row: &rusqlite::Row<'_>) -> SqliteResult<PendingTradeRow> {
    Ok(PendingTradeRow {
        id: row.get(0)?,
        channel_id: row.get(1)?,
        trade_id: row.get(2)?,
        payment_id: row.get(3)?,
        fee_msat: row.get::<_, i64>(4)?.max(0) as u64,
        new_expected_usd: row.get(5)?,
        btc_price: row.get(6)?,
        new_backing_sats: row.get::<_, Option<i64>>(7)?.map(|value| value.max(0) as u64),
        action: row.get(8)?,
        status: row.get(9)?,
    })
}

/// A durable LSP response to an authenticated inbound trade. The signed envelope and amount are
/// stored before any send is attempted, making retries independent of the original LDK event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTradeResponse {
    pub inbound_payment_id: String,
    pub counterparty: String,
    pub response_envelope: String,
    pub response_amount_msat: u64,
    pub attempts: u32,
}

fn finish_transaction<T>(conn: &Connection, result: SqliteResult<T>) -> SqliteResult<T> {
    match result {
        Ok(value) => match conn.execute_batch("COMMIT") {
            Ok(()) => Ok(value),
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            },
        },
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        },
    }
}

impl Database {
    /// Open or create the database at the given directory path.
    pub fn open(data_dir: &Path) -> SqliteResult<Self> {
        let db_path = data_dir.join(DB_FILENAME);

        // Ensure directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(&db_path)?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.init_schema()?;
        Ok(db)
    }

    /// Open an in-memory database (for testing)
    #[cfg(test)]
    pub fn open_in_memory() -> SqliteResult<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    /// Initialize database schema
    fn init_schema(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();

        // Channels table - stores channel settings
        conn.execute(
            "CREATE TABLE IF NOT EXISTS channels (
                channel_id TEXT PRIMARY KEY,
                expected_usd REAL NOT NULL DEFAULT 0.0,
                note TEXT,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Trades table - stores trade history
        conn.execute(
            "CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                action TEXT NOT NULL,
                amount_usd REAL NOT NULL,
                amount_btc REAL NOT NULL DEFAULT 0.0,
                btc_price REAL NOT NULL,
                fee_usd REAL NOT NULL DEFAULT 0.0,
                new_expected_usd REAL NOT NULL DEFAULT 0.0,
                new_backing_sats INTEGER,
                payment_id TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Migration: Add amount_btc column to existing trades table if missing
        let _ = conn.execute(
            "ALTER TABLE trades ADD COLUMN amount_btc REAL NOT NULL DEFAULT 0.0",
            [],
        ); // Ignore error if column already exists

        // Migration: persist new_expected_usd so a trade settling after a restart can be
        // finalized from its pending row (the in-memory pending-trade map is empty on launch).
        let _ = conn.execute(
            "ALTER TABLE trades ADD COLUMN new_expected_usd REAL NOT NULL DEFAULT 0.0",
            [],
        );

        // Exact allocation signed in TRADE_V1. NULL identifies trades written by older wallets,
        // which still need the legacy price-derived recovery path.
        let _ = conn.execute("ALTER TABLE trades ADD COLUMN new_backing_sats INTEGER", []);

        // Durable desktop TRADE_V1 lifecycle. New rows are created in `sending` before LDK is
        // called; nullable/defaulted migrations keep historical and mobile-created databases
        // readable.
        let _ = conn.execute("ALTER TABLE trades ADD COLUMN trade_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE trades ADD COLUMN fee_msat INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE trades ADD COLUMN fee_status TEXT NOT NULL DEFAULT 'legacy'",
            [],
        );
        let _ = conn.execute("ALTER TABLE trades ADD COLUMN failure_code TEXT", []);
        let _ = conn.execute("ALTER TABLE trades ADD COLUMN failure_reason TEXT", []);
        let _ = conn.execute("ALTER TABLE trades ADD COLUMN expires_at INTEGER", []);
        let _ = conn.execute("ALTER TABLE trades ADD COLUMN resolved_at INTEGER", []);
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_trades_trade_id
             ON trades(trade_id) WHERE trade_id IS NOT NULL",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_trades_expiry
             ON trades(status, expires_at)",
            [],
        )?;

        // Migration: Add stable_sats column to channels table if missing
        // stable_sats tracks the BTC backing the stable portion (excludes native BTC)
        let _ = conn.execute(
            "ALTER TABLE channels ADD COLUMN stable_sats INTEGER NOT NULL DEFAULT 0",
            [],
        ); // Ignore error if column already exists

        // Migration: Add user_channel_id column (stable across splices, unlike channel_id)
        let _ = conn.execute("ALTER TABLE channels ADD COLUMN user_channel_id TEXT", []); // Ignore error if column already exists

        // Migration: Add native_sats column — sats NOT backing the stable position
        let _ = conn.execute(
            "ALTER TABLE channels ADD COLUMN native_sats INTEGER NOT NULL DEFAULT 0",
            [],
        ); // Ignore error if column already exists

        // Monotonic version for signed SYNC_V1 state. The LSP increments this before sending;
        // the wallet persists the last accepted value with the allocation it protects.
        let _ = conn.execute(
            "ALTER TABLE channels ADD COLUMN sync_version INTEGER NOT NULL DEFAULT 0",
            [],
        );

        // Migration: Add closed_at column. NULL = active, unix timestamp = soft-closed.
        // We never hard-delete channel rows from reconcile / handle_channel_closed —
        // they're marked closed so closed-channel forensics survive transient gRPC blips.
        let _ = conn.execute("ALTER TABLE channels ADD COLUMN closed_at INTEGER", []);
        // Ignore error if column already exists

        // Price history table - stores historical prices for charts
        conn.execute(
            "CREATE TABLE IF NOT EXISTS price_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                price REAL NOT NULL,
                source TEXT,
                timestamp INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Create index for faster price history queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_price_history_timestamp
             ON price_history(timestamp DESC)",
            [],
        )?;

        // Payments table - stores incoming/outgoing payment history
        conn.execute(
            "CREATE TABLE IF NOT EXISTS payments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payment_id TEXT,
                payment_type TEXT NOT NULL DEFAULT 'manual',
                direction TEXT NOT NULL,
                amount_msat INTEGER NOT NULL,
                amount_usd REAL,
                btc_price REAL,
                counterparty TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                user_channel_id TEXT,
                backing_sats_before INTEGER,
                backing_sats_after INTEGER,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Migration: Add payment_type column to existing payments table if missing
        let _ = conn.execute(
            "ALTER TABLE payments ADD COLUMN payment_type TEXT NOT NULL DEFAULT 'manual'",
            [],
        ); // Ignore error if column already exists

        // Migration: Add fee_msat column to existing payments table if missing
        let _ = conn.execute(
            "ALTER TABLE payments ADD COLUMN fee_msat INTEGER NOT NULL DEFAULT 0",
            [],
        ); // Ignore error if column already exists

        // Metadata needed to conditionally roll back an optimistic stability allocation when LDK
        // later reports PaymentFailed. NULL values identify legacy payment rows.
        let _ = conn.execute("ALTER TABLE payments ADD COLUMN user_channel_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE payments ADD COLUMN backing_sats_before INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE payments ADD COLUMN backing_sats_after INTEGER",
            [],
        );

        // Migration: Add on-chain fields to payments table
        let _ = conn.execute("ALTER TABLE payments ADD COLUMN txid TEXT", []);
        let _ = conn.execute("ALTER TABLE payments ADD COLUMN address TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE payments ADD COLUMN confirmations INTEGER NOT NULL DEFAULT 0",
            [],
        );

        // Durable reconciliation marker. Splices use it to prevent a second deduction at
        // ChannelReady; outgoing Lightning payments use it to make PaymentSuccessful replay-safe.
        let _ = conn.execute(
            "ALTER TABLE payments ADD COLUMN stable_reconciled INTEGER NOT NULL DEFAULT 0",
            [],
        );

        // Create index for faster payment queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_payments_created
             ON payments(created_at DESC)",
            [],
        )?;

        // On-chain transactions table - stores on-chain tx history
        conn.execute(
            "CREATE TABLE IF NOT EXISTS onchain_txs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                txid TEXT NOT NULL,
                direction TEXT NOT NULL,
                amount_sats INTEGER NOT NULL,
                address TEXT,
                btc_price REAL,
                status TEXT NOT NULL DEFAULT 'pending',
                confirmations INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Create index for faster on-chain tx queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_onchain_txs_created
             ON onchain_txs(created_at DESC)",
            [],
        )?;

        // Daily prices table - stores daily OHLC data for long-term charts
        conn.execute(
            "CREATE TABLE IF NOT EXISTS daily_prices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL UNIQUE,
                open REAL NOT NULL,
                high REAL NOT NULL,
                low REAL NOT NULL,
                close REAL NOT NULL,
                volume REAL,
                source TEXT
            )",
            [],
        )?;

        // Create index for faster daily price queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_daily_prices_date
             ON daily_prices(date DESC)",
            [],
        )?;

        // Settlement payments - records stable-channel settlement keysends by payment_id + kind
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settlement_payments (
                payment_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                recorded_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Migration: add user_channel_id column to settlement_payments for outcome-event keying
        let _ = conn.execute(
            "ALTER TABLE settlement_payments ADD COLUMN user_channel_id TEXT",
            [],
        ); // Ignore error if column already exists
        // Outbound stability sends optimistically move backing to equilibrium. Persist the exact
        // transition so a later asynchronous PaymentFailed can undo it without guessing or
        // overwriting a newer trade/sync allocation.
        let _ = conn.execute(
            "ALTER TABLE settlement_payments ADD COLUMN backing_sats_before INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE settlement_payments ADD COLUMN backing_sats_after INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE settlement_payments ADD COLUMN native_sats_before INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE settlement_payments ADD COLUMN expected_usd REAL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE settlement_payments ADD COLUMN last_stability_payment_before INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE settlement_payments ADD COLUMN outcome TEXT NOT NULL DEFAULT 'pending'",
            [],
        );

        // Authenticated TRADE_V1 decisions and their durable response obligations. `trade_id` is
        // intentionally not UNIQUE: a second payment reusing one must itself be persisted as a
        // rejected decision. `inbound_payment_id` is the event-level deduplication key.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS trade_decisions (
                inbound_payment_id TEXT PRIMARY KEY,
                trade_id TEXT,
                channel_id TEXT NOT NULL,
                user_channel_id TEXT NOT NULL,
                remote_user_channel_id TEXT,
                counterparty TEXT NOT NULL,
                outcome TEXT NOT NULL,
                reason_code TEXT,
                explanation TEXT,
                expected_usd REAL,
                backing_sats INTEGER,
                sync_version INTEGER,
                response_envelope TEXT NOT NULL,
                response_amount_msat INTEGER NOT NULL,
                response_payment_id TEXT,
                response_status TEXT NOT NULL DEFAULT 'pending',
                response_attempts INTEGER NOT NULL DEFAULT 0,
                next_response_attempt_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                resolved_at INTEGER
            )",
            [],
        )?;
        let _ = conn.execute(
            "ALTER TABLE trade_decisions ADD COLUMN remote_user_channel_id TEXT",
            [],
        );
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_trade_decisions_trade_id
             ON trade_decisions(trade_id) WHERE trade_id IS NOT NULL",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_trade_decisions_response_due
             ON trade_decisions(response_status, next_response_attempt_at)",
            [],
        )?;

        // Forwarded-payment dedup: tracks fingerprints of forwards already audited (live or backfilled)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS forwarded_seen (fingerprint TEXT PRIMARY KEY)",
            [],
        )?;

        // Append-only operator history. There is intentionally no pruning path.
        ledger::init_schema(&conn)?;

        Ok(())
    }

    // =========================================================================
    // Authoritative channel ledger
    // =========================================================================

    pub fn append_ledger_event(&self, draft: &LedgerEventDraft) -> SqliteResult<AppendOutcome> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let outcome = ledger::append_on_connection(&tx, draft)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Append a reconstructed snapshot only when this entity differs from the last snapshot seen
    /// for the same scope. Unlike a permanent dedup key, overwriting the stored fingerprint keeps
    /// A -> B -> A transitions visible while suppressing identical reconnect snapshots.
    pub fn append_reconstructed_event_if_changed(
        &self,
        scope: &str,
        identity: &str,
        fingerprint: &str,
        draft: &LedgerEventDraft,
    ) -> SqliteResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT fingerprint FROM ledger_reconstruction_state
                 WHERE scope = ?1 AND identity = ?2",
                params![scope, identity],
                |row| row.get(0),
            )
            .optional()?;
        if previous.as_deref() == Some(fingerprint) {
            tx.commit()?;
            return Ok(false);
        }

        let outcome = ledger::append_on_connection(&tx, draft)?;
        tx.execute(
            "INSERT INTO ledger_reconstruction_state (scope, identity, fingerprint)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(scope, identity) DO UPDATE SET
                fingerprint = excluded.fingerprint,
                updated_at_ms = unixepoch('subsec') * 1000",
            params![scope, identity, fingerprint],
        )?;
        tx.commit()?;
        if outcome.inserted {
            crate::audit::mirror_committed_ledger_event(draft, outcome.event_id);
        }
        Ok(outcome.inserted)
    }

    pub fn list_ledger_events(&self, query: &LedgerQuery) -> SqliteResult<LedgerPage> {
        let conn = self.conn.lock().unwrap();
        ledger::list_on_connection(&conn, query)
    }

    /// Import valid historical JSONL once. Malformed source lines remain in the
    /// raw file and are reported as skipped, but are not invented into events.
    pub fn import_legacy_audit_log(&self, path: &Path) -> SqliteResult<LegacyImportReport> {
        let conn = self.conn.lock().unwrap();
        ledger::import_legacy_jsonl(&conn, path)
    }

    // =========================================================================
    // Channel Operations
    // =========================================================================

    /// Save or update channel settings.
    ///
    /// Calling this is an active assertion that the channel is live, so any
    /// prior `closed_at` is cleared on UPDATE. This way a channel that was
    /// marked closed in error (e.g. a transient gRPC blip during reconcile)
    /// re-activates the next time we save it.
    pub fn save_channel(
        &self,
        channel_id: &str,
        user_channel_id: &str,
        expected_usd: f64,
        backing_sats: u64,
        native_sats: u64,
        note: Option<&str>,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result = (|| {
            let before: Option<(f64, i64, i64, String, Option<i64>, Option<String>)> = conn
                .query_row(
                    "SELECT expected_usd, stable_sats, native_sats, channel_id, closed_at, note
                     FROM channels WHERE user_channel_id = ?1",
                    params![user_channel_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .optional()?;
            let accounting_changed = before.as_ref().map_or(true, |previous| {
                previous.0.to_bits() != expected_usd.to_bits()
                    || previous.1 != backing_sats as i64
                    || previous.2 != native_sats as i64
                    || previous.3 != channel_id
                    || previous.4.is_some()
            });
            let row_changed = accounting_changed
                || before
                    .as_ref()
                    .map_or(true, |previous| previous.5.as_deref() != note);
            if !row_changed {
                return Ok(());
            }
            // Try to update by user_channel_id first (handles channel_id changes from splices)
            let updated = conn.execute(
            "UPDATE channels SET channel_id = ?1, expected_usd = ?2, stable_sats = ?3,
                                 note = ?4, user_channel_id = ?5, native_sats = ?6,
                                 closed_at = NULL,
                                 updated_at = strftime('%s', 'now')
             WHERE user_channel_id = ?5",
            params![
                channel_id,
                expected_usd,
                backing_sats as i64,
                note,
                user_channel_id,
                native_sats as i64
            ],
            )?;
            if updated == 0 {
                // No existing row — insert new
                conn.execute(
                "INSERT INTO channels (channel_id, user_channel_id, expected_usd, stable_sats, native_sats, note)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(channel_id) DO UPDATE SET
                    user_channel_id = ?2,
                    expected_usd = ?3,
                    stable_sats = ?4,
                    native_sats = ?5,
                    note = ?6,
                    closed_at = NULL,
                    updated_at = strftime('%s', 'now')",
                params![channel_id, user_channel_id, expected_usd, backing_sats as i64, native_sats as i64, note],
                )?;
            }
            if !accounting_changed {
                return Ok(());
            }
            let live_receiver_sats = backing_sats.saturating_add(native_sats);
            let draft = LedgerEventDraft {
                event_type: "CHANNEL_ACCOUNTING_STATE_COMMITTED".to_owned(),
                category: "channel".to_owned(),
                severity: "info".to_owned(),
                status: "completed".to_owned(),
                source: "database".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: None,
                before: before
                    .as_ref()
                    .map(|(expected, backing, native, _, _, _)| AccountingSnapshot {
                    expected_usd: Some(*expected),
                    backing_sats: u64::try_from(*backing).ok(),
                    native_sats: u64::try_from(*native).ok(),
                    live_receiver_sats: u64::try_from(backing.saturating_add(*native)).ok(),
                    ..Default::default()
                }),
                after: Some(AccountingSnapshot {
                    expected_usd: Some(expected_usd),
                    backing_sats: Some(backing_sats),
                    native_sats: Some(native_sats),
                    live_receiver_sats: Some(live_receiver_sats),
                    ..Default::default()
                }),
                detail: serde_json::json!({
                    "user_channel_id": user_channel_id,
                    "channel_id": channel_id,
                    "previous_channel_id": before.as_ref().map(|row| row.3.as_str()),
                    "expected_usd": expected_usd,
                    "backing_sats": backing_sats,
                    "native_sats": native_sats,
                    "live_receiver_sats": live_receiver_sats,
                }),
                refs: {
                    let mut refs = vec![
                        LedgerRef::new("user_channel_id", user_channel_id),
                        LedgerRef::new("channel_id", channel_id),
                    ];
                    if let Some((_, _, _, previous_channel_id, _, _)) = &before {
                        if previous_channel_id != channel_id {
                            refs.push(LedgerRef::new("channel_id", previous_channel_id));
                        }
                    }
                    refs
                },
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }
            Ok(())
        })();
        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Save channel settings without touching `stable_sats`.
    ///
    /// `stable_sats` is owned by the transactional payment path
    /// (`record_payment_and_maybe_update_backing`) and intentional absolute
    /// writers (trades, channel creation, settings edits). State saves that
    /// only carry a stale in-memory snapshot must use this so they can't
    /// silently overwrite a backing delta committed concurrently.
    ///
    /// UPDATE-only: returns Ok(true) if a row was updated, Ok(false) if no
    /// row exists for `user_channel_id` (caller may fall back to the full
    /// `save_channel` insert path). Like `save_channel`, this asserts the
    /// channel is live, so `closed_at` is cleared.
    pub fn save_channel_preserving_backing(
        &self,
        channel_id: &str,
        user_channel_id: &str,
        expected_usd: f64,
        native_sats: u64,
        note: Option<&str>,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result: SqliteResult<bool> = (|| {
            let before: Option<(String, f64, i64, i64, Option<i64>, Option<String>)> = conn
                .query_row(
                    "SELECT channel_id, expected_usd, stable_sats, native_sats, closed_at, note
                     FROM channels WHERE user_channel_id = ?1",
                    params![user_channel_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .optional()?;
            let Some((
                previous_channel_id,
                previous_expected,
                backing,
                previous_native,
                previous_closed_at,
                previous_note,
            )) = before
            else {
                return Ok(false);
            };
            let accounting_changed = previous_channel_id != channel_id
                || previous_expected.to_bits() != expected_usd.to_bits()
                || previous_native != native_sats as i64
                || previous_closed_at.is_some();
            if !accounting_changed && previous_note.as_deref() == note {
                return Ok(true);
            }
            let updated = conn.execute(
                "UPDATE channels SET channel_id = ?1, expected_usd = ?2,
                                     note = ?3, user_channel_id = ?4, native_sats = ?5,
                                     closed_at = NULL,
                                     updated_at = strftime('%s', 'now')
                 WHERE user_channel_id = ?4",
                params![
                    channel_id,
                    expected_usd,
                    note,
                    user_channel_id,
                    native_sats as i64
                ],
            )?;
            if !accounting_changed {
                return Ok(updated > 0);
            }
            let draft = LedgerEventDraft {
                event_type: "CHANNEL_ACCOUNTING_STATE_COMMITTED".to_owned(),
                category: "channel".to_owned(),
                severity: "info".to_owned(),
                status: "completed".to_owned(),
                source: "database".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: None,
                before: Some(AccountingSnapshot {
                    expected_usd: Some(previous_expected),
                    backing_sats: u64::try_from(backing).ok(),
                    native_sats: u64::try_from(previous_native).ok(),
                    live_receiver_sats: u64::try_from(backing.saturating_add(previous_native)).ok(),
                    ..Default::default()
                }),
                after: Some(AccountingSnapshot {
                    expected_usd: Some(expected_usd),
                    backing_sats: u64::try_from(backing).ok(),
                    native_sats: Some(native_sats),
                    live_receiver_sats: u64::try_from(backing)
                        .ok()
                        .map(|value| value.saturating_add(native_sats)),
                    ..Default::default()
                }),
                detail: serde_json::json!({
                    "user_channel_id": user_channel_id,
                    "channel_id": channel_id,
                    "previous_channel_id": previous_channel_id,
                    "expected_usd": expected_usd,
                    "backing_sats": backing,
                    "native_sats": native_sats,
                }),
                refs: {
                    let mut refs = vec![
                        LedgerRef::new("user_channel_id", user_channel_id),
                        LedgerRef::new("channel_id", channel_id),
                    ];
                    if previous_channel_id != channel_id {
                        refs.push(LedgerRef::new("channel_id", previous_channel_id));
                    }
                    refs
                },
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }
            Ok(updated > 0)
        })();
        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Atomically reserve the next outbound SYNC_V1 version for a channel.
    pub fn next_sync_version(&self, user_channel_id: &str) -> SqliteResult<u64> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let updated = tx.execute(
            "UPDATE channels
             SET sync_version = sync_version + 1
             WHERE user_channel_id = ?1 AND sync_version < 9223372036854775807",
            params![user_channel_id],
        )?;
        if updated == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        let version: i64 = tx.query_row(
            "SELECT sync_version FROM channels WHERE user_channel_id = ?1",
            params![user_channel_id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(version as u64)
    }

    /// Return the next version without reserving it. Trade acceptance signs a response using this
    /// candidate, then atomically compares/increments the version while storing the signed
    /// response and allocation.
    pub fn candidate_sync_version(&self, user_channel_id: &str) -> SqliteResult<u64> {
        let conn = self.conn.lock().unwrap();
        let version: i64 = conn.query_row(
            "SELECT sync_version FROM channels WHERE user_channel_id = ?1",
            params![user_channel_id],
            |row| row.get(0),
        )?;
        version
            .checked_add(1)
            .filter(|value| *value > 0)
            .map(|value| value as u64)
            .ok_or(rusqlite::Error::IntegralValueOutOfRange(0, version))
    }

    /// True when another inbound payment has already claimed this caller-generated trade id.
    pub fn trade_id_seen_on_other_payment(
        &self,
        trade_id: &str,
        inbound_payment_id: &str,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM trade_decisions
                WHERE trade_id = ?1 AND inbound_payment_id <> ?2
             )",
            params![trade_id, inbound_payment_id],
            |row| row.get(0),
        )
    }

    pub fn trade_decision_exists(&self, inbound_payment_id: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM trade_decisions WHERE inbound_payment_id = ?1)",
            params![inbound_payment_id],
            |row| row.get(0),
        )
    }

    /// Requeue the original authoritative response when another payment reuses its trade id.
    /// The new payment still receives `duplicate_trade`; sending the original decision first
    /// prevents that secondary rejection from becoming the wallet's only observed outcome.
    pub fn requeue_original_trade_response(
        &self,
        trade_id: &str,
        inbound_payment_id: &str,
        now: i64,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE trade_decisions
             SET response_status = 'pending', response_payment_id = NULL,
                 response_attempts = 0, next_response_attempt_at = ?3,
                 resolved_at = NULL
             WHERE rowid = (
                SELECT rowid FROM trade_decisions
                WHERE trade_id = ?1 AND inbound_payment_id <> ?2
                ORDER BY rowid ASC LIMIT 1
             )",
            params![trade_id, inbound_payment_id, now],
        )?;
        Ok(updated == 1)
    }

    /// Give an explicitly replayed inbound payment's abandoned response a fresh delivery budget.
    pub fn requeue_abandoned_trade_response(
        &self,
        inbound_payment_id: &str,
        now: i64,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE trade_decisions
             SET response_status = 'pending', response_payment_id = NULL,
                 response_attempts = 0, next_response_attempt_at = ?2,
                 resolved_at = NULL
             WHERE inbound_payment_id = ?1 AND response_status = 'abandoned'",
            params![inbound_payment_id, now],
        )?;
        Ok(updated == 1)
    }

    /// Persist an authenticated rejection and its nominal control response before sending it.
    #[allow(clippy::too_many_arguments)]
    pub fn persist_trade_rejection(
        &self,
        inbound_payment_id: &str,
        trade_id: Option<&str>,
        channel_id: &str,
        user_channel_id: &str,
        remote_user_channel_id: Option<&str>,
        counterparty: &str,
        reason_code: &str,
        explanation: &str,
        response_envelope: &str,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO trade_decisions (
                inbound_payment_id, trade_id, channel_id, user_channel_id,
                remote_user_channel_id, counterparty,
                outcome, reason_code, explanation, response_envelope,
                response_amount_msat
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'rejected', ?7, ?8, ?9, 1)",
            params![
                inbound_payment_id,
                trade_id,
                channel_id,
                user_channel_id,
                remote_user_channel_id,
                counterparty,
                reason_code,
                explanation,
                response_envelope,
            ],
        )?;
        Ok(inserted == 1)
    }

    /// Atomically apply an accepted allocation, advance its SYNC version, and persist the signed
    /// response/decision. A stale candidate version or duplicate payment leaves everything intact.
    #[allow(clippy::too_many_arguments)]
    pub fn persist_trade_acceptance(
        &self,
        inbound_payment_id: &str,
        trade_id: Option<&str>,
        channel_id: &str,
        user_channel_id: &str,
        remote_user_channel_id: Option<&str>,
        counterparty: &str,
        expected_usd: f64,
        backing_sats: u64,
        native_sats: u64,
        quote_price: Option<f64>,
        lsp_price: f64,
        quote_deviation_percent: Option<f64>,
        sync_version: u64,
        response_envelope: &str,
    ) -> SqliteResult<bool> {
        if !expected_usd.is_finite()
            || expected_usd < 0.0
            || backing_sats > i64::MAX as u64
            || native_sats > i64::MAX as u64
            || !lsp_price.is_finite()
            || lsp_price <= 0.0
            || quote_price.is_some_and(|price| !price.is_finite() || price <= 0.0)
            || quote_deviation_percent
                .is_some_and(|deviation| !deviation.is_finite() || deviation < 0.0)
            || sync_version == 0
            || sync_version > i64::MAX as u64
        {
            return Ok(false);
        }

        let previous_version = sync_version as i64 - 1;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut ledger_mirror = None;
        let result = (|| {
            let duplicate: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM trade_decisions WHERE inbound_payment_id = ?1)",
                params![inbound_payment_id],
                |row| row.get(0),
            )?;
            if duplicate {
                return Ok(false);
            }
            let before: Option<(String, f64, i64, i64)> = tx
                .query_row(
                    "SELECT channel_id, expected_usd, stable_sats, native_sats
                     FROM channels WHERE user_channel_id = ?1",
                    params![user_channel_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let updated = tx.execute(
                "UPDATE channels
                 SET channel_id = ?1, expected_usd = ?2, stable_sats = ?3, native_sats = ?4,
                     sync_version = ?5, updated_at = strftime('%s', 'now'), closed_at = NULL
                 WHERE user_channel_id = ?6 AND sync_version = ?7",
                params![
                    channel_id,
                    expected_usd,
                    backing_sats,
                    native_sats,
                    sync_version,
                    user_channel_id,
                    previous_version,
                ],
            )?;
            if updated != 1 {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO trade_decisions (
                    inbound_payment_id, trade_id, channel_id, user_channel_id,
                    remote_user_channel_id, counterparty,
                    outcome, expected_usd, backing_sats, sync_version, response_envelope,
                    response_amount_msat
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'accepted', ?7, ?8, ?9, ?10, 1)",
                params![
                    inbound_payment_id,
                    trade_id,
                    channel_id,
                    user_channel_id,
                    remote_user_channel_id,
                    counterparty,
                    expected_usd,
                    backing_sats,
                    sync_version,
                    response_envelope,
                ],
            )?;
            let detail = serde_json::json!({
                "trade_id": trade_id,
                "trade_payment_id": inbound_payment_id,
                "channel_id": channel_id,
                "previous_channel_id": before.as_ref().map(|row| row.0.as_str()),
                "user_channel_id": user_channel_id,
                "remote_user_channel_id": remote_user_channel_id,
                "counterparty_node_id": counterparty,
                "new_expected_usd": expected_usd,
                "backing_sats": backing_sats,
                "native_sats": native_sats,
                "sync_version": sync_version,
                "quote_price": quote_price,
                "lsp_price": lsp_price,
                "quote_deviation_percent": quote_deviation_percent,
            });
            let mut refs = vec![
                LedgerRef::new("payment_id", inbound_payment_id),
                LedgerRef::new("channel_id", channel_id),
                LedgerRef::new("user_channel_id", user_channel_id),
                LedgerRef::new("node_id", counterparty),
            ];
            if let Some(remote_user_channel_id) = remote_user_channel_id {
                refs.push(LedgerRef::new("user_channel_id", remote_user_channel_id));
            }
            if let Some(trade_id) = trade_id {
                refs.push(LedgerRef::new("trade_id", trade_id));
            }
            if let Some((previous_channel_id, ..)) = before.as_ref() {
                if previous_channel_id != channel_id {
                    refs.push(LedgerRef::new("channel_id", previous_channel_id));
                }
            }
            let draft = LedgerEventDraft {
                event_type: "TRADE_APPLIED".to_owned(),
                category: "trade".to_owned(),
                severity: "info".to_owned(),
                status: "completed".to_owned(),
                source: "lsp_trade".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: Some(format!("lsp:trade-applied:{inbound_payment_id}")),
                before: before.as_ref().map(
                    |(_, previous_expected, previous_backing, previous_native)| {
                        AccountingSnapshot {
                            expected_usd: Some(*previous_expected),
                            backing_sats: u64::try_from(*previous_backing).ok(),
                            native_sats: u64::try_from(*previous_native).ok(),
                            live_receiver_sats: u64::try_from(
                                previous_backing.saturating_add(*previous_native),
                            )
                            .ok(),
                            ..Default::default()
                        }
                    },
                ),
                after: Some(AccountingSnapshot {
                    expected_usd: Some(expected_usd),
                    backing_sats: Some(backing_sats),
                    native_sats: Some(native_sats),
                    live_receiver_sats: Some(backing_sats.saturating_add(native_sats)),
                    ..Default::default()
                }),
                detail,
                refs,
            };
            let outcome = ledger::append_on_connection(&tx, &draft)?;
            if outcome.inserted {
                ledger_mirror = Some((draft, outcome.event_id));
            }
            Ok(true)
        })();
        match result {
            Ok(value) => {
                tx.commit()?;
                if let Some((draft, event_id)) = ledger_mirror {
                    crate::audit::mirror_committed_ledger_event(&draft, event_id);
                }
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }

    pub fn due_trade_responses(
        &self,
        now: i64,
        max_attempts: u32,
        limit: usize,
    ) -> SqliteResult<Vec<PendingTradeResponse>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT inbound_payment_id, counterparty, response_envelope,
                    response_amount_msat, response_attempts
             FROM trade_decisions
             WHERE response_status = 'pending' AND next_response_attempt_at <= ?1
                   AND response_attempts < ?2
             ORDER BY next_response_attempt_at ASC, created_at ASC, rowid ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![now, max_attempts, limit as i64], |row| {
            let amount: i64 = row.get(3)?;
            let attempts: i64 = row.get(4)?;
            Ok(PendingTradeResponse {
                inbound_payment_id: row.get(0)?,
                counterparty: row.get(1)?,
                response_envelope: row.get(2)?,
                response_amount_msat: amount.max(0) as u64,
                attempts: attempts.max(0) as u32,
            })
        })?;
        rows.collect()
    }

    /// Move exhausted response obligations to a durable terminal state and append their
    /// dead-letter ledger events in the same transaction. A replay can explicitly requeue one.
    pub fn abandon_exhausted_trade_responses(
        &self,
        max_attempts: u32,
        now: i64,
        limit: usize,
    ) -> SqliteResult<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exhausted = {
            let mut stmt = tx.prepare(
                "SELECT inbound_payment_id, trade_id, channel_id, user_channel_id,
                        remote_user_channel_id, counterparty, outcome, response_attempts
                 FROM trade_decisions
                 WHERE response_status = 'pending' AND response_attempts >= ?1
                 ORDER BY next_response_attempt_at ASC, created_at ASC, rowid ASC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![max_attempts, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?;
            rows.collect::<SqliteResult<Vec<_>>>()?
        };

        let mut abandoned = 0;
        let mut ledger_mirrors = Vec::new();
        for (
            inbound_payment_id,
            trade_id,
            channel_id,
            user_channel_id,
            remote_user_channel_id,
            counterparty,
            outcome,
            response_attempts,
        ) in exhausted
        {
            let updated = tx.execute(
                "UPDATE trade_decisions
                 SET response_status = 'abandoned', resolved_at = ?2
                 WHERE inbound_payment_id = ?1 AND response_status = 'pending'
                       AND response_attempts >= ?3",
                params![inbound_payment_id, now, max_attempts],
            )?;
            if updated != 1 {
                continue;
            }

            let detail = serde_json::json!({
                "trade_id": trade_id,
                "trade_payment_id": inbound_payment_id,
                "channel_id": channel_id,
                "user_channel_id": user_channel_id,
                "remote_user_channel_id": remote_user_channel_id,
                "counterparty_node_id": counterparty,
                "outcome": outcome,
                "response_attempts": response_attempts,
                "max_response_attempts": max_attempts,
                "response_status": "abandoned",
            });
            let mut refs = vec![
                LedgerRef::new("payment_id", &inbound_payment_id),
                LedgerRef::new("channel_id", &channel_id),
                LedgerRef::new("user_channel_id", &user_channel_id),
                LedgerRef::new("node_id", &counterparty),
            ];
            if let Some(remote_user_channel_id) = remote_user_channel_id.as_deref() {
                refs.push(LedgerRef::new("user_channel_id", remote_user_channel_id));
            }
            if let Some(trade_id) = trade_id.as_deref() {
                refs.push(LedgerRef::new("trade_id", trade_id));
            }
            let draft = LedgerEventDraft {
                event_type: "TRADE_RESPONSE_ABANDONED".to_owned(),
                category: "trade".to_owned(),
                severity: "warning".to_owned(),
                status: "failed".to_owned(),
                source: "lsp_trade".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: now.saturating_mul(1000),
                dedup_key: Some(format!(
                    "lsp:trade-response-abandoned:{inbound_payment_id}"
                )),
                before: None,
                after: None,
                detail,
                refs,
            };
            let ledger_outcome = ledger::append_on_connection(&tx, &draft)?;
            if ledger_outcome.inserted {
                ledger_mirrors.push((draft, ledger_outcome.event_id));
            }
            abandoned += 1;
        }

        tx.commit()?;
        for (draft, event_id) in ledger_mirrors {
            crate::audit::mirror_committed_ledger_event(&draft, event_id);
        }
        Ok(abandoned)
    }

    pub fn in_flight_trade_response_payment_ids(&self) -> SqliteResult<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT response_payment_id FROM trade_decisions
             WHERE response_status = 'in_flight' AND response_payment_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    fn trade_response_delay_secs(attempts_after_failure: u32, response_key: &str) -> i64 {
        let exponent = attempts_after_failure.saturating_sub(1).min(10);
        let base = (5_i64.saturating_mul(1_i64 << exponent)).min(60 * 60);
        if attempts_after_failure <= 1 || base >= 60 * 60 {
            return base;
        }
        let hash = response_key.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
            hash.wrapping_mul(0x100000001b3) ^ u64::from(byte)
        });
        let jitter = (hash % (base as u64 / 5 + 1)) as i64;
        base.saturating_add(jitter).min(60 * 60)
    }

    pub fn mark_trade_response_send_failed(
        &self,
        inbound_payment_id: &str,
        now: i64,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        let attempts: i64 = conn.query_row(
            "SELECT response_attempts FROM trade_decisions
             WHERE inbound_payment_id = ?1 AND response_status = 'pending'",
            params![inbound_payment_id],
            |row| row.get(0),
        )?;
        let next_attempt = attempts.saturating_add(1).max(1) as u32;
        let delay = Self::trade_response_delay_secs(next_attempt, inbound_payment_id);
        let updated = conn.execute(
            "UPDATE trade_decisions
             SET response_status = 'pending', response_attempts = ?2,
                 next_response_attempt_at = ?3
             WHERE inbound_payment_id = ?1 AND response_status = 'pending'",
            params![inbound_payment_id, next_attempt, now.saturating_add(delay)],
        )?;
        if updated == 1 {
            Ok(())
        } else {
            Err(rusqlite::Error::QueryReturnedNoRows)
        }
    }

    /// Store the generated outbound id immediately after LDK accepts the control send. A process
    /// crash in the preceding gap can produce a duplicate nominal response after restart.
    pub fn mark_trade_response_in_flight(
        &self,
        inbound_payment_id: &str,
        response_payment_id: &str,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE trade_decisions
             SET response_status = 'in_flight', response_payment_id = ?2,
                 response_attempts = response_attempts + 1
             WHERE inbound_payment_id = ?1 AND response_status = 'pending'",
            params![inbound_payment_id, response_payment_id],
        )?;
        if updated == 1 {
            Ok(())
        } else {
            Err(rusqlite::Error::QueryReturnedNoRows)
        }
    }

    pub fn mark_trade_response_succeeded(&self, response_payment_id: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE trade_decisions
             SET response_status = 'succeeded', resolved_at = strftime('%s', 'now')
             WHERE response_payment_id = ?1 AND response_status <> 'succeeded'",
            params![response_payment_id],
        )?;
        Ok(updated > 0)
    }

    pub fn mark_trade_response_payment_failed(
        &self,
        response_payment_id: &str,
        now: i64,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let attempts = conn
            .query_row(
                "SELECT response_attempts FROM trade_decisions
                 WHERE response_payment_id = ?1 AND response_status = 'in_flight'",
                params![response_payment_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(attempts) = attempts else {
            return Ok(false);
        };
        let delay = Self::trade_response_delay_secs(attempts.max(1) as u32, response_payment_id);
        let updated = conn.execute(
            "UPDATE trade_decisions
             SET response_status = 'pending', response_payment_id = NULL,
                 next_response_attempt_at = ?2
             WHERE response_payment_id = ?1 AND response_status = 'in_flight'",
            params![response_payment_id, now.saturating_add(delay)],
        )?;
        Ok(updated > 0)
    }

    /// Apply a signed inbound SYNC_V1 allocation only when its version is newer.
    /// The version and allocation share one SQLite statement, so a crash cannot
    /// persist one without the other. Returns false for stale/replayed versions.
    pub fn apply_sync_if_newer(
        &self,
        user_channel_id: &str,
        sync_version: u64,
        expected_usd: f64,
        backing_sats: u64,
        native_sats: u64,
    ) -> SqliteResult<bool> {
        self.apply_sync_if_newer_and_complete_trade(
            user_channel_id,
            sync_version,
            expected_usd,
            backing_sats,
            native_sats,
            None,
        )
    }

    /// Apply a signed sync and, when it acknowledges a wallet-authored trade, complete that trade
    /// in the same transaction. This closes the crash window between accepting the allocation and
    /// marking its fee payment as a trade rather than an ordinary outgoing payment.
    pub fn apply_sync_if_newer_and_complete_trade(
        &self,
        user_channel_id: &str,
        sync_version: u64,
        expected_usd: f64,
        backing_sats: u64,
        native_sats: u64,
        trade_db_id: Option<i64>,
    ) -> SqliteResult<bool> {
        self.apply_correlated_sync_if_newer_and_complete_trade(
            user_channel_id,
            sync_version,
            expected_usd,
            backing_sats,
            native_sats,
            trade_db_id,
            None,
        )
    }

    /// Correlated variant that can durably repair a missing outbound payment id after a crash.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_correlated_sync_if_newer_and_complete_trade(
        &self,
        user_channel_id: &str,
        sync_version: u64,
        expected_usd: f64,
        backing_sats: u64,
        native_sats: u64,
        trade_db_id: Option<i64>,
        trade_payment_id: Option<&str>,
    ) -> SqliteResult<bool> {
        if sync_version == 0
            || sync_version > i64::MAX as u64
            || !expected_usd.is_finite()
            || expected_usd < 0.0
            || backing_sats > i64::MAX as u64
            || native_sats > i64::MAX as u64
        {
            return Ok(false);
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(trade_db_id) = trade_db_id {
            let stored_intent = tx
                .query_row(
                    "SELECT payment_id, new_expected_usd, new_backing_sats, trade_id
                     FROM trades WHERE id = ?1",
                    params![trade_db_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, f64>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
                .optional()?;
            let Some((stored_payment_id, stored_expected_usd, stored_backing_sats, trade_id)) =
                stored_intent
            else {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            };
            if let (Some(stored), Some(received)) =
                (stored_payment_id.as_deref(), trade_payment_id)
            {
                if stored != received {
                    return Ok(false);
                }
            }
            if trade_id.is_some()
                && ((stored_expected_usd - expected_usd).abs() > 0.000000001
                    || stored_backing_sats != Some(backing_sats as i64))
            {
                return Ok(false);
            }
        }
        let before: Option<(f64, i64, i64)> = tx
            .query_row(
                "SELECT expected_usd, stable_sats, native_sats FROM channels WHERE user_channel_id = ?1",
                params![user_channel_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let updated = tx.execute(
            "UPDATE channels
             SET expected_usd = ?1, stable_sats = ?2, native_sats = ?3,
                 sync_version = ?4, updated_at = strftime('%s', 'now')
             WHERE user_channel_id = ?5 AND sync_version < ?4",
            params![
                expected_usd,
                backing_sats as i64,
                native_sats as i64,
                sync_version as i64,
                user_channel_id,
            ],
        )?;
        if updated == 0 {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM channels WHERE user_channel_id = ?1
                 )",
                params![user_channel_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            tx.commit()?;
            return Ok(false);
        }
        if let Some(trade_db_id) = trade_db_id {
            let completed = tx.execute(
                "UPDATE trades
                 SET status = 'completed', fee_status = 'paid',
                     payment_id = COALESCE(payment_id, ?2),
                     failure_code = NULL, failure_reason = NULL,
                     resolved_at = strftime('%s', 'now')
                 WHERE id = ?1
                   AND (?2 IS NULL OR payment_id IS NULL OR payment_id = ?2)",
                params![trade_db_id, trade_payment_id],
            )?;
            if completed != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
        }
        let draft = LedgerEventDraft {
            event_type: "SYNC_V1_APPLIED".to_owned(),
            category: "stability".to_owned(),
            severity: "info".to_owned(),
            status: "completed".to_owned(),
            source: "signed_sync".to_owned(),
            completeness: LedgerCompleteness::Observed,
            occurred_at_ms: Utc::now().timestamp_millis(),
            dedup_key: Some(format!(
                "signed-sync:sync-v1:{user_channel_id}:{sync_version}"
            )),
            before: before.map(|(expected, backing, native)| AccountingSnapshot {
                expected_usd: Some(expected),
                backing_sats: u64::try_from(backing).ok(),
                native_sats: u64::try_from(native).ok(),
                live_receiver_sats: u64::try_from(backing.saturating_add(native)).ok(),
                ..Default::default()
            }),
            after: Some(AccountingSnapshot {
                expected_usd: Some(expected_usd),
                backing_sats: Some(backing_sats),
                native_sats: Some(native_sats),
                live_receiver_sats: Some(backing_sats.saturating_add(native_sats)),
                ..Default::default()
            }),
            detail: serde_json::json!({
                "user_channel_id": user_channel_id,
                "sync_version": sync_version,
                "trade_db_id": trade_db_id,
                "trade_payment_id": trade_payment_id,
                "new_expected_usd": expected_usd,
                "new_backing_sats": backing_sats,
                "new_native_sats": native_sats,
                "live_receiver_sats": backing_sats.saturating_add(native_sats),
            }),
            refs: vec![LedgerRef::new("user_channel_id", user_channel_id)],
        };
        let outcome = ledger::append_on_connection(&tx, &draft)?;
        tx.commit()?;
        if outcome.inserted {
            crate::audit::mirror_committed_ledger_event(&draft, outcome.event_id);
        }
        Ok(true)
    }

    pub fn get_sync_version(&self, user_channel_id: &str) -> SqliteResult<Option<u64>> {
        let conn = self.conn.lock().unwrap();
        let version = conn
            .query_row(
                "SELECT sync_version FROM channels WHERE user_channel_id = ?1",
                params![user_channel_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(version.map(|value| value.max(0) as u64))
    }

    /// Hard-delete a channel row. Reserved for explicit admin purge; reconcile
    /// and channel-close paths should call `mark_channel_closed` instead so the
    /// row survives for forensics.
    pub fn delete_channel(&self, user_channel_id: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM channels WHERE user_channel_id = ?1",
            params![user_channel_id],
        )?;
        Ok(())
    }

    /// Soft-close a channel row: set `closed_at` to now if not already set.
    /// Idempotent — preserves the original close time on subsequent calls so
    /// the audit trail stays meaningful.
    pub fn mark_channel_closed(&self, user_channel_id: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result: SqliteResult<()> = (|| {
            let before: Option<(String, f64, i64, i64)> = conn
                .query_row(
                    "SELECT channel_id, expected_usd, stable_sats, native_sats
                     FROM channels WHERE user_channel_id = ?1 AND closed_at IS NULL",
                    params![user_channel_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let updated = conn.execute(
                "UPDATE channels
                 SET closed_at = strftime('%s', 'now'),
                     updated_at = strftime('%s', 'now')
                 WHERE user_channel_id = ?1 AND closed_at IS NULL",
                params![user_channel_id],
            )?;
            if updated == 0 {
                return Ok(());
            }
            let Some((channel_id, expected_usd, backing, native)) = before else {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            };
            let snapshot = AccountingSnapshot {
                expected_usd: Some(expected_usd),
                backing_sats: u64::try_from(backing).ok(),
                native_sats: u64::try_from(native).ok(),
                live_receiver_sats: u64::try_from(backing.saturating_add(native)).ok(),
                ..Default::default()
            };
            let draft = LedgerEventDraft {
                event_type: "CHANNEL_CLOSED_COMMITTED".to_owned(),
                category: "channel".to_owned(),
                severity: "info".to_owned(),
                status: "completed".to_owned(),
                source: "database".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                // The UPDATE guard above is the idempotency boundary. A later reopen followed by
                // another close is a distinct transition and must remain visible.
                dedup_key: None,
                before: Some(snapshot.clone()),
                after: Some(snapshot),
                detail: serde_json::json!({
                    "user_channel_id": user_channel_id,
                    "channel_id": channel_id,
                }),
                refs: vec![
                    LedgerRef::new("user_channel_id", user_channel_id),
                    LedgerRef::new("channel_id", channel_id),
                ],
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }
            Ok(())
        })();
        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Resolve user_channel_id from a (possibly closed) channel_id.
    pub fn get_user_channel_id_by_channel_id(&self, channel_id: &str) -> SqliteResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT user_channel_id FROM channels WHERE channel_id = ?1")?;
        let mut rows = stmt.query(params![channel_id])?;
        if let Some(row) = rows.next()? {
            Ok(row.get::<_, Option<String>>(0)?)
        } else {
            Ok(None)
        }
    }

    /// Load channel settings by user_channel_id (stable across splices)
    pub fn load_channel(&self, user_channel_id: &str) -> SqliteResult<Option<ChannelRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT channel_id, expected_usd, note, stable_sats, user_channel_id, native_sats
             FROM channels WHERE user_channel_id = ?1",
        )?;

        let mut rows = stmt.query(params![user_channel_id])?;

        if let Some(row) = rows.next()? {
            let backing_sats: i64 = row.get(3).unwrap_or(0);
            let native_sats: i64 = row.get(5).unwrap_or(0);
            Ok(Some(ChannelRecord {
                channel_id: row.get(0)?,
                user_channel_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                expected_usd: row.get(1)?,
                note: row.get(2)?,
                backing_sats: backing_sats as u64,
                native_sats: native_sats as u64,
            }))
        } else {
            Ok(None)
        }
    }

    /// Load all *active* channel records (closed_at IS NULL). This is the
    /// load called by reconcile and the stability tick — closed channels are
    /// excluded so we never act on them.
    pub fn load_all_channels(&self) -> SqliteResult<Vec<ChannelRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT channel_id, expected_usd, note, stable_sats, user_channel_id, native_sats
             FROM channels
             WHERE closed_at IS NULL",
        )?;

        let rows = stmt.query_map([], |row| {
            let backing_sats: i64 = row.get(3).unwrap_or(0);
            let native_sats: i64 = row.get(5).unwrap_or(0);
            Ok(ChannelRecord {
                channel_id: row.get(0)?,
                user_channel_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                expected_usd: row.get(1)?,
                note: row.get(2)?,
                backing_sats: backing_sats as u64,
                native_sats: native_sats as u64,
            })
        })?;

        let mut channels = Vec::new();
        for row in rows {
            channels.push(row?);
        }
        Ok(channels)
    }

    /// Load every channel row, active or closed. Use this for forensics /
    /// closed-channel history views, never for reconcile.
    pub fn load_all_channels_including_closed(&self) -> SqliteResult<Vec<ChannelRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT channel_id, expected_usd, note, stable_sats, user_channel_id, native_sats FROM channels",
        )?;

        let rows = stmt.query_map([], |row| {
            let backing_sats: i64 = row.get(3).unwrap_or(0);
            let native_sats: i64 = row.get(5).unwrap_or(0);
            Ok(ChannelRecord {
                channel_id: row.get(0)?,
                user_channel_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                expected_usd: row.get(1)?,
                note: row.get(2)?,
                backing_sats: backing_sats as u64,
                native_sats: native_sats as u64,
            })
        })?;

        let mut channels = Vec::new();
        for row in rows {
            channels.push(row?);
        }
        Ok(channels)
    }

    // =========================================================================
    // Trade Operations
    // =========================================================================

    /// Record a trade
    pub fn record_trade(
        &self,
        channel_id: &str,
        action: &str,
        amount_usd: f64,
        amount_btc: f64,
        btc_price: f64,
        fee_usd: f64,
        new_expected_usd: f64,
        new_backing_sats: Option<u64>,
        payment_id: Option<&str>,
        status: &str,
    ) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO trades (channel_id, action, amount_usd, amount_btc, btc_price, fee_usd,
                                 new_expected_usd, new_backing_sats, payment_id, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                channel_id, action, amount_usd, amount_btc, btc_price, fee_usd, new_expected_usd,
                new_backing_sats, payment_id, status
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Persist every byte of a desktop trade's local intent before invoking LDK.
    #[allow(clippy::too_many_arguments)]
    pub fn record_prepared_trade(
        &self,
        channel_id: &str,
        trade_id: &str,
        action: &str,
        amount_usd: f64,
        amount_btc: f64,
        btc_price: f64,
        fee_usd: f64,
        fee_msat: u64,
        new_expected_usd: f64,
        new_backing_sats: u64,
        expires_at: i64,
    ) -> SqliteResult<i64> {
        if trade_id.len() != 64
            || !trade_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || fee_msat > i64::MAX as u64
            || new_backing_sats > i64::MAX as u64
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO trades (
                channel_id, trade_id, action, amount_usd, amount_btc, btc_price, fee_usd,
                fee_msat, fee_status, new_expected_usd, new_backing_sats, status, expires_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'sending', ?9, ?10, 'sending', ?11)",
            params![
                channel_id,
                trade_id,
                action,
                amount_usd,
                amount_btc,
                btc_price,
                fee_usd,
                fee_msat,
                new_expected_usd,
                new_backing_sats,
                expires_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Attach LDK's payment id immediately after send construction succeeds.
    pub fn attach_trade_payment_id(
        &self,
        trade_db_id: i64,
        payment_id: &str,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE trades
             SET payment_id = ?2, fee_status = 'pending'
             WHERE id = ?1 AND payment_id IS NULL AND status = 'sending'",
            params![trade_db_id, payment_id],
        )?;
        Ok(updated == 1)
    }

    pub fn mark_trade_fee_awaiting_acceptance(&self, payment_id: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE trades
             SET fee_status = 'paid',
                 status = CASE WHEN status = 'sending' THEN 'awaiting_acceptance' ELSE status END
             WHERE payment_id = ?1
               AND status IN ('sending', 'awaiting_acceptance', 'expired', 'pending')",
            params![payment_id],
        )?;
        Ok(updated == 1)
    }

    /// Look up a still-pending trade by its payment_id, for restart recovery. Only returns rows
    /// that have either an exact allocation or a non-zero legacy target.
    pub fn get_pending_trade_by_payment_id(
        &self,
        payment_id: &str,
    ) -> SqliteResult<Option<PendingTradeRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, channel_id, trade_id, payment_id, fee_msat, new_expected_usd, btc_price,
                    new_backing_sats, action, status
             FROM trades
             WHERE payment_id = ?1
               AND status IN ('sending', 'awaiting_acceptance', 'expired', 'pending')
               AND (new_backing_sats IS NOT NULL OR new_expected_usd > 0.0)
             ORDER BY id DESC LIMIT 1",
            params![payment_id],
            |row| {
                Ok(PendingTradeRow {
                    id: row.get(0)?,
                    channel_id: row.get(1)?,
                    trade_id: row.get(2)?,
                    payment_id: row.get(3)?,
                    fee_msat: row.get::<_, i64>(4)?.max(0) as u64,
                    new_expected_usd: row.get(5)?,
                    btc_price: row.get(6)?,
                    new_backing_sats: row.get::<_, Option<i64>>(7)?.map(|v| v.max(0) as u64),
                    action: row.get(8)?,
                    status: row.get(9)?,
                })
            },
        )
        .optional()
    }

    /// Match a signed allocation acknowledgment to a trade authored by this wallet. The LSP can
    /// deliver SYNC_V1 before or after LDK reports the trade-fee payment as successful, so the
    /// allocation itself is the stable correlation key.
    pub fn get_pending_trade_by_allocation(
        &self,
        new_expected_usd: f64,
        new_backing_sats: u64,
    ) -> SqliteResult<Option<PendingTradeRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, channel_id, trade_id, payment_id, fee_msat, new_expected_usd, btc_price,
                    new_backing_sats, action, status
             FROM trades
             WHERE status IN ('sending', 'awaiting_acceptance', 'expired', 'pending')
               AND new_backing_sats = ?1
               AND ABS(new_expected_usd - ?2) <= 0.000000001
             ORDER BY id DESC LIMIT 1",
            params![new_backing_sats, new_expected_usd],
            |row| {
                Ok(PendingTradeRow {
                    id: row.get(0)?,
                    channel_id: row.get(1)?,
                    trade_id: row.get(2)?,
                    payment_id: row.get(3)?,
                    fee_msat: row.get::<_, i64>(4)?.max(0) as u64,
                    new_expected_usd: row.get(5)?,
                    btc_price: row.get(6)?,
                    new_backing_sats: row.get::<_, Option<i64>>(7)?.map(|v| v.max(0) as u64),
                    action: row.get(8)?,
                    status: row.get(9)?,
                })
            },
        )
        .optional()
    }

    /// Resolve a new protocol response by caller-generated id and/or LDK payment id. When both
    /// are present they must identify the same row. A missing stored payment id is allowed so an
    /// authoritative response can repair the crash window after send.
    pub fn get_trade_by_protocol_ids(
        &self,
        trade_id: Option<&str>,
        trade_payment_id: Option<&str>,
    ) -> SqliteResult<Option<PendingTradeRow>> {
        if trade_id.is_none() && trade_payment_id.is_none() {
            return Ok(None);
        }
        let conn = self.conn.lock().unwrap();
        let mut row = if let Some(trade_id) = trade_id {
            conn.query_row(
                "SELECT id, channel_id, trade_id, payment_id, fee_msat, new_expected_usd,
                        btc_price, new_backing_sats, action, status
                 FROM trades WHERE trade_id = ?1 LIMIT 1",
                params![trade_id],
                pending_trade_from_row,
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT id, channel_id, trade_id, payment_id, fee_msat, new_expected_usd,
                        btc_price, new_backing_sats, action, status
                 FROM trades WHERE payment_id = ?1 ORDER BY id DESC LIMIT 1",
                params![trade_payment_id],
                pending_trade_from_row,
            )
            .optional()?
        };
        if let (Some(found), Some(expected_payment_id)) = (&row, trade_payment_id) {
            if found
                .payment_id
                .as_deref()
                .is_some_and(|stored| stored != expected_payment_id)
            {
                row = None;
            }
        }
        Ok(row)
    }

    /// Whether this payment id belongs to any trade, including one already completed by an
    /// earlier SYNC_V1. This prevents an out-of-order PaymentSuccessful event from being
    /// reconciled as an ordinary wallet send.
    pub fn is_trade_payment(&self, payment_id: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM trades WHERE payment_id = ?1)",
            params![payment_id],
            |row| row.get(0),
        )
    }

    /// Update trade status
    pub fn update_trade_status(&self, trade_id: i64, status: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE trades SET status = ?1 WHERE id = ?2",
            params![status, trade_id],
        )?;
        Ok(())
    }

    pub fn mark_trade_send_failed(&self, trade_db_id: i64, reason: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE trades
             SET status = 'failed', fee_status = 'failed', failure_code = 'payment_failed',
                 failure_reason = ?2, resolved_at = strftime('%s', 'now')
             WHERE id = ?1
               AND status IN ('sending', 'awaiting_acceptance', 'expired', 'pending')",
            params![trade_db_id, reason],
        )?;
        Ok(updated == 1)
    }

    pub fn mark_trade_payment_failed(
        &self,
        payment_id: &str,
        reason: &str,
    ) -> SqliteResult<Option<PendingTradeRow>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT id, channel_id, trade_id, payment_id, fee_msat, new_expected_usd,
                        btc_price, new_backing_sats, action, status
                 FROM trades
                 WHERE payment_id = ?1
                   AND status IN ('sending', 'awaiting_acceptance', 'expired', 'pending')
                 ORDER BY id DESC LIMIT 1",
                params![payment_id],
                pending_trade_from_row,
            )
            .optional()?;
        let Some(row) = row else {
            tx.commit()?;
            return Ok(None);
        };
        tx.execute(
            "UPDATE trades
             SET status = 'failed', fee_status = 'failed', failure_code = 'payment_failed',
                 failure_reason = ?2, resolved_at = strftime('%s', 'now')
             WHERE id = ?1",
            params![row.id, reason],
        )?;
        tx.commit()?;
        Ok(Some(row))
    }

    /// Atomically validate rejection correlation and resolve the order. Returns false for a
    /// mismatch or replay; callers still treat a replayed valid control payment as handled.
    pub fn mark_trade_rejected(
        &self,
        trade_db_id: i64,
        trade_id: Option<&str>,
        trade_payment_id: &str,
        reason_code: &str,
        explanation: &str,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE trades
             SET status = 'rejected', fee_status = 'paid', failure_code = ?4,
                 failure_reason = ?5,
                 payment_id = COALESCE(payment_id, ?3), resolved_at = strftime('%s', 'now')
             WHERE id = ?1
               AND status IN ('sending', 'awaiting_acceptance', 'expired', 'pending')
               AND (?2 IS NULL OR trade_id = ?2)
               AND (payment_id IS NULL OR payment_id = ?3)",
            params![
                trade_db_id,
                trade_id,
                trade_payment_id,
                reason_code,
                explanation,
            ],
        )?;
        Ok(updated == 1)
    }

    /// Timer expiry is informational. Authoritative late sync, rejection, or payment-failure paths
    /// explicitly include `expired` rows above.
    pub fn expire_unresolved_trades(&self, now: i64) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE trades
             SET status = 'expired', resolved_at = ?1
             WHERE status IN ('sending', 'awaiting_acceptance')
               AND expires_at IS NOT NULL AND expires_at <= ?1",
            params![now],
        )
    }

    /// Get recent trades across all channels
    pub fn get_recent_trades(&self, limit: usize) -> SqliteResult<Vec<TradeRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, channel_id, action, amount_usd, amount_btc, btc_price, fee_usd,
                    payment_id, status, created_at, trade_id, fee_msat, fee_status,
                    failure_code, failure_reason, expires_at, resolved_at
             FROM trades
             ORDER BY id DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(TradeRecord {
                id: row.get(0)?,
                channel_id: row.get(1)?,
                action: row.get(2)?,
                amount_usd: row.get(3)?,
                amount_btc: row.get(4)?,
                btc_price: row.get(5)?,
                fee_usd: row.get(6)?,
                payment_id: row.get(7)?,
                status: row.get(8)?,
                created_at: row.get(9)?,
                trade_id: row.get(10)?,
                fee_msat: row.get::<_, i64>(11)?.max(0) as u64,
                fee_status: row.get(12)?,
                failure_code: row.get(13)?,
                failure_reason: row.get(14)?,
                expires_at: row.get(15)?,
                resolved_at: row.get(16)?,
            })
        })?;

        rows.collect()
    }

    // =========================================================================
    // Price History Operations
    // =========================================================================

    /// Record a price point
    pub fn record_price(&self, price: f64, source: Option<&str>) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO price_history (price, source) VALUES (?1, ?2)",
            params![price, source],
        )?;
        Ok(())
    }

    /// Record a price with a specific timestamp (for backfill)
    pub fn record_price_at(
        &self,
        price: f64,
        timestamp: i64,
        source: Option<&str>,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO price_history (price, source, timestamp) VALUES (?1, ?2, ?3)",
            params![price, source, timestamp],
        )?;
        Ok(())
    }

    /// Get price history for the last N hours
    pub fn get_price_history(&self, hours: u32) -> SqliteResult<Vec<PriceRecord>> {
        let conn = self.conn.lock().unwrap();
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - (hours as i64 * 3600);

        let mut stmt = conn.prepare(
            "SELECT id, price, source, timestamp
             FROM price_history
             WHERE timestamp > ?1
             ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(params![cutoff], |row| {
            Ok(PriceRecord {
                id: row.get(0)?,
                price: row.get(1)?,
                source: row.get(2)?,
                timestamp: row.get(3)?,
            })
        })?;

        rows.collect()
    }

    /// Get the price from approximately 24 hours ago (for 24h change calculation)
    pub fn get_price_24h_ago(&self) -> SqliteResult<Option<f64>> {
        let conn = self.conn.lock().unwrap();
        let target_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 86400; // 24 hours ago

        // Get the price closest to 24 hours ago
        let mut stmt = conn.prepare(
            "SELECT price FROM price_history
             WHERE timestamp <= ?1
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let mut rows = stmt.query(params![target_time])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Prune old price history (keep last N days)
    pub fn prune_price_history(&self, days_to_keep: u32) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - (days_to_keep as i64 * 86400);

        conn.execute(
            "DELETE FROM price_history WHERE timestamp < ?1",
            params![cutoff],
        )
    }

    // =========================================================================
    // Daily Price Operations (for long-term charts)
    // =========================================================================

    /// Record or update a daily price (OHLC)
    pub fn record_daily_price(
        &self,
        date: &str,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: Option<f64>,
        source: Option<&str>,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO daily_prices (date, open, high, low, close, volume, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![date, open, high, low, close, volume, source],
        )?;
        Ok(())
    }

    /// Bulk insert daily prices (for seeding historical data)
    pub fn bulk_insert_daily_prices(
        &self,
        prices: &[(String, f64, f64, f64, f64, Option<f64>)],
    ) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let mut count = 0;
        for (date, open, high, low, close, volume) in prices {
            conn.execute(
                "INSERT OR IGNORE INTO daily_prices (date, open, high, low, close, volume, source)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'seed')",
                params![date, open, high, low, close, volume],
            )?;
            count += 1;
        }
        Ok(count)
    }

    /// Get daily prices for chart (returns prices within the given number of days from today)
    pub fn get_daily_prices(&self, days: u32) -> SqliteResult<Vec<DailyPriceRecord>> {
        let conn = self.conn.lock().unwrap();

        // Calculate the cutoff date
        let cutoff_date = Utc::now()
            .checked_sub_signed(ChronoDuration::days(days as i64))
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "2000-01-01".to_string());

        let mut stmt = conn.prepare(
            "SELECT date, open, high, low, close, volume
             FROM daily_prices
             WHERE date >= ?1
             ORDER BY date ASC",
        )?;

        let rows = stmt.query_map(params![cutoff_date], |row| {
            Ok(DailyPriceRecord {
                date: row.get(0)?,
                open: row.get(1)?,
                high: row.get(2)?,
                low: row.get(3)?,
                close: row.get(4)?,
                volume: row.get(5)?,
            })
        })?;

        rows.collect()
    }

    /// Get the most recent daily price date
    pub fn get_latest_daily_price_date(&self) -> SqliteResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT date FROM daily_prices ORDER BY date DESC LIMIT 1")?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Get the oldest daily price date
    pub fn get_oldest_daily_price_date(&self) -> SqliteResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT date FROM daily_prices ORDER BY date ASC LIMIT 1")?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Get daily price count
    pub fn get_daily_price_count(&self) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM daily_prices")?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok(count)
    }

    // =========================================================================
    // Payment Operations
    // =========================================================================

    /// Check if a payment with the given payment_id already exists
    pub fn payment_exists(&self, payment_id: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT 1 FROM payments WHERE payment_id = ?1 LIMIT 1")?;
        let exists = stmt.exists(params![payment_id])?;
        Ok(exists)
    }

    /// Whether a payment (by payment_id) is a recorded stability (peg-maintenance) payment.
    pub fn is_stability_payment(&self, payment_id: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT 1 FROM payments WHERE payment_id = ?1 AND payment_type = 'stability' LIMIT 1",
        )?;
        let exists = stmt.exists(params![payment_id])?;
        Ok(exists)
    }

    /// Atomically persist an outgoing payment's completed status and post-settlement channel state.
    ///
    /// `event_id` is the LDK `PaymentId` when available (falling back to its payment hash for old
    /// events). `stable_reconciled` is the durable idempotency marker: a replay after commit returns
    /// `Ok(false)` without applying the supplied state again. Any error rolls back the marker,
    /// payment completion, and channel update together so the LDK event can be retried safely.
    pub fn persist_outgoing_reconciliation(
        &self,
        event_id: &str,
        payment_db_id: Option<i64>,
        fee_msat: Option<u64>,
        channel_id: &str,
        user_channel_id: &str,
        expected_usd: f64,
        backing_sats: u64,
        native_sats: u64,
        note: Option<&str>,
        btc_price: Option<f64>,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result: SqliteResult<bool> = (|| {
            let before: Option<(f64, i64, i64)> = conn
                .query_row(
                    "SELECT expected_usd, stable_sats, native_sats
                     FROM channels WHERE user_channel_id = ?1",
                    params![user_channel_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let payment_row = if let Some(id) = payment_db_id {
                conn.query_row(
                    "SELECT id, stable_reconciled FROM payments WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0)),
                )
                .optional()?
            } else {
                conn.query_row(
                    "SELECT id, stable_reconciled FROM payments
                     WHERE payment_id = ?1 ORDER BY id DESC LIMIT 1",
                    params![event_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0)),
                )
                .optional()?
            };

            if matches!(payment_row, Some((_, true))) {
                return Ok(false);
            }

            let updated = conn.execute(
                "UPDATE channels SET channel_id = ?1, expected_usd = ?2, stable_sats = ?3,
                                     note = ?4, user_channel_id = ?5, native_sats = ?6,
                                     closed_at = NULL, updated_at = strftime('%s', 'now')
                 WHERE user_channel_id = ?5",
                params![
                    channel_id,
                    expected_usd,
                    backing_sats as i64,
                    note,
                    user_channel_id,
                    native_sats as i64,
                ],
            )?;
            if updated == 0 {
                conn.execute(
                    "INSERT INTO channels
                        (channel_id, user_channel_id, expected_usd, stable_sats, native_sats, note)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(channel_id) DO UPDATE SET
                        user_channel_id = ?2, expected_usd = ?3, stable_sats = ?4,
                        native_sats = ?5, note = ?6, closed_at = NULL,
                        updated_at = strftime('%s', 'now')",
                    params![
                        channel_id,
                        user_channel_id,
                        expected_usd,
                        backing_sats as i64,
                        native_sats as i64,
                        note,
                    ],
                )?;
            }

            let fee_msat_db = fee_msat.map(|fee| fee as i64);
            if let Some((id, _)) = payment_row {
                conn.execute(
                    "UPDATE payments SET payment_id = COALESCE(payment_id, ?1),
                                         status = 'completed',
                                         fee_msat = COALESCE(?2, fee_msat),
                                         stable_reconciled = 1
                     WHERE id = ?3",
                    params![event_id, fee_msat_db, id],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO payments
                        (payment_id, payment_type, direction, amount_msat, btc_price, status,
                         fee_msat, stable_reconciled)
                     VALUES (?1, 'lightning', 'sent', 0, ?2, 'completed', ?3, 1)",
                    params![event_id, btc_price, fee_msat_db.unwrap_or(0)],
                )?;
            }

            let draft = LedgerEventDraft {
                event_type: "PAYMENT_OUTGOING_RECONCILED".to_owned(),
                category: "payment".to_owned(),
                severity: "info".to_owned(),
                status: "completed".to_owned(),
                source: "ldk_event".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: Some(format!(
                    "ldk-event:outgoing-reconcile:{event_id}:{user_channel_id}"
                )),
                before: before.map(|(expected, backing, native)| AccountingSnapshot {
                    expected_usd: Some(expected),
                    backing_sats: u64::try_from(backing).ok(),
                    native_sats: u64::try_from(native).ok(),
                    live_receiver_sats: u64::try_from(backing.saturating_add(native)).ok(),
                    btc_price,
                    fee_msat,
                    ..Default::default()
                }),
                after: Some(AccountingSnapshot {
                    expected_usd: Some(expected_usd),
                    backing_sats: Some(backing_sats),
                    native_sats: Some(native_sats),
                    live_receiver_sats: Some(backing_sats.saturating_add(native_sats)),
                    btc_price,
                    fee_msat,
                    ..Default::default()
                }),
                detail: serde_json::json!({
                    "payment_id": event_id,
                    "channel_id": channel_id,
                    "user_channel_id": user_channel_id,
                    "fee_msat": fee_msat,
                    "btc_price": btc_price,
                    "new_expected_usd": expected_usd,
                    "new_backing_sats": backing_sats,
                    "new_native_sats": native_sats,
                    "live_receiver_sats": backing_sats.saturating_add(native_sats),
                }),
                refs: vec![
                    LedgerRef::new("payment_id", event_id),
                    LedgerRef::new("user_channel_id", user_channel_id),
                    LedgerRef::new("channel_id", channel_id),
                ],
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }

            Ok(true)
        })();

        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Record a payment
    /// payment_type: "stability", "lightning", "splice_in", "splice_out", or "manual"
    pub fn record_payment(
        &self,
        payment_id: Option<&str>,
        payment_type: &str,
        direction: &str,
        amount_msat: u64,
        amount_usd: Option<f64>,
        btc_price: Option<f64>,
        counterparty: Option<&str>,
        status: &str,
        txid: Option<&str>,
        address: Option<&str>,
    ) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, txid, address)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![payment_id, payment_type, direction, amount_msat as i64, amount_usd, btc_price, counterparty, status, txid, address],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Persist a sent stability payment and its optimistic channel allocation in one transaction.
    /// The before/after values make a later PaymentFailed rollback both durable across restart and
    /// conditional, so it cannot overwrite a newer trade, sync, or reconciliation.
    #[allow(clippy::too_many_arguments)]
    pub fn record_pending_stability_payment(
        &self,
        payment_id: &str,
        amount_msat: u64,
        amount_usd: Option<f64>,
        btc_price: f64,
        counterparty: &str,
        channel_id: &str,
        user_channel_id: &str,
        expected_usd: f64,
        backing_sats_before: u64,
        backing_sats_after: u64,
        native_sats: u64,
        note: Option<&str>,
    ) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result: SqliteResult<i64> = (|| {
            let before = i64::try_from(backing_sats_before)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, i64::MAX))?;
            let after = i64::try_from(backing_sats_after)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, i64::MAX))?;
            let native = i64::try_from(native_sats)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, i64::MAX))?;
            let amount = i64::try_from(amount_msat)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, i64::MAX))?;

            conn.execute(
                "INSERT INTO payments
                    (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price,
                     counterparty, status, user_channel_id, backing_sats_before,
                     backing_sats_after)
                 VALUES (?1, 'stability', 'sent', ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?8)",
                params![
                    payment_id,
                    amount,
                    amount_usd,
                    btc_price,
                    counterparty,
                    user_channel_id,
                    before,
                    after,
                ],
            )?;
            let payment_db_id = conn.last_insert_rowid();

            let updated = conn.execute(
                "UPDATE channels SET channel_id = ?1, expected_usd = ?2, stable_sats = ?3,
                                     note = ?4, user_channel_id = ?5, native_sats = ?6,
                                     closed_at = NULL, updated_at = strftime('%s', 'now')
                 WHERE user_channel_id = ?5",
                params![channel_id, expected_usd, after, note, user_channel_id, native],
            )?;
            if updated == 0 {
                conn.execute(
                    "INSERT INTO channels
                        (channel_id, user_channel_id, expected_usd, stable_sats, native_sats, note)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(channel_id) DO UPDATE SET
                        user_channel_id = ?2, expected_usd = ?3, stable_sats = ?4,
                        native_sats = ?5, note = ?6, closed_at = NULL,
                        updated_at = strftime('%s', 'now')",
                    params![channel_id, user_channel_id, expected_usd, after, native, note],
                )?;
            }

            let before_snapshot = AccountingSnapshot {
                expected_usd: Some(expected_usd),
                backing_sats: Some(backing_sats_before),
                native_sats: Some(native_sats),
                live_receiver_sats: Some(backing_sats_before.saturating_add(native_sats)),
                btc_price: Some(btc_price),
                amount_msat: Some(amount_msat),
                amount_usd,
                ..Default::default()
            };
            let after_snapshot = AccountingSnapshot {
                backing_sats: Some(backing_sats_after),
                live_receiver_sats: Some(backing_sats_after.saturating_add(native_sats)),
                ..before_snapshot.clone()
            };
            let draft = LedgerEventDraft {
                event_type: "STABILITY_PAYMENT_SENT".to_owned(),
                category: "stability".to_owned(),
                severity: "info".to_owned(),
                status: "pending".to_owned(),
                source: "desktop_wallet".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: Some(format!(
                    "desktop-wallet:stability-payment-sent:{payment_id}"
                )),
                before: Some(before_snapshot),
                after: Some(after_snapshot),
                detail: serde_json::json!({
                    "payment_id": payment_id,
                    "channel_id": channel_id,
                    "user_channel_id": user_channel_id,
                    "counterparty_node_id": counterparty,
                    "direction": "outbound",
                    "amount_msat": amount_msat,
                    "amount_usd": amount_usd,
                    "btc_price": btc_price,
                    "before_backing_sats": backing_sats_before,
                    "after_backing_sats": backing_sats_after,
                    "native_sats": native_sats,
                    "expected_usd": expected_usd,
                    "status": "pending",
                }),
                refs: vec![
                    LedgerRef::new("payment_id", payment_id),
                    LedgerRef::new("channel_id", channel_id),
                    LedgerRef::new("user_channel_id", user_channel_id),
                    LedgerRef::new("node_id", counterparty),
                ],
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }

            Ok(payment_db_id)
        })();
        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Mark a pending stability payment failed and restore its prior backing only if no newer
    /// accounting transition has replaced the optimistic after-state.
    pub fn fail_pending_stability_payment(
        &self,
        payment_id: &str,
    ) -> SqliteResult<Option<StabilityPaymentRollback>> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result: SqliteResult<Option<StabilityPaymentRollback>> = (|| {
            let row: Option<(i64, Option<String>, Option<i64>, Option<i64>)> = conn
                .query_row(
                    "SELECT id, user_channel_id, backing_sats_before, backing_sats_after
                     FROM payments
                     WHERE payment_id = ?1 AND payment_type = 'stability'
                       AND direction = 'sent' AND status = 'pending'
                     ORDER BY id DESC LIMIT 1",
                    params![payment_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((payment_db_id, user_channel_id, before_i64, after_i64)) = row else {
                return Ok(None);
            };

            conn.execute(
                "UPDATE payments SET status = 'failed' WHERE id = ?1 AND status = 'pending'",
                params![payment_db_id],
            )?;

            let backing_sats_before = before_i64.and_then(|value| u64::try_from(value).ok());
            let backing_sats_after = after_i64.and_then(|value| u64::try_from(value).ok());
            let channel_before: Option<(String, f64, i64, i64)> = match user_channel_id.as_deref() {
                Some(uid) => conn
                    .query_row(
                        "SELECT channel_id, expected_usd, stable_sats, native_sats
                         FROM channels WHERE user_channel_id = ?1",
                        params![uid],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .optional()?,
                None => None,
            };
            let restored = match (
                user_channel_id.as_deref(),
                backing_sats_before,
                backing_sats_after,
            ) {
                (Some(uid), Some(before), Some(after)) => {
                    let before = i64::try_from(before).ok();
                    let after = i64::try_from(after).ok();
                    match (before, after) {
                        (Some(before), Some(after)) => {
                            conn.execute(
                                "UPDATE channels SET stable_sats = ?1,
                                                     updated_at = strftime('%s', 'now')
                                 WHERE user_channel_id = ?2 AND stable_sats = ?3",
                                params![before, uid, after],
                            )? > 0
                        }
                        _ => false,
                    }
                }
                _ => false,
            };

            let channel_after: Option<(String, f64, i64, i64)> = match user_channel_id.as_deref() {
                Some(uid) => conn
                    .query_row(
                        "SELECT channel_id, expected_usd, stable_sats, native_sats
                         FROM channels WHERE user_channel_id = ?1",
                        params![uid],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .optional()?,
                None => None,
            };
            let snapshot = |row: &(String, f64, i64, i64)| AccountingSnapshot {
                expected_usd: Some(row.1),
                backing_sats: u64::try_from(row.2).ok(),
                native_sats: u64::try_from(row.3).ok(),
                live_receiver_sats: u64::try_from(row.2.saturating_add(row.3)).ok(),
                ..Default::default()
            };
            let mut refs = vec![LedgerRef::new("payment_id", payment_id)];
            if let Some(uid) = user_channel_id.as_deref() {
                refs.push(LedgerRef::new("user_channel_id", uid));
            }
            if let Some((channel_id, ..)) = channel_after.as_ref().or(channel_before.as_ref()) {
                refs.push(LedgerRef::new("channel_id", channel_id));
            }
            let draft = LedgerEventDraft {
                event_type: "STABILITY_PAYMENT_FAILED_RECONCILED".to_owned(),
                category: "stability".to_owned(),
                severity: "error".to_owned(),
                status: "failed".to_owned(),
                source: "desktop_wallet".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: Some(format!(
                    "desktop-wallet:stability-payment-failure:{payment_id}"
                )),
                before: channel_before.as_ref().map(snapshot),
                after: channel_after.as_ref().map(snapshot),
                detail: serde_json::json!({
                    "payment_id": payment_id,
                    "user_channel_id": user_channel_id,
                    "backing_sats_before": backing_sats_before,
                    "backing_sats_after": backing_sats_after,
                    "database_restored": restored,
                    "status": "failed",
                }),
                refs,
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }

            Ok(Some(StabilityPaymentRollback {
                user_channel_id,
                backing_sats_before,
                backing_sats_after,
                restored,
            }))
        })();
        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Complete a desktop stability payment and append its terminal ledger event atomically.
    pub fn complete_pending_stability_payment(
        &self,
        payment_id: &str,
        payment_hash: &str,
        fee_paid_msat: Option<u64>,
    ) -> SqliteResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let payment: Option<(i64, Option<String>, i64, Option<f64>, Option<f64>, Option<String>)> =
            tx.query_row(
                "SELECT id, user_channel_id, amount_msat, amount_usd, btc_price, counterparty
                 FROM payments
                 WHERE payment_id = ?1 AND payment_type = 'stability'
                   AND direction = 'sent' AND status = 'pending'
                 ORDER BY id DESC LIMIT 1",
                params![payment_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((payment_db_id, user_channel_id, amount_msat, amount_usd, btc_price, counterparty)) =
            payment
        else {
            tx.commit()?;
            return Ok(false);
        };
        let fee = fee_paid_msat.and_then(|value| i64::try_from(value).ok());
        let updated = tx.execute(
            "UPDATE payments SET status = 'completed', fee_msat = COALESCE(?1, fee_msat)
             WHERE id = ?2 AND status = 'pending'",
            params![fee, payment_db_id],
        )?;
        if updated != 1 {
            tx.commit()?;
            return Ok(false);
        }

        let channel: Option<(String, f64, i64, i64)> = match user_channel_id.as_deref() {
            Some(user_channel_id) => tx
                .query_row(
                    "SELECT channel_id, expected_usd, stable_sats, native_sats
                     FROM channels WHERE user_channel_id = ?1",
                    params![user_channel_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?,
            None => None,
        };
        let amount_msat = u64::try_from(amount_msat).ok();
        let after = channel.as_ref().map(|(_, expected_usd, backing, native)| {
            AccountingSnapshot {
                expected_usd: Some(*expected_usd),
                backing_sats: u64::try_from(*backing).ok(),
                native_sats: u64::try_from(*native).ok(),
                live_receiver_sats: u64::try_from(backing.saturating_add(*native)).ok(),
                btc_price,
                amount_msat,
                amount_usd,
                fee_msat: fee_paid_msat,
                ..Default::default()
            }
        });
        let mut refs = vec![
            LedgerRef::new("payment_id", payment_id),
            LedgerRef::new("payment_hash", payment_hash),
        ];
        if let Some(user_channel_id) = user_channel_id.as_deref() {
            refs.push(LedgerRef::new("user_channel_id", user_channel_id));
        }
        if let Some((channel_id, ..)) = channel.as_ref() {
            refs.push(LedgerRef::new("channel_id", channel_id));
        }
        if let Some(counterparty) = counterparty.as_deref() {
            refs.push(LedgerRef::new("node_id", counterparty));
        }
        let draft = LedgerEventDraft {
            event_type: "STABILITY_PAYMENT_SETTLED".to_owned(),
            category: "stability".to_owned(),
            severity: "info".to_owned(),
            status: "completed".to_owned(),
            source: "desktop_wallet".to_owned(),
            completeness: LedgerCompleteness::Observed,
            occurred_at_ms: Utc::now().timestamp_millis(),
            dedup_key: Some(format!(
                "desktop-wallet:stability-payment-settled:{payment_id}"
            )),
            before: None,
            after,
            detail: serde_json::json!({
                "payment_id": payment_id,
                "payment_hash": payment_hash,
                "user_channel_id": user_channel_id,
                "channel_id": channel.as_ref().map(|row| row.0.as_str()),
                "counterparty_node_id": counterparty,
                "amount_msat": amount_msat,
                "amount_usd": amount_usd,
                "btc_price": btc_price,
                "fee_paid_msat": fee_paid_msat,
                "direction": "outbound",
                "status": "completed",
            }),
            refs,
        };
        let ledger_outcome = ledger::append_on_connection(&tx, &draft)?;
        tx.commit()?;
        if ledger_outcome.inserted {
            crate::audit::mirror_committed_ledger_event(&draft, ledger_outcome.event_id);
        }
        Ok(true)
    }

    /// Insert a payment and optionally update channel backing sats in one SQLite transaction.
    ///
    /// The dedup check runs inside `BEGIN IMMEDIATE` so concurrent writers
    /// (including other processes) can't race between the check and the insert.
    /// The backing update is a floored delta: `new = max(0, current + delta)` —
    /// the payment already happened, so refusing to record (or going negative)
    /// would misaccount; clamping is surfaced via `PaymentPersistence::clamped`.
    ///
    /// Errors: if a backing update is requested but no `channels` row exists for
    /// `user_channel_id`, the transaction is rolled back and the distinct
    /// `rusqlite::Error::QueryReturnedNoRows` is returned (match it with
    /// `is_missing_channel_row`) so callers can recreate the row and retry.
    /// No other failure mode of this function returns that variant.
    pub fn record_payment_and_maybe_update_backing(
        &self,
        payment_id: Option<&str>,
        payment_type: &str,
        direction: &str,
        amount_msat: u64,
        amount_usd: Option<f64>,
        btc_price: Option<f64>,
        status: &str,
        user_channel_id: Option<&str>,
        backing_delta_sats: Option<i64>,
    ) -> SqliteResult<PaymentPersistence> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result: SqliteResult<PaymentPersistence> = (|| {
            // Dedup check inside the transaction to prevent cross-process TOCTOU
            if let Some(pid) = payment_id {
                let exists: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM payments WHERE payment_id = ?1 LIMIT 1",
                        params![pid],
                        |row| row.get(0),
                    )
                    .optional()?;
                if exists.is_some() {
                    return Ok(PaymentPersistence {
                        is_new: false,
                        new_backing: None,
                        clamped: false,
                    });
                }
            }
            conn.execute(
                "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    payment_id, payment_type, direction,
                    amount_msat as i64, amount_usd, btc_price, status
                ],
            )?;
            let payment_row_id = conn.last_insert_rowid();
            let mut new_backing = None;
            let mut clamped = false;
            let mut channel_before: Option<(f64, i64, i64)> = None;
            if let Some(delta) = backing_delta_sats {
                // user_channel_id must be set when a backing update is requested.
                let ucid = user_channel_id.ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "user_channel_id required for backing update".to_string(),
                    )
                })?;
                channel_before = conn
                    .query_row(
                        "SELECT expected_usd, stable_sats, native_sats
                         FROM channels WHERE user_channel_id = ?1",
                        params![ucid],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                // Distinct missing-channel-row error — see doc comment.
                let current = channel_before
                    .as_ref()
                    .map(|row| row.1)
                    .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
                let target = current.saturating_add(delta);
                let updated = target.max(0);
                clamped = target < 0;
                conn.execute(
                    "UPDATE channels SET stable_sats = ?1, updated_at = strftime('%s', 'now') WHERE user_channel_id = ?2",
                    params![updated, ucid],
                )?;
                new_backing = Some(updated);
            }
            let after_backing = new_backing.and_then(|value| u64::try_from(value).ok());
            let before_snapshot = channel_before.map(|(expected, backing, native)| AccountingSnapshot {
                expected_usd: Some(expected),
                backing_sats: u64::try_from(backing).ok(),
                native_sats: u64::try_from(native).ok(),
                live_receiver_sats: u64::try_from(backing.saturating_add(native)).ok(),
                btc_price,
                amount_msat: Some(amount_msat),
                amount_usd,
                ..Default::default()
            });
            let after_snapshot = before_snapshot.as_ref().map(|before| AccountingSnapshot {
                expected_usd: before.expected_usd,
                backing_sats: after_backing.or(before.backing_sats),
                native_sats: before.native_sats,
                live_receiver_sats: after_backing
                    .or(before.backing_sats)
                    .zip(before.native_sats)
                    .map(|(backing, native)| backing.saturating_add(native)),
                btc_price,
                amount_msat: Some(amount_msat),
                amount_usd,
                ..Default::default()
            });
            let event_type = if payment_type == "stability" {
                "STABILITY_PAYMENT_RECORDED"
            } else {
                "PAYMENT_RECORDED"
            };
            let mut refs = Vec::new();
            if let Some(payment_id) = payment_id {
                refs.push(LedgerRef::new("payment_id", payment_id));
            }
            if let Some(user_channel_id) = user_channel_id {
                refs.push(LedgerRef::new("user_channel_id", user_channel_id));
            }
            let draft = LedgerEventDraft {
                event_type: event_type.to_owned(),
                category: if payment_type == "stability" { "stability" } else { "payment" }.to_owned(),
                severity: "info".to_owned(),
                status: status.to_owned(),
                source: "ldk_event".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: Some(format!(
                    "ldk-event:payment-recorded:{}:{status}",
                    payment_id.map(str::to_owned).unwrap_or_else(|| format!("row-{payment_row_id}"))
                )),
                before: before_snapshot,
                after: after_snapshot,
                detail: serde_json::json!({
                    "payment_id": payment_id,
                    "payment_type": payment_type,
                    "direction": direction,
                    "amount_msat": amount_msat,
                    "amount_usd": amount_usd,
                    "btc_price": btc_price,
                    "status": status,
                    "user_channel_id": user_channel_id,
                    "backing_delta_sats": backing_delta_sats,
                    "new_backing_sats": new_backing,
                    "clamped": clamped,
                }),
                refs,
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }
            Ok(PaymentPersistence {
                is_new: true,
                new_backing,
                clamped,
            })
        })();
        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Update payment status (pending -> completed/failed) and optionally set fee
    pub fn update_payment_status(
        &self,
        payment_db_id: i64,
        status: &str,
        fee_msat: Option<u64>,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        if let Some(fee) = fee_msat {
            conn.execute(
                "UPDATE payments SET status = ?1, fee_msat = ?2 WHERE id = ?3",
                params![status, fee as i64, payment_db_id],
            )?;
        } else {
            conn.execute(
                "UPDATE payments SET status = ?1 WHERE id = ?2",
                params![status, payment_db_id],
            )?;
        }
        Ok(())
    }

    /// Update payment status by payment_id string and optionally set fee
    pub fn update_payment_status_by_pid(
        &self,
        payment_id: &str,
        status: &str,
        fee_msat: Option<u64>,
    ) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let rows = if let Some(fee) = fee_msat {
            conn.execute(
                "UPDATE payments SET status = ?1, fee_msat = ?2 WHERE payment_id = ?3 AND status = 'pending'",
                params![status, fee as i64, payment_id],
            )?
        } else {
            conn.execute(
                "UPDATE payments SET status = ?1 WHERE payment_id = ?2 AND status = 'pending'",
                params![status, payment_id],
            )?
        };
        Ok(rows)
    }

    /// Set txid on the most recent pending splice_in payment (recorded before txid was known)
    pub fn set_pending_splice_txid(&self, txid: &str) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE payments SET txid = ?1, payment_id = ?1
             WHERE id = (SELECT id FROM payments WHERE payment_type = 'splice_in' AND status IN ('pending','failed') AND txid IS NULL ORDER BY id DESC LIMIT 1)",
            params![txid],
        )?;
        Ok(rows)
    }

    /// Set txid on the most recent pending splice_out payment (desktop records it before txid is known)
    pub fn set_pending_splice_out_txid(&self, txid: &str) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE payments SET txid = ?1, payment_id = ?1
             WHERE id = (SELECT id FROM payments WHERE payment_type = 'splice_out' AND status IN ('pending','failed') AND txid IS NULL ORDER BY id DESC LIMIT 1)",
            params![txid],
        )?;
        Ok(rows)
    }

    pub fn complete_latest_splice(&self, txid: Option<&str>) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let rows = match txid {
            Some(txid) if !txid.is_empty() => conn.execute(
                "UPDATE payments SET status = 'completed'
                 WHERE payment_type IN ('splice_in','splice_out') AND txid = ?1 AND status IN ('pending','failed')",
                params![txid],
            )?,
            _ => conn.execute(
                "UPDATE payments SET status = 'completed'
                 WHERE id = (SELECT id FROM payments WHERE payment_type IN ('splice_in','splice_out') AND status IN ('pending','failed') ORDER BY id DESC LIMIT 1)",
                [],
            )?,
        };
        Ok(rows)
    }

    pub fn fail_latest_pending_splice(&self) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE payments SET status = 'failed'
             WHERE id = (SELECT id FROM payments WHERE payment_type IN ('splice_in','splice_out') AND status = 'pending' ORDER BY id DESC LIMIT 1)",
            [],
        )?;
        Ok(rows)
    }

    /// Whether the splice with this funding txid was already stable-reconciled.
    pub fn is_splice_stable_reconciled(&self, txid: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM payments
             WHERE payment_type IN ('splice_in','splice_out') AND txid = ?1 AND stable_reconciled = 1",
            params![txid],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Return the recorded splice direction ("in" or "out") for a funding transaction.
    pub fn get_splice_direction(&self, txid: &str) -> SqliteResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT CASE payment_type
                        WHEN 'splice_in' THEN 'in'
                        WHEN 'splice_out' THEN 'out'
                    END
             FROM payments
             WHERE txid = ?1 AND payment_type IN ('splice_in','splice_out')
             ORDER BY id DESC LIMIT 1",
            params![txid],
            |row| row.get(0),
        )
        .optional()
    }

    /// Atomically persist post-splice allocation state and its durable idempotency marker.
    /// Returns false when this funding transaction was already reconciled.
    pub fn persist_splice_reconciliation(
        &self,
        txid: &str,
        channel_id: &str,
        user_channel_id: &str,
        expected_usd: f64,
        backing_sats: u64,
        native_sats: u64,
        note: Option<&str>,
    ) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result: SqliteResult<bool> = (|| {
            let before: Option<(f64, i64, i64, String)> = conn
                .query_row(
                    "SELECT expected_usd, stable_sats, native_sats, channel_id
                     FROM channels WHERE user_channel_id = ?1",
                    params![user_channel_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let payment = conn
                .query_row(
                    "SELECT id, stable_reconciled FROM payments
                     WHERE txid = ?1 AND payment_type IN ('splice_in','splice_out')
                     ORDER BY id DESC LIMIT 1",
                    params![txid],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0)),
                )
                .optional()?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;

            if payment.1 {
                return Ok(false);
            }

            let updated = conn.execute(
                "UPDATE channels SET channel_id = ?1, expected_usd = ?2, stable_sats = ?3,
                                     note = ?4, user_channel_id = ?5, native_sats = ?6,
                                     closed_at = NULL, updated_at = strftime('%s', 'now')
                 WHERE user_channel_id = ?5",
                params![
                    channel_id,
                    expected_usd,
                    backing_sats as i64,
                    note,
                    user_channel_id,
                    native_sats as i64,
                ],
            )?;
            if updated == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }

            conn.execute(
                "UPDATE payments SET status = 'completed', stable_reconciled = 1 WHERE id = ?1",
                params![payment.0],
            )?;
            let draft = LedgerEventDraft {
                event_type: "SPLICE_RECONCILED".to_owned(),
                category: "channel".to_owned(),
                severity: "info".to_owned(),
                status: "completed".to_owned(),
                source: "ldk_event".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: Some(format!(
                    "ldk-event:splice-reconcile:{txid}:{user_channel_id}"
                )),
                before: before.map(|(expected, backing, native, _)| AccountingSnapshot {
                    expected_usd: Some(expected),
                    backing_sats: u64::try_from(backing).ok(),
                    native_sats: u64::try_from(native).ok(),
                    live_receiver_sats: u64::try_from(backing.saturating_add(native)).ok(),
                    ..Default::default()
                }),
                after: Some(AccountingSnapshot {
                    expected_usd: Some(expected_usd),
                    backing_sats: Some(backing_sats),
                    native_sats: Some(native_sats),
                    live_receiver_sats: Some(backing_sats.saturating_add(native_sats)),
                    ..Default::default()
                }),
                detail: serde_json::json!({
                    "txid": txid,
                    "channel_id": channel_id,
                    "user_channel_id": user_channel_id,
                    "new_expected_usd": expected_usd,
                    "new_backing_sats": backing_sats,
                    "new_native_sats": native_sats,
                    "live_receiver_sats": backing_sats.saturating_add(native_sats),
                }),
                refs: vec![
                    LedgerRef::new("transaction_id", txid),
                    LedgerRef::new("user_channel_id", user_channel_id),
                    LedgerRef::new("channel_id", channel_id),
                ],
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }
            Ok(true)
        })();

        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Update confirmations and status for a payment by txid
    pub fn update_payment_confirmations(
        &self,
        txid: &str,
        confirmations: u32,
        status: &str,
    ) -> SqliteResult<usize> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE payments SET confirmations = ?1, status = ?2 WHERE txid = ?3",
            params![confirmations as i32, status, txid],
        )?;
        Ok(rows)
    }

    /// Get recent payments
    pub fn get_recent_payments(&self, limit: usize) -> SqliteResult<Vec<PaymentRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, created_at, fee_msat, txid, address, confirmations
             FROM payments
             WHERE NOT (payment_type = 'lightning' AND amount_msat < 1000)
             ORDER BY id DESC
             LIMIT ?1"
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(PaymentRecord {
                id: row.get(0)?,
                payment_id: row.get(1)?,
                payment_type: row.get(2)?,
                direction: row.get(3)?,
                amount_msat: row.get::<_, i64>(4)? as u64,
                amount_usd: row.get(5)?,
                btc_price: row.get(6)?,
                counterparty: row.get(7)?,
                status: row.get(8)?,
                created_at: row.get(9)?,
                fee_msat: row.get::<_, i64>(10).unwrap_or(0) as u64,
                txid: row.get(11)?,
                address: row.get(12)?,
                confirmations: row.get::<_, i32>(13).unwrap_or(0) as u32,
            })
        })?;

        rows.collect()
    }

    // =========================================================================
    // On-chain Transaction Operations
    // =========================================================================

    /// Record an on-chain transaction
    pub fn record_onchain_tx(
        &self,
        txid: &str,
        direction: &str,
        amount_sats: u64,
        address: Option<&str>,
        btc_price: Option<f64>,
        status: &str,
    ) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO onchain_txs (txid, direction, amount_sats, address, btc_price, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![txid, direction, amount_sats, address, btc_price, status],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Update on-chain transaction status and confirmations
    pub fn update_onchain_tx_status(
        &self,
        txid: &str,
        status: &str,
        confirmations: u32,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE onchain_txs SET status = ?1, confirmations = ?2 WHERE txid = ?3",
            params![status, confirmations, txid],
        )?;
        Ok(())
    }

    /// Get recent on-chain transactions
    pub fn get_recent_onchain_txs(&self, limit: usize) -> SqliteResult<Vec<OnchainTxRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, txid, direction, amount_sats, address, btc_price, status, confirmations, created_at
             FROM onchain_txs
             ORDER BY created_at DESC
             LIMIT ?1"
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(OnchainTxRecord {
                id: row.get(0)?,
                txid: row.get(1)?,
                direction: row.get(2)?,
                amount_sats: row.get(3)?,
                address: row.get(4)?,
                btc_price: row.get(5)?,
                status: row.get(6)?,
                confirmations: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

        rows.collect()
    }

    /// Record a settlement keysend by payment_id + kind ("stability"/"sync"). INSERT OR IGNORE so a duplicate id is a no-op.
    pub fn record_settlement(&self, payment_id: &str, kind: &str) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO settlement_payments (payment_id, kind)
             VALUES (?1, ?2)",
            params![payment_id, kind],
        )?;
        Ok(())
    }

    /// Like `record_settlement` but also records the `user_channel_id` for outcome-event keying.
    pub fn record_settlement_with_channel(
        &self,
        payment_id: &str,
        kind: &str,
        user_channel_id: &str,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO settlement_payments (payment_id, kind, user_channel_id)
             VALUES (?1, ?2, ?3)",
            params![payment_id, kind, user_channel_id],
        )?;
        Ok(())
    }

    /// Record the reversible allocation transition, optimistic channel state, and ledger event in
    /// one transaction. Returns false only if the payment id was already present or the supplied
    /// rollback metadata is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn record_stability_settlement_with_rollback(
        &self,
        payment_id: &str,
        user_channel_id: &str,
        channel_id: &str,
        backing_sats_before: u64,
        backing_sats_after: u64,
        native_sats_before: u64,
        expected_usd: f64,
        last_stability_payment_before: i64,
        amount_msat: u64,
        direction: &str,
        counterparty: &str,
        note: Option<&str>,
    ) -> SqliteResult<bool> {
        if backing_sats_before > i64::MAX as u64
            || backing_sats_after > i64::MAX as u64
            || native_sats_before > i64::MAX as u64
            || !expected_usd.is_finite()
            || expected_usd < 0.0
        {
            return Ok(false);
        }
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let mut mirror = None;
        let result = (|| {
            let before: Option<(String, f64, i64, i64)> = conn
                .query_row(
                    "SELECT channel_id, expected_usd, stable_sats, native_sats
                     FROM channels WHERE user_channel_id = ?1",
                    params![user_channel_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO settlement_payments
                    (payment_id, kind, user_channel_id, backing_sats_before,
                     backing_sats_after, native_sats_before, expected_usd,
                     last_stability_payment_before)
                 VALUES (?1, 'stability', ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    payment_id,
                    user_channel_id,
                    backing_sats_before as i64,
                    backing_sats_after as i64,
                    native_sats_before as i64,
                    expected_usd,
                    last_stability_payment_before,
                ],
            )?;
            if inserted != 1 {
                return Ok(false);
            }

            let updated = conn.execute(
                "UPDATE channels SET channel_id = ?1, expected_usd = ?2, stable_sats = ?3,
                                     native_sats = ?4, note = ?5, closed_at = NULL,
                                     updated_at = strftime('%s', 'now')
                 WHERE user_channel_id = ?6",
                params![
                    channel_id,
                    expected_usd,
                    backing_sats_after as i64,
                    native_sats_before as i64,
                    note,
                    user_channel_id,
                ],
            )?;
            if updated == 0 {
                conn.execute(
                    "INSERT INTO channels
                        (channel_id, user_channel_id, expected_usd, stable_sats, native_sats, note)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(channel_id) DO UPDATE SET
                        user_channel_id = ?2, expected_usd = ?3, stable_sats = ?4,
                        native_sats = ?5, note = ?6, closed_at = NULL,
                        updated_at = strftime('%s', 'now')",
                    params![
                        channel_id,
                        user_channel_id,
                        expected_usd,
                        backing_sats_after as i64,
                        native_sats_before as i64,
                        note,
                    ],
                )?;
            }

            let before_snapshot = before
                .as_ref()
                .map(|(_, expected, backing, native)| AccountingSnapshot {
                    expected_usd: Some(*expected),
                    backing_sats: u64::try_from(*backing).ok(),
                    native_sats: u64::try_from(*native).ok(),
                    live_receiver_sats: u64::try_from(backing.saturating_add(*native)).ok(),
                    amount_msat: Some(amount_msat),
                    ..Default::default()
                })
                .unwrap_or_else(|| AccountingSnapshot {
                    expected_usd: Some(expected_usd),
                    backing_sats: Some(backing_sats_before),
                    native_sats: Some(native_sats_before),
                    live_receiver_sats: Some(backing_sats_before.saturating_add(native_sats_before)),
                    amount_msat: Some(amount_msat),
                    ..Default::default()
                });
            let after_snapshot = AccountingSnapshot {
                expected_usd: Some(expected_usd),
                backing_sats: Some(backing_sats_after),
                native_sats: Some(native_sats_before),
                live_receiver_sats: Some(backing_sats_after.saturating_add(native_sats_before)),
                amount_msat: Some(amount_msat),
                ..Default::default()
            };
            let draft = LedgerEventDraft {
                event_type: "STABILITY_PAYMENT_SENT".to_owned(),
                category: "stability".to_owned(),
                severity: "info".to_owned(),
                status: "pending".to_owned(),
                source: "lsp".to_owned(),
                completeness: LedgerCompleteness::Observed,
                occurred_at_ms: Utc::now().timestamp_millis(),
                dedup_key: Some(format!("lsp:stability-payment-sent:{payment_id}")),
                before: Some(before_snapshot),
                after: Some(after_snapshot),
                detail: serde_json::json!({
                    "payment_id": payment_id,
                    "channel_id": channel_id,
                    "user_channel_id": user_channel_id,
                    "counterparty_node_id": counterparty,
                    "direction": direction,
                    "amount_msat": amount_msat,
                    "expected_usd": expected_usd,
                    "before_backing_sats": backing_sats_before,
                    "after_backing_sats": backing_sats_after,
                    "native_sats": native_sats_before,
                    "status": "pending",
                }),
                refs: vec![
                    LedgerRef::new("payment_id", payment_id),
                    LedgerRef::new("channel_id", channel_id),
                    LedgerRef::new("user_channel_id", user_channel_id),
                    LedgerRef::new("node_id", counterparty),
                ],
            };
            let outcome = ledger::append_on_connection(&conn, &draft)?;
            if outcome.inserted {
                mirror = Some((draft, outcome.event_id));
            }
            Ok(true)
        })();
        let committed = finish_transaction(&conn, result);
        if committed.is_ok() {
            if let Some((draft, event_id)) = mirror {
                crate::audit::mirror_committed_ledger_event(&draft, event_id);
            }
        }
        committed
    }

    /// Mark a settlement successful and append its terminal ledger event in the same transaction.
    /// Returns true when this call consumed the pending outcome.
    pub fn mark_settlement_succeeded(
        &self,
        payment_id: &str,
        amount_msat: Option<u64>,
        fee_paid_msat: Option<u64>,
        direction: Option<&str>,
    ) -> SqliteResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row: Option<(String, Option<String>, String)> = tx
            .query_row(
                "SELECT kind, user_channel_id, outcome FROM settlement_payments
                 WHERE payment_id = ?1",
                params![payment_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((kind, user_channel_id, outcome)) = row else {
            tx.commit()?;
            return Ok(false);
        };
        if outcome != "pending" {
            tx.commit()?;
            return Ok(false);
        }
        let updated = tx.execute(
            "UPDATE settlement_payments SET outcome = 'succeeded'
             WHERE payment_id = ?1 AND outcome = 'pending'",
            params![payment_id],
        )?;
        if updated != 1 {
            tx.commit()?;
            return Ok(false);
        }

        let channel: Option<(String, f64, i64, i64)> = match user_channel_id.as_deref() {
            Some(user_channel_id) => tx
                .query_row(
                    "SELECT channel_id, expected_usd, stable_sats, native_sats
                     FROM channels WHERE user_channel_id = ?1",
                    params![user_channel_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?,
            None => None,
        };
        let after = channel.as_ref().map(|(_, expected_usd, backing, native)| {
            AccountingSnapshot {
                expected_usd: Some(*expected_usd),
                backing_sats: u64::try_from(*backing).ok(),
                native_sats: u64::try_from(*native).ok(),
                live_receiver_sats: u64::try_from(backing.saturating_add(*native)).ok(),
                amount_msat,
                fee_msat: fee_paid_msat,
                ..Default::default()
            }
        });
        let mut refs = vec![LedgerRef::new("payment_id", payment_id)];
        if let Some(user_channel_id) = user_channel_id.as_deref() {
            refs.push(LedgerRef::new("user_channel_id", user_channel_id));
        }
        if let Some((channel_id, ..)) = channel.as_ref() {
            refs.push(LedgerRef::new("channel_id", channel_id));
        }
        let event_type = if kind == "stability" {
            "STABILITY_PAYMENT_SETTLED"
        } else {
            "PAYMENT_SETTLED"
        };
        let category = if kind == "stability" {
            "stability"
        } else {
            "payment"
        };
        let draft = LedgerEventDraft {
            event_type: event_type.to_owned(),
            category: category.to_owned(),
            severity: "info".to_owned(),
            status: "completed".to_owned(),
            source: "lsp".to_owned(),
            completeness: LedgerCompleteness::Observed,
            occurred_at_ms: Utc::now().timestamp_millis(),
            dedup_key: Some(format!("lsp:{}:{payment_id}", event_type.to_ascii_lowercase())),
            before: None,
            after,
            detail: serde_json::json!({
                "payment_id": payment_id,
                "user_channel_id": user_channel_id,
                "channel_id": channel.as_ref().map(|row| row.0.as_str()),
                "settlement_kind": kind,
                "amount_msat": amount_msat,
                "fee_paid_msat": fee_paid_msat,
                "direction": direction,
                "status": "completed",
            }),
            refs,
        };
        let ledger_outcome = ledger::append_on_connection(&tx, &draft)?;
        tx.commit()?;
        if ledger_outcome.inserted {
            crate::audit::mirror_committed_ledger_event(&draft, ledger_outcome.event_id);
        }
        Ok(true)
    }

    /// Consume a failed outbound stability settlement and conditionally restore its allocation.
    /// The channel update compares the optimistic backing, native sats, and expected USD snapshot:
    /// if a newer trade, sync, or settlement changed the allocation, that newer state wins. The
    /// outcome is still consumed so a replay cannot accidentally roll back a future allocation.
    pub fn rollback_failed_stability_settlement(
        &self,
        payment_id: &str,
    ) -> SqliteResult<Option<StabilityRollback>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT user_channel_id, backing_sats_before, backing_sats_after,
                        native_sats_before, expected_usd, last_stability_payment_before, outcome
                 FROM settlement_payments
                 WHERE payment_id = ?1 AND kind = 'stability'",
                params![payment_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<f64>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((Some(user_channel_id), Some(before), Some(after), Some(native_before), Some(expected_usd), last_before, outcome)) = row else {
            return Ok(None);
        };
        if outcome != "pending"
            || before < 0
            || after < 0
            || native_before < 0
            || !expected_usd.is_finite()
            || expected_usd < 0.0
        {
            return Ok(None);
        }

        let channel_before: Option<(String, f64, i64, i64)> = tx
            .query_row(
                "SELECT channel_id, expected_usd, stable_sats, native_sats
                 FROM channels WHERE user_channel_id = ?1",
                params![user_channel_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;

        let consumed = tx.execute(
            "UPDATE settlement_payments SET outcome = 'failed'
             WHERE payment_id = ?1 AND outcome = 'pending'",
            params![payment_id],
        )?;
        if consumed != 1 {
            tx.commit()?;
            return Ok(None);
        }
        let applied = tx.execute(
            "UPDATE channels
             SET stable_sats = ?1, native_sats = ?2, updated_at = strftime('%s', 'now')
             WHERE user_channel_id = ?3 AND stable_sats = ?4
                   AND native_sats = ?2 AND expected_usd = ?5",
            params![before, native_before, user_channel_id, after, expected_usd],
        )? == 1;
        let channel_after: Option<(String, f64, i64, i64)> = tx
            .query_row(
                "SELECT channel_id, expected_usd, stable_sats, native_sats
                 FROM channels WHERE user_channel_id = ?1",
                params![user_channel_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let snapshot = |row: &(String, f64, i64, i64)| AccountingSnapshot {
            expected_usd: Some(row.1),
            backing_sats: u64::try_from(row.2).ok(),
            native_sats: u64::try_from(row.3).ok(),
            live_receiver_sats: u64::try_from(row.2.saturating_add(row.3)).ok(),
            ..Default::default()
        };
        let event_type = if applied {
            "STABILITY_PAYMENT_ROLLED_BACK"
        } else {
            "STABILITY_PAYMENT_ROLLBACK_SKIPPED"
        };
        let mut refs = vec![
            LedgerRef::new("payment_id", payment_id),
            LedgerRef::new("user_channel_id", &user_channel_id),
        ];
        if let Some((channel_id, ..)) = channel_after.as_ref().or(channel_before.as_ref()) {
            refs.push(LedgerRef::new("channel_id", channel_id));
        }
        let draft = LedgerEventDraft {
            event_type: event_type.to_owned(),
            category: "stability".to_owned(),
            severity: "error".to_owned(),
            status: "failed".to_owned(),
            source: "lsp".to_owned(),
            completeness: LedgerCompleteness::Observed,
            occurred_at_ms: Utc::now().timestamp_millis(),
            dedup_key: Some(format!("lsp:stability-payment-failure:{payment_id}")),
            before: channel_before.as_ref().map(snapshot),
            after: channel_after.as_ref().map(snapshot),
            detail: serde_json::json!({
                "payment_id": payment_id,
                "user_channel_id": user_channel_id,
                "backing_sats_before": before,
                "backing_sats_after": after,
                "restored_native_sats": native_before,
                "expected_usd": expected_usd,
                "database_restored": applied,
                "status": "failed",
            }),
            refs,
        };
        let ledger_outcome = ledger::append_on_connection(&tx, &draft)?;
        tx.commit()?;
        if ledger_outcome.inserted {
            crate::audit::mirror_committed_ledger_event(&draft, ledger_outcome.event_id);
        }

        Ok(Some(StabilityRollback {
            user_channel_id,
            backing_sats_before: before as u64,
            backing_sats_after: after as u64,
            native_sats_before: native_before as u64,
            expected_usd,
            last_stability_payment_before: last_before.unwrap_or(0),
            applied,
        }))
    }

    /// Record a forward and its ledger row in one transaction. The fingerprint marker cannot
    /// survive without the event, so a transient ledger failure remains retryable on reconnect.
    pub fn append_forwarded_event_if_unseen(
        &self,
        fingerprint: &str,
        draft: &LedgerEventDraft,
    ) -> SqliteResult<bool> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO forwarded_seen (fingerprint) VALUES (?1)",
            params![fingerprint],
        )?;
        if inserted == 0 {
            tx.commit()?;
            return Ok(false);
        }
        let outcome = ledger::append_on_connection(&tx, draft)?;
        tx.commit()?;
        if outcome.inserted {
            crate::audit::mirror_committed_ledger_event(draft, outcome.event_id);
        }
        Ok(outcome.inserted)
    }

    pub fn settlement_exists(&self, payment_id: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM settlement_payments WHERE payment_id = ?1)",
            params![payment_id],
            |row| row.get(0),
        )
    }

    /// Pending protocol payments whose terminal LDK outcome still needs to be persisted.
    pub fn list_pending_settlements(&self) -> SqliteResult<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT payment_id, kind FROM settlement_payments
             WHERE outcome = 'pending' ORDER BY recorded_at ASC, payment_id ASC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Return the stored `user_channel_id` for a payment_id, or None if absent/NULL/not found.
    pub fn get_settlement_channel(&self, payment_id: &str) -> SqliteResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT user_channel_id FROM settlement_payments WHERE payment_id = ?1",
        )?;
        let mut rows = stmt.query(params![payment_id])?;
        if let Some(row) = rows.next()? {
            Ok(row.get::<_, Option<String>>(0)?)
        } else {
            Ok(None)
        }
    }

    /// List recorded settlements as (payment_id, kind) pairs, oldest first.
    pub fn list_settlements(&self) -> SqliteResult<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT payment_id, kind FROM settlement_payments ORDER BY recorded_at ASC")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }
}

// =============================================================================
// Record Types
// =============================================================================

#[derive(Debug, Clone)]
pub struct ChannelRecord {
    pub channel_id: String,
    pub user_channel_id: String,
    pub expected_usd: f64,
    pub note: Option<String>,
    pub backing_sats: u64,
    pub native_sats: u64,
}

#[derive(Debug, Clone)]
pub struct TradeRecord {
    pub id: i64,
    pub channel_id: String,
    pub action: String,
    pub amount_usd: f64,
    pub amount_btc: f64,
    pub btc_price: f64,
    pub fee_usd: f64,
    pub payment_id: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub trade_id: Option<String>,
    pub fee_msat: u64,
    pub fee_status: String,
    pub failure_code: Option<String>,
    pub failure_reason: Option<String>,
    pub expires_at: Option<i64>,
    pub resolved_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PriceRecord {
    pub id: i64,
    pub price: f64,
    pub source: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub struct PaymentRecord {
    pub id: i64,
    pub payment_id: Option<String>,
    pub payment_type: String,
    pub direction: String,
    pub amount_msat: u64,
    pub amount_usd: Option<f64>,
    pub btc_price: Option<f64>,
    pub counterparty: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub fee_msat: u64,
    pub txid: Option<String>,
    pub address: Option<String>,
    pub confirmations: u32,
}

#[derive(Debug, Clone)]
pub struct OnchainTxRecord {
    pub id: i64,
    pub txid: String,
    pub direction: String,
    pub amount_sats: u64,
    pub address: Option<String>,
    pub btc_price: Option<f64>,
    pub status: String,
    pub confirmations: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct DailyPriceRecord {
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: Option<f64>,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.conn.lock().is_ok());
    }

    #[test]
    fn test_record_and_list_settlements() {
        let db = Database::open_in_memory().unwrap();
        db.record_settlement("pay_a", "stability").unwrap();
        db.record_settlement("pay_b", "sync").unwrap();
        db.record_settlement("pay_a", "stability").unwrap(); // duplicate is a no-op
        let list = db.list_settlements().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&("pay_a".to_string(), "stability".to_string())));
        assert!(list.contains(&("pay_b".to_string(), "sync".to_string())));
        assert!(db.settlement_exists("pay_a").unwrap());
        assert!(!db.settlement_exists("missing").unwrap());
        assert_eq!(db.list_pending_settlements().unwrap().len(), 2);
    }

    #[test]
    fn test_settlement_channel_round_trip() {
        let db = Database::open_in_memory().unwrap();
        // with-channel variant stores and retrieves user_channel_id
        db.record_settlement_with_channel("pmt1", "stability", "12345").unwrap();
        assert_eq!(db.get_settlement_channel("pmt1").unwrap(), Some("12345".to_string()));
        // absent key returns None
        assert_eq!(db.get_settlement_channel("absent").unwrap(), None);
        // plain record_settlement leaves user_channel_id as NULL (returns None)
        db.record_settlement("pmt2", "sync").unwrap();
        assert_eq!(db.get_settlement_channel("pmt2").unwrap(), None);
    }

    #[test]
    fn failed_stability_settlement_rolls_back_only_its_optimistic_state() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel", "user-channel", 50.0, 50_000, 50_000, None)
            .unwrap();
        assert!(db
            .record_stability_settlement_with_rollback(
                "payment", "user-channel", "channel", 50_000, 62_500, 50_000, 50.0, 17,
                12_500_000, "lsp_to_user", "counterparty", None,
            )
            .unwrap());
        db.save_channel("channel", "user-channel", 50.0, 62_500, 50_000, None)
            .unwrap();

        let rollback = db
            .rollback_failed_stability_settlement("payment")
            .unwrap()
            .unwrap();
        assert!(rollback.applied);
        assert_eq!(rollback.backing_sats_before, 50_000);
        assert_eq!(rollback.backing_sats_after, 62_500);
        assert_eq!(rollback.last_stability_payment_before, 17);
        let channel = db.load_channel("user-channel").unwrap().unwrap();
        assert_eq!(channel.backing_sats, 50_000);
        assert_eq!(channel.native_sats, 50_000);
        assert!(db
            .rollback_failed_stability_settlement("payment")
            .unwrap()
            .is_none());
    }

    #[test]
    fn failed_stability_settlement_does_not_overwrite_newer_allocation() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel", "user-channel", 50.0, 50_000, 50_000, None)
            .unwrap();
        db.record_stability_settlement_with_rollback(
            "payment", "user-channel", "channel", 50_000, 62_500, 50_000, 50.0, 0,
            12_500_000, "lsp_to_user", "counterparty", None,
        )
        .unwrap();
        db.save_channel("channel", "user-channel", 55.0, 70_000, 30_000, None)
            .unwrap();

        let rollback = db
            .rollback_failed_stability_settlement("payment")
            .unwrap()
            .unwrap();
        assert!(!rollback.applied);
        let channel = db.load_channel("user-channel").unwrap().unwrap();
        assert_eq!(channel.backing_sats, 70_000);
        assert_eq!(channel.native_sats, 30_000);
    }

    #[test]
    fn successful_stability_settlement_cannot_be_rolled_back() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel", "user-channel", 50.0, 62_500, 50_000, None)
            .unwrap();
        db.record_stability_settlement_with_rollback(
            "payment", "user-channel", "channel", 50_000, 62_500, 50_000, 50.0, 0,
            12_500_000, "lsp_to_user", "counterparty", None,
        )
        .unwrap();
        assert!(db
            .mark_settlement_succeeded(
                "payment",
                Some(12_500_000),
                Some(1_000),
                Some("outbound"),
            )
            .unwrap());
        assert!(db
            .rollback_failed_stability_settlement("payment")
            .unwrap()
            .is_none());
        assert_eq!(
            db.load_channel("user-channel")
                .unwrap()
                .unwrap()
                .backing_sats,
            62_500
        );
        let terminal = db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("user-channel".to_owned()),
                status: Some("completed".to_owned()),
                limit: 20,
                ..Default::default()
            })
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.event_type == "STABILITY_PAYMENT_SETTLED")
            .unwrap();
        assert_eq!(terminal.detail["payment_id"], "payment");
    }

    #[test]
    fn splice_reconciliation_direction_and_state_commit_once() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel(
            "channel-before",
            "user-channel",
            31.4424,
            47_615,
            44_407,
            None,
        )
        .unwrap();
        db.record_payment(
            None,
            "splice_out",
            "sent",
            90_094_000,
            Some(59.34),
            Some(65_872.5),
            None,
            "pending",
            None,
            Some("tb1qexample"),
        )
        .unwrap();
        assert_eq!(db.set_pending_splice_out_txid("splice-txid").unwrap(), 1);
        assert_eq!(
            db.get_splice_direction("splice-txid").unwrap().as_deref(),
            Some("out")
        );

        assert!(db
            .persist_splice_reconciliation(
                "splice-txid",
                "channel-after",
                "user-channel",
                1.224708075,
                1_742,
                0,
                None,
            )
            .unwrap());
        assert!(!db
            .persist_splice_reconciliation(
                "splice-txid",
                "channel-after",
                "user-channel",
                0.0,
                0,
                1_742,
                None,
            )
            .unwrap());

        let channel = db.load_channel("user-channel").unwrap().unwrap();
        assert!((channel.expected_usd - 1.224708075).abs() < 1e-9);
        assert_eq!(channel.backing_sats, 1_742);
        assert_eq!(channel.native_sats, 0);
        assert!(db.is_splice_stable_reconciled("splice-txid").unwrap());

        db.record_payment(
            None,
            "splice_out",
            "sent",
            1_000,
            None,
            None,
            None,
            "pending",
            None,
            None,
        )
        .unwrap();
        assert_eq!(db.set_pending_splice_out_txid("rollback-txid").unwrap(), 1);
        assert!(db
            .persist_splice_reconciliation(
                "rollback-txid",
                "missing-channel",
                "missing-user-channel",
                0.0,
                0,
                0,
                None,
            )
            .is_err());
        assert!(!db
            .is_splice_stable_reconciled("rollback-txid")
            .unwrap());
    }

    #[test]
    fn forwarded_seen_dedups_and_fingerprint_is_stable() {
        let db = Database::open_in_memory().unwrap();
        let fp = forward_fingerprint("aa", "bb", Some(1000), Some(7));
        assert_eq!(fp, forward_fingerprint("aa", "bb", Some(1000), Some(7)));
        assert_ne!(fp, forward_fingerprint("aa", "bb", Some(1001), Some(7)));
        let draft = LedgerEventDraft::from_audit_event(
            "PAYMENT_FORWARDED",
            serde_json::json!({"prev_channel_id": "aa", "next_channel_id": "bb"}),
        );
        assert!(db.append_forwarded_event_if_unseen(&fp, &draft).unwrap());
        assert!(!db.append_forwarded_event_if_unseen(&fp, &draft).unwrap());
    }

    #[test]
    fn forwarded_marker_rolls_back_when_ledger_append_fails() {
        let db = Database::open_in_memory().unwrap();
        let fingerprint = forward_fingerprint("aa", "bb", Some(1_000), Some(7));
        let draft = LedgerEventDraft::from_audit_event(
            "PAYMENT_FORWARDED_BACKFILL",
            serde_json::json!({
                "prev_channel_id": "aa",
                "next_channel_id": "bb",
                "outbound_amount_msat": 1_000,
                "total_fee_msat": 7,
            }),
        );
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER inject_forward_ledger_failure
                 BEFORE INSERT ON ledger_event_refs
                 BEGIN SELECT RAISE(ABORT, 'injected forward ledger failure'); END;",
            )
            .unwrap();

        assert!(db
            .append_forwarded_event_if_unseen(&fingerprint, &draft)
            .is_err());
        let conn = db.conn.lock().unwrap();
        let marker_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM forwarded_seen", [], |row| row.get(0))
            .unwrap();
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM ledger_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(marker_count, 0);
        assert_eq!(event_count, 0);
    }

    #[test]
    fn test_save_and_load_channel() {
        let db = Database::open_in_memory().unwrap();

        // backing_sats = 100_000 (backing $100 at some price), native_sats = 50_000
        db.save_channel(
            "test_channel_123",
            "uch_123",
            100.0,
            100_000,
            50_000,
            Some("test note"),
        )
        .unwrap();

        let loaded = db.load_channel("uch_123").unwrap().unwrap();
        assert_eq!(loaded.channel_id, "test_channel_123");
        assert_eq!(loaded.user_channel_id, "uch_123");
        assert!((loaded.expected_usd - 100.0).abs() < 0.001);
        assert_eq!(loaded.backing_sats, 100_000);
        assert_eq!(loaded.native_sats, 50_000);
        assert_eq!(loaded.note, Some("test note".to_string()));
    }

    #[test]
    fn sync_versions_are_monotonic_and_persisted() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 10.0, 10_000, 5_000, None)
            .unwrap();

        assert_eq!(db.next_sync_version("user-channel-1").unwrap(), 1);
        assert_eq!(db.next_sync_version("user-channel-1").unwrap(), 2);
        assert_eq!(db.get_sync_version("user-channel-1").unwrap(), Some(2));
        assert!(db.next_sync_version("missing").is_err());
    }

    #[test]
    fn inbound_sync_replay_cannot_overwrite_newer_state() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 10.0, 10_000, 5_000, None)
            .unwrap();

        assert!(db
            .apply_sync_if_newer("user-channel-1", 2, 20.0, 20_000, 4_000)
            .unwrap());
        assert!(!db
            .apply_sync_if_newer("user-channel-1", 2, 12.0, 12_000, 1_000)
            .unwrap());
        assert!(!db
            .apply_sync_if_newer("user-channel-1", 1, 11.0, 11_000, 2_000)
            .unwrap());
        assert!(!db
            .apply_sync_if_newer("user-channel-1", 3, 30.0, u64::MAX, 0)
            .unwrap());

        let channel = db.load_channel("user-channel-1").unwrap().unwrap();
        assert_eq!(channel.expected_usd, 20.0);
        assert_eq!(channel.backing_sats, 20_000);
        assert_eq!(channel.native_sats, 4_000);
        assert_eq!(db.get_sync_version("user-channel-1").unwrap(), Some(2));
    }

    #[test]
    fn inbound_sync_for_missing_channel_is_an_error() {
        let db = Database::open_in_memory().unwrap();
        assert!(matches!(
            db.apply_sync_if_newer("missing", 1, 10.0, 10_000, 5_000),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
    }

    #[test]
    fn lsp_trade_decision_is_atomic_deduplicated_and_retried_with_backoff() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel", "user", 10.0, 10_000, 5_000, None)
            .unwrap();
        let version = db.candidate_sync_version("user").unwrap();
        assert!(db
            .persist_trade_acceptance(
                "inbound-payment",
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                "channel",
                "user",
                Some("wallet-user"),
                "counterparty",
                20.0,
                20_000,
                4_000,
                Some(100_000.0),
                100_000.0,
                Some(0.0),
                version,
                "signed-envelope",
            )
            .unwrap());
        assert_eq!(db.get_sync_version("user").unwrap(), Some(1));
        let channel = db.load_channel("user").unwrap().unwrap();
        assert_eq!(channel.expected_usd, 20.0);
        assert_eq!(channel.backing_sats, 20_000);
        let decision_ids: (String, Option<String>) = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT user_channel_id, remote_user_channel_id
                 FROM trade_decisions WHERE inbound_payment_id = 'inbound-payment'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(decision_ids, ("user".to_owned(), Some("wallet-user".to_owned())));
        let trade_event = db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("inbound-payment".to_owned()),
                limit: 20,
                ..Default::default()
            })
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.event_type == "TRADE_APPLIED")
            .expect("accepted allocation and ledger event must commit together");
        assert_eq!(trade_event.before.unwrap().expected_usd, Some(10.0));
        assert_eq!(trade_event.after.unwrap().expected_usd, Some(20.0));
        assert!(!db
            .persist_trade_acceptance(
                "inbound-payment",
                None,
                "channel",
                "user",
                None,
                "counterparty",
                99.0,
                99_000,
                0,
                None,
                100_000.0,
                None,
                2,
                "other-envelope",
            )
            .unwrap());
        assert_eq!(db.load_channel("user").unwrap().unwrap().expected_usd, 20.0);

        let due = db.due_trade_responses(i64::MAX, u32::MAX, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].response_amount_msat, 1);
        db.mark_trade_response_in_flight("inbound-payment", "response-payment")
            .unwrap();
        assert!(matches!(
            db.mark_trade_response_in_flight("inbound-payment", "response-payment-two"),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
        assert!(matches!(
            db.mark_trade_response_in_flight("missing-payment", "response-payment"),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
        assert!(db
            .due_trade_responses(i64::MAX, u32::MAX, 10)
            .unwrap()
            .is_empty());
        assert!(db
            .mark_trade_response_payment_failed("response-payment", 100)
            .unwrap());
        assert!(db
            .due_trade_responses(104, u32::MAX, 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            db.due_trade_responses(105, u32::MAX, 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(Database::trade_response_delay_secs(1, "response-a"), 5);
        assert!((10..=12).contains(&Database::trade_response_delay_secs(2, "response-a")));
        assert_eq!(
            Database::trade_response_delay_secs(u32::MAX, "response-a"),
            60 * 60
        );
    }

    #[test]
    fn lsp_trade_acceptance_rolls_back_when_ledger_append_fails() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel", "user", 10.0, 10_000, 5_000, None)
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_trade_ledger
                 BEFORE INSERT ON ledger_events
                 WHEN NEW.event_type = 'TRADE_APPLIED'
                 BEGIN
                    SELECT RAISE(ABORT, 'injected ledger failure');
                 END;",
            )
            .unwrap();

        assert!(db
            .persist_trade_acceptance(
                "inbound-payment",
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                "channel",
                "user",
                Some("wallet-user"),
                "counterparty",
                20.0,
                20_000,
                4_000,
                Some(100_000.0),
                100_000.0,
                Some(0.0),
                1,
                "signed-envelope",
            )
            .is_err());

        let channel = db.load_channel("user").unwrap().unwrap();
        assert_eq!(channel.expected_usd, 10.0);
        assert_eq!(db.get_sync_version("user").unwrap(), Some(0));
        assert!(!db.trade_decision_exists("inbound-payment").unwrap());
    }

    #[test]
    fn lsp_rejection_uses_nominal_response_and_reused_trade_id_is_detected() {
        let db = Database::open_in_memory().unwrap();
        let trade_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert!(db
            .persist_trade_rejection(
                "payment-one",
                Some(trade_id),
                "channel",
                "user",
                Some("wallet-user"),
                "counterparty",
                "invalid_fee",
                "wrong fee",
                "signed-rejection",
            )
            .unwrap());
        let response = db
            .due_trade_responses(i64::MAX, u32::MAX, 10)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(response.response_amount_msat, 1);
        assert!(db
            .trade_id_seen_on_other_payment(trade_id, "payment-two")
            .unwrap());
        assert!(!db
            .trade_id_seen_on_other_payment(trade_id, "payment-one")
            .unwrap());
    }

    #[test]
    fn exhausted_trade_response_is_atomically_abandoned_and_replayable() {
        const MAX_ATTEMPTS: u32 = 3;
        let db = Database::open_in_memory().unwrap();
        let trade_id = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        assert!(db
            .persist_trade_rejection(
                "exhausted-payment",
                Some(trade_id),
                "channel",
                "user",
                Some("wallet-user"),
                "counterparty",
                "invalid_fee",
                "wrong fee",
                "signed-rejection",
            )
            .unwrap());
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE trade_decisions
                 SET response_attempts = ?1, next_response_attempt_at = 0
                 WHERE inbound_payment_id = 'exhausted-payment'",
                params![MAX_ATTEMPTS],
            )
            .unwrap();

        assert!(db
            .due_trade_responses(i64::MAX, MAX_ATTEMPTS, 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            db.abandon_exhausted_trade_responses(MAX_ATTEMPTS, 123, 10)
                .unwrap(),
            1
        );
        let abandoned: (String, i64, Option<i64>) = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT response_status, response_attempts, resolved_at
                 FROM trade_decisions WHERE inbound_payment_id = 'exhausted-payment'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(abandoned, ("abandoned".to_owned(), 3, Some(123)));
        let dead_letter = db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("exhausted-payment".to_owned()),
                limit: 20,
                ..Default::default()
            })
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.event_type == "TRADE_RESPONSE_ABANDONED")
            .expect("abandoned response and dead-letter event must commit together");
        assert_eq!(dead_letter.status, "failed");
        assert_eq!(dead_letter.detail["response_attempts"], 3);
        assert!(matches!(
            db.mark_trade_response_send_failed("exhausted-payment", 200),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));

        assert!(db
            .requeue_abandoned_trade_response("exhausted-payment", 456)
            .unwrap());
        let requeued: (String, i64, i64, Option<i64>) = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT response_status, response_attempts, next_response_attempt_at, resolved_at
                 FROM trade_decisions WHERE inbound_payment_id = 'exhausted-payment'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(requeued, ("pending".to_owned(), 0, 456, None));
        assert_eq!(
            db.due_trade_responses(456, MAX_ATTEMPTS, 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn abandoned_trade_response_rolls_back_when_dead_letter_append_fails() {
        let db = Database::open_in_memory().unwrap();
        assert!(db
            .persist_trade_rejection(
                "exhausted-payment",
                None,
                "channel",
                "user",
                None,
                "counterparty",
                "internal_error",
                "temporary failure",
                "signed-rejection",
            )
            .unwrap());
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "UPDATE trade_decisions SET response_attempts = 3;
                 CREATE TRIGGER fail_dead_letter_ledger
                 BEFORE INSERT ON ledger_events
                 WHEN NEW.event_type = 'TRADE_RESPONSE_ABANDONED'
                 BEGIN
                    SELECT RAISE(ABORT, 'injected dead-letter failure');
                 END;",
            )
            .unwrap();

        assert!(db.abandon_exhausted_trade_responses(3, 123, 10).is_err());
        let state: (String, Option<i64>) = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT response_status, resolved_at FROM trade_decisions
                 WHERE inbound_payment_id = 'exhausted-payment'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, ("pending".to_owned(), None));
    }

    #[test]
    fn desktop_trade_expires_then_accepts_late_legacy_sync() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel", "user", 10.0, 10_000, 5_000, None)
            .unwrap();
        let trade_id = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let row_id = db
            .record_prepared_trade(
                "channel", trade_id, "buy", 5.0, 0.00005, 100_000.0, 0.05, 50_000, 5.0,
                5_000, 100,
            )
            .unwrap();
        assert_eq!(db.expire_unresolved_trades(100).unwrap(), 1);
        let expired = db
            .get_trade_by_protocol_ids(Some(trade_id), Some("fee-payment"))
            .unwrap()
            .unwrap();
        assert_eq!(expired.status, "expired");
        assert!(!db
            .apply_correlated_sync_if_newer_and_complete_trade(
                "user",
                1,
                6.0,
                6_000,
                9_000,
                Some(row_id),
                Some("fee-payment"),
            )
            .unwrap());
        assert_eq!(db.get_sync_version("user").unwrap(), Some(0));
        assert!(db
            .apply_correlated_sync_if_newer_and_complete_trade(
                "user",
                1,
                5.0,
                5_000,
                10_000,
                Some(row_id),
                None,
            )
            .unwrap());
        let completed = db.get_recent_trades(1).unwrap().pop().unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.payment_id, None);
        assert!(completed.resolved_at.is_some());
    }

    #[test]
    fn desktop_correlated_sync_repairs_missing_payment_id() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel", "user", 10.0, 10_000, 5_000, None)
            .unwrap();
        let trade_id = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let row_id = db
            .record_prepared_trade(
                "channel", trade_id, "buy", 5.0, 0.00005, 100_000.0, 0.05, 50_000, 5.0,
                5_000, 100,
            )
            .unwrap();

        assert!(db
            .apply_correlated_sync_if_newer_and_complete_trade(
                "user",
                1,
                5.0,
                5_000,
                10_000,
                Some(row_id),
                Some("fee-payment"),
            )
            .unwrap());
        let completed = db.get_recent_trades(1).unwrap().pop().unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.payment_id.as_deref(), Some("fee-payment"));
    }

    #[test]
    fn authoritative_newer_sync_overrides_terminal_trade_status() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel", "user", 10.0, 10_000, 5_000, None)
            .unwrap();
        let trade_id = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let row_id = db
            .record_prepared_trade(
                "channel", trade_id, "sell", 5.0, 0.00005, 100_000.0, 0.05, 50_000, 15.0,
                15_000, 100,
            )
            .unwrap();
        assert!(db
            .mark_trade_rejected(
                row_id,
                Some(trade_id),
                "fee-payment",
                "internal_error",
                "temporary rejection",
            )
            .unwrap());

        assert!(db
            .apply_correlated_sync_if_newer_and_complete_trade(
                "user",
                1,
                15.0,
                15_000,
                0,
                Some(row_id),
                Some("fee-payment"),
            )
            .unwrap());
        let completed = db.get_recent_trades(1).unwrap().pop().unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.failure_code, None);
        assert_eq!(completed.failure_reason, None);
        assert_eq!(db.load_channel("user").unwrap().unwrap().expected_usd, 15.0);
    }

    #[test]
    fn desktop_rejection_accepts_missing_trade_id_only_with_exact_payment() {
        let db = Database::open_in_memory().unwrap();
        let trade_id = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        let row_id = db
            .record_prepared_trade(
                "channel", trade_id, "sell", 5.0, 0.00005, 100_000.0, 0.05, 50_000, 14.95,
                14_950, 1_000,
            )
            .unwrap();
        assert!(db.attach_trade_payment_id(row_id, "fee-payment").unwrap());
        assert!(!db
            .mark_trade_rejected(
                row_id,
                None,
                "wrong-payment",
                "invalid_quote",
                "bad quote",
            )
            .unwrap());
        assert!(db
            .mark_trade_rejected(
                row_id,
                None,
                "fee-payment",
                "invalid_quote",
                "bad quote",
            )
            .unwrap());
        assert!(!db
            .mark_trade_rejected(
                row_id,
                Some(trade_id),
                "fee-payment",
                "invalid_quote",
                "bad quote",
            )
            .unwrap());
        let rejected = db.get_recent_trades(1).unwrap().pop().unwrap();
        assert_eq!(rejected.status, "rejected");
        assert_eq!(rejected.fee_status, "paid");
        assert_eq!(rejected.failure_code.as_deref(), Some("invalid_quote"));
    }

    #[test]
    fn test_payment_backing_delta_is_applied_once() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 100.0, 1_000, 0, None)
            .unwrap();

        let first = db
            .record_payment_and_maybe_update_backing(
                Some("payment-1"),
                "stability",
                "received",
                100_000,
                Some(1.0),
                Some(100_000.0),
                "completed",
                Some("user-channel-1"),
                Some(100),
            )
            .unwrap();
        assert!(first.is_new);
        assert_eq!(first.new_backing, Some(1_100));
        assert!(!first.clamped);
        assert_eq!(
            db.load_channel("user-channel-1")
                .unwrap()
                .unwrap()
                .backing_sats,
            1_100
        );

        let duplicate = db
            .record_payment_and_maybe_update_backing(
                Some("payment-1"),
                "stability",
                "received",
                100_000,
                Some(1.0),
                Some(100_000.0),
                "completed",
                Some("user-channel-1"),
                Some(100),
            )
            .unwrap();
        assert!(!duplicate.is_new);
        assert_eq!(duplicate.new_backing, None);
        assert_eq!(
            db.load_channel("user-channel-1")
                .unwrap()
                .unwrap()
                .backing_sats,
            1_100
        );

        let second = db
            .record_payment_and_maybe_update_backing(
                Some("payment-2"),
                "stability",
                "received",
                50_000,
                Some(0.5),
                Some(100_000.0),
                "completed",
                Some("user-channel-1"),
                Some(50),
            )
            .unwrap();
        assert!(second.is_new);
        assert_eq!(second.new_backing, Some(1_150));
        assert_eq!(
            db.load_channel("user-channel-1")
                .unwrap()
                .unwrap()
                .backing_sats,
            1_150
        );
    }

    #[test]
    fn test_payment_backing_delta_clamps_at_zero() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 100.0, 1_000, 0, None)
            .unwrap();

        let result = db
            .record_payment_and_maybe_update_backing(
                Some("payment-neg"),
                "stability",
                "sent",
                100_000,
                Some(1.0),
                Some(100_000.0),
                "completed",
                Some("user-channel-1"),
                Some(-5_000),
            )
            .unwrap();
        assert!(result.is_new);
        assert!(result.clamped);
        assert_eq!(result.new_backing, Some(0));
        assert_eq!(
            db.load_channel("user-channel-1")
                .unwrap()
                .unwrap()
                .backing_sats,
            0
        );
    }

    #[test]
    fn failed_stability_payment_restores_its_optimistic_backing() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 100.0, 120_000, 20_000, None)
            .unwrap();
        db.record_pending_stability_payment(
            "stability-1",
            20_000_000,
            Some(20.0),
            100_000.0,
            "counterparty",
            "channel-1",
            "user-channel-1",
            100.0,
            120_000,
            100_000,
            20_000,
            None,
        )
        .unwrap();
        assert_eq!(
            db.load_channel("user-channel-1")
                .unwrap()
                .unwrap()
                .backing_sats,
            100_000
        );

        let rollback = db
            .fail_pending_stability_payment("stability-1")
            .unwrap()
            .unwrap();
        assert!(rollback.restored);
        assert_eq!(rollback.backing_sats_before, Some(120_000));
        assert_eq!(rollback.backing_sats_after, Some(100_000));
        assert_eq!(
            db.load_channel("user-channel-1")
                .unwrap()
                .unwrap()
                .backing_sats,
            120_000
        );
        assert_eq!(db.get_recent_payments(1).unwrap()[0].status, "failed");
        assert!(db
            .fail_pending_stability_payment("stability-1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn late_stability_failure_does_not_overwrite_newer_allocation() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 100.0, 120_000, 20_000, None)
            .unwrap();
        db.record_pending_stability_payment(
            "stability-1",
            20_000_000,
            Some(20.0),
            100_000.0,
            "counterparty",
            "channel-1",
            "user-channel-1",
            100.0,
            120_000,
            100_000,
            20_000,
            None,
        )
        .unwrap();
        db.save_channel("channel-1", "user-channel-1", 80.0, 80_000, 40_000, None)
            .unwrap();

        let rollback = db
            .fail_pending_stability_payment("stability-1")
            .unwrap()
            .unwrap();
        assert!(!rollback.restored);
        let channel = db.load_channel("user-channel-1").unwrap().unwrap();
        assert_eq!(channel.expected_usd, 80.0);
        assert_eq!(channel.backing_sats, 80_000);
    }

    #[test]
    fn test_payment_backing_missing_channel_row_is_distinct_error() {
        let db = Database::open_in_memory().unwrap();

        let err = db
            .record_payment_and_maybe_update_backing(
                Some("payment-orphan"),
                "stability",
                "received",
                100_000,
                Some(1.0),
                Some(100_000.0),
                "completed",
                Some("no-such-channel"),
                Some(100),
            )
            .unwrap_err();
        assert!(is_missing_channel_row(&err));
        // Transaction rolled back — the payment row must not exist either,
        // so a retry after recreating the channel row succeeds.
        assert!(!db.payment_exists("payment-orphan").unwrap());
    }

    #[test]
    fn outgoing_reconciliation_commits_once_across_event_replay() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 50.0, 50_000, 10_000, None)
            .unwrap();
        let payment_db_id = db
            .record_payment(
                Some("payment-1"),
                "lightning",
                "sent",
                20_000_000,
                Some(20.0),
                Some(100_000.0),
                None,
                "pending",
                None,
                None,
            )
            .unwrap();

        assert!(db
            .persist_outgoing_reconciliation(
                "payment-1",
                Some(payment_db_id),
                Some(12_000),
                "channel-1",
                "user-channel-1",
                40.0,
                40_000,
                0,
                None,
                Some(100_000.0),
            )
            .unwrap());

        // Simulate restart replay: the in-memory row id is gone and the caller proposes a
        // different state. The durable marker must keep the first committed state unchanged.
        assert!(!db
            .persist_outgoing_reconciliation(
                "payment-1",
                None,
                Some(12_000),
                "channel-1",
                "user-channel-1",
                1.0,
                1,
                1,
                None,
                Some(100_000.0),
            )
            .unwrap());

        let channel = db.load_channel("user-channel-1").unwrap().unwrap();
        assert!((channel.expected_usd - 40.0).abs() < 1e-9);
        assert_eq!(channel.backing_sats, 40_000);
        assert_eq!(channel.native_sats, 0);
        let payments = db.get_recent_payments(1).unwrap();
        assert_eq!(payments[0].status, "completed");
        assert_eq!(payments[0].fee_msat, 12_000);
    }

    #[test]
    fn outgoing_reconciliation_failure_rolls_back_marker_payment_and_channel() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 50.0, 50_000, 10_000, None)
            .unwrap();
        let payment_db_id = db
            .record_payment(
                Some("payment-rollback"),
                "lightning",
                "sent",
                20_000_000,
                Some(20.0),
                Some(100_000.0),
                None,
                "pending",
                None,
                None,
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_outgoing_channel_save
                 BEFORE UPDATE ON channels
                 BEGIN SELECT RAISE(ABORT, 'forced channel save failure'); END;",
            )
            .unwrap();

        assert!(db
            .persist_outgoing_reconciliation(
                "payment-rollback",
                Some(payment_db_id),
                Some(12_000),
                "channel-1",
                "user-channel-1",
                40.0,
                40_000,
                0,
                None,
                Some(100_000.0),
            )
            .is_err());

        let channel = db.load_channel("user-channel-1").unwrap().unwrap();
        assert!((channel.expected_usd - 50.0).abs() < 1e-9);
        assert_eq!(channel.backing_sats, 50_000);
        assert_eq!(channel.native_sats, 10_000);
        let conn = db.conn.lock().unwrap();
        let (status, reconciled): (String, i64) = conn
            .query_row(
                "SELECT status, stable_reconciled FROM payments WHERE id = ?1",
                params![payment_db_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(reconciled, 0);
    }

    #[test]
    fn test_save_channel_preserving_backing() {
        let db = Database::open_in_memory().unwrap();
        // No row yet — signals false so the caller can fall back to save_channel
        assert!(!db
            .save_channel_preserving_backing("ch1", "uch1", 50.0, 10_000, None)
            .unwrap());

        db.save_channel("ch1", "uch1", 50.0, 50_000, 10_000, None)
            .unwrap();
        assert!(db
            .save_channel_preserving_backing("ch1", "uch1", 75.0, 20_000, Some("n"))
            .unwrap());

        let loaded = db.load_channel("uch1").unwrap().unwrap();
        assert!((loaded.expected_usd - 75.0).abs() < 0.001);
        assert_eq!(loaded.backing_sats, 50_000); // untouched
        assert_eq!(loaded.native_sats, 20_000);
        assert_eq!(loaded.note, Some("n".to_string()));
    }

    #[test]
    fn test_channel_upsert() {
        let db = Database::open_in_memory().unwrap();

        db.save_channel("ch1", "uch1", 50.0, 50_000, 10_000, None)
            .unwrap();
        // Same user_channel_id, new channel_id (simulates splice)
        db.save_channel("ch2", "uch1", 100.0, 100_000, 20_000, Some("updated"))
            .unwrap();

        let loaded = db.load_channel("uch1").unwrap().unwrap();
        assert_eq!(loaded.channel_id, "ch2"); // channel_id updated
        assert!((loaded.expected_usd - 100.0).abs() < 0.001);
        assert_eq!(loaded.backing_sats, 100_000);
        assert_eq!(loaded.native_sats, 20_000);
    }

    #[test]
    fn test_mark_channel_closed_excludes_from_load_all() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("ch_a", "uch_a", 10.0, 1000, 0, None)
            .unwrap();
        db.save_channel("ch_b", "uch_b", 20.0, 2000, 0, None)
            .unwrap();
        assert_eq!(db.load_all_channels().unwrap().len(), 2);

        db.mark_channel_closed("uch_a").unwrap();

        let active = db.load_all_channels().unwrap();
        assert_eq!(
            active.len(),
            1,
            "closed channel must be excluded from load_all_channels"
        );
        assert_eq!(active[0].user_channel_id, "uch_b");

        let all = db.load_all_channels_including_closed().unwrap();
        assert_eq!(all.len(), 2, "closed channel must still exist in DB");
    }

    #[test]
    fn test_save_channel_reactivates_closed_row() {
        // A row marked closed by mistake (e.g. transient gRPC blip) must
        // re-activate the next time we save it.
        let db = Database::open_in_memory().unwrap();
        db.save_channel("ch_x", "uch_x", 50.0, 5000, 0, None)
            .unwrap();
        db.mark_channel_closed("uch_x").unwrap();
        assert!(db.load_all_channels().unwrap().is_empty());

        db.save_channel("ch_x", "uch_x", 75.0, 7500, 0, Some("revived"))
            .unwrap();

        let active = db.load_all_channels().unwrap();
        assert_eq!(active.len(), 1, "save_channel must clear closed_at");
        assert!((active[0].expected_usd - 75.0).abs() < 0.001);
        assert_eq!(active[0].note.as_deref(), Some("revived"));
    }

    #[test]
    fn test_mark_channel_closed_is_idempotent() {
        // Calling mark_channel_closed twice must preserve the original
        // close timestamp, not overwrite it.
        let db = Database::open_in_memory().unwrap();
        db.save_channel("ch_y", "uch_y", 10.0, 1000, 0, None)
            .unwrap();

        db.mark_channel_closed("uch_y").unwrap();
        // Read the closed_at value directly so we can compare across calls.
        let conn = db.conn.lock().unwrap();
        let first_ts: i64 = conn
            .query_row(
                "SELECT closed_at FROM channels WHERE user_channel_id = ?1",
                params!["uch_y"],
                |r| r.get(0),
            )
            .unwrap();
        drop(conn);

        // Sleep a beat so the wall clock advances past 1s resolution, then mark closed again.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        db.mark_channel_closed("uch_y").unwrap();

        let conn = db.conn.lock().unwrap();
        let second_ts: i64 = conn
            .query_row(
                "SELECT closed_at FROM channels WHERE user_channel_id = ?1",
                params!["uch_y"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(first_ts, second_ts, "closed_at must not be overwritten");
    }

    #[test]
    fn test_record_and_get_trades() {
        let db = Database::open_in_memory().unwrap();

        db.record_trade(
            "ch1",
            "buy",
            25.0,
            0.00025,
            100000.0,
            0.25,
            75.0,
            Some(75_000),
            Some("pay123"),
            "completed",
        )
        .unwrap();

        db.record_trade(
            "ch1",
            "sell",
            10.0,
            0.000099,
            101000.0,
            0.10,
            60.0,
            Some(59_405),
            Some("pay456"),
            "completed",
        )
        .unwrap();

        let trades = db.get_recent_trades(10).unwrap();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].action, "sell"); // Most recent first
        assert_eq!(trades[1].action, "buy");
    }

    #[test]
    fn pending_full_buy_recovers_exact_zero_allocation() {
        let db = Database::open_in_memory().unwrap();
        db.record_trade(
            "ch1",
            "buy",
            25.0,
            0.00025,
            100_000.0,
            0.25,
            0.0,
            Some(0),
            Some("pending-full-buy"),
            "pending",
        )
        .unwrap();

        let pending = db
            .get_pending_trade_by_payment_id("pending-full-buy")
            .unwrap()
            .expect("new exact-allocation trades must recover even when the target is zero");
        assert_eq!(pending.new_expected_usd, 0.0);
        assert_eq!(pending.new_backing_sats, Some(0));
    }

    #[test]
    fn pending_trade_matches_signed_allocation_and_payment_id_remains_classified() {
        let db = Database::open_in_memory().unwrap();
        let trade_id = db
            .record_trade(
                "ch1",
                "sell",
                10.0,
                0.0001,
                100_000.0,
                0.10,
                60.0,
                Some(60_000),
                Some("trade-payment"),
                "pending",
            )
            .unwrap();

        let matched = db
            .get_pending_trade_by_allocation(60.0, 60_000)
            .unwrap()
            .expect("exact wallet-authored allocation must match");
        assert_eq!(matched.id, trade_id);
        assert!(db
            .get_pending_trade_by_allocation(60.01, 60_000)
            .unwrap()
            .is_none());
        assert!(db.is_trade_payment("trade-payment").unwrap());

        db.update_trade_status(trade_id, "completed").unwrap();
        assert!(db.is_trade_payment("trade-payment").unwrap());
        assert!(db
            .get_pending_trade_by_allocation(60.0, 60_000)
            .unwrap()
            .is_none());
    }

    #[test]
    fn inbound_sync_atomically_completes_matching_trade() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("channel-1", "user-channel-1", 10.0, 10_000, 5_000, None)
            .unwrap();
        let trade_id = db
            .record_trade(
                "user-channel-1",
                "sell",
                10.0,
                0.0001,
                100_000.0,
                0.10,
                20.0,
                Some(20_000),
                Some("trade-payment"),
                "pending",
            )
            .unwrap();

        assert!(db
            .apply_sync_if_newer_and_complete_trade(
                "user-channel-1",
                1,
                20.0,
                20_000,
                4_000,
                Some(trade_id),
            )
            .unwrap());
        assert!(db
            .get_pending_trade_by_payment_id("trade-payment")
            .unwrap()
            .is_none());
        assert!(db.is_trade_payment("trade-payment").unwrap());
        let channel = db.load_channel("user-channel-1").unwrap().unwrap();
        assert_eq!(channel.expected_usd, 20.0);
        assert_eq!(channel.backing_sats, 20_000);
    }

    #[test]
    fn test_record_and_get_price_history() {
        let db = Database::open_in_memory().unwrap();

        db.record_price(100000.0, Some("test")).unwrap();
        db.record_price(100500.0, Some("test")).unwrap();

        let history = db.get_price_history(24).unwrap();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_load_nonexistent_channel() {
        let db = Database::open_in_memory().unwrap();
        let result = db.load_channel("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn user_channel_id_by_channel_id_roundtrips() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("chan_x", "42", 0.0, 0, 0, None).unwrap();
        assert_eq!(db.get_user_channel_id_by_channel_id("chan_x").unwrap(), Some("42".to_string()));
        assert_eq!(db.get_user_channel_id_by_channel_id("missing").unwrap(), None);
    }

    #[test]
    fn ledger_schema_migrates_existing_database_and_keeps_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILENAME);
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE channels (
                channel_id TEXT PRIMARY KEY,
                expected_usd REAL NOT NULL DEFAULT 0.0,
                note TEXT,
                created_at INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0
             )",
            [],
        )
        .unwrap();
        drop(conn);

        let db = Database::open(dir.path()).unwrap();
        let conn = db.conn.lock().unwrap();
        for name in ["ledger_events", "ledger_event_refs", "ledger_metadata"] {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    params![name],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing table {name}");
        }
        let plan = conn
            .prepare("EXPLAIN QUERY PLAN SELECT event_id FROM ledger_event_refs WHERE value = ?1")
            .unwrap()
            .query_map(params!["exact"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<SqliteResult<Vec<_>>>()
            .unwrap()
            .join(" ");
        assert!(plan.contains("idx_ledger_refs_exact"), "query plan was {plan}");
    }

    #[test]
    fn ledger_reference_migration_backfills_plural_identifier_arrays_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM ledger_metadata WHERE key = 'ledger_ref_backfill_v2_plural_ids'",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO ledger_events
                    (event_type, category, severity, status, source, completeness,
                     occurred_at_ms, detail_json)
                 VALUES ('PEER_CONNECTED', 'peer', 'info', 'observed', 'lsp', 'legacy',
                         1, ?1)",
                params![serde_json::json!({
                    "user_channel_ids": ["stable-a", "stable-b"],
                    "counterparty_node_id": "peer-node"
                })
                .to_string()],
            )
            .unwrap();
        }
        drop(db);

        let reopened = Database::open(dir.path()).unwrap();
        for identifier in ["stable-a", "stable-b", "peer-node"] {
            let page = reopened
                .list_ledger_events(&LedgerQuery {
                    identifier: Some(identifier.to_owned()),
                    limit: 10,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(page.events.len(), 1, "missing backfilled ref for {identifier}");
        }
        drop(reopened);

        let reopened_again = Database::open(dir.path()).unwrap();
        let conn = reopened_again.conn.lock().unwrap();
        let ref_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM ledger_event_refs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ref_count, 3);
        let metadata_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_metadata
                 WHERE key = 'ledger_ref_backfill_v2_plural_ids'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(metadata_count, 1);
    }

    #[test]
    fn ledger_multi_reference_exact_lookup_dedup_and_pagination() {
        let db = Database::open_in_memory().unwrap();
        let mut first = LedgerEventDraft::from_audit_event(
            "SPLICE_RECONSTRUCTED",
            serde_json::json!({
                "dedup_key": "splice-1",
                "user_channel_id": "stable-42",
                "channel_id": "physical-old",
                "previous": {"channel_id": "physical-new"},
                "txid": "funding-tx"
            }),
        );
        first.refs.push(LedgerRef::new("channel_id", "physical-new"));
        let one = db.append_ledger_event(&first).unwrap();
        let replay = db.append_ledger_event(&first).unwrap();
        assert!(one.inserted);
        assert!(!replay.inserted);
        assert_eq!(one.event_id, replay.event_id);

        for index in 0..6 {
            db.append_ledger_event(&LedgerEventDraft::from_audit_event(
                "OPERATOR_NOTE_EDITED",
                serde_json::json!({"correlation_id": format!("corr-{index}")}),
            ))
            .unwrap();
        }

        for identifier in ["stable-42", "physical-old", "physical-new", "funding-tx"] {
            let page = db
                .list_ledger_events(&LedgerQuery {
                    identifier: Some(identifier.to_owned()),
                    limit: 50,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(page.events.len(), 1, "lookup failed for {identifier}");
            assert_eq!(page.events[0].id, one.event_id);
        }
        assert!(db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("physical".to_owned()),
                limit: 50,
                ..Default::default()
            })
            .unwrap()
            .events
            .is_empty());
        let filtered = db
            .list_ledger_events(&LedgerQuery {
                category: Some("channel".to_owned()),
                status: Some("observed".to_owned()),
                completeness: Some("reconstructed".to_owned()),
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(filtered.events.len(), 1);
        assert_eq!(filtered.events[0].id, one.event_id);

        let newest = db
            .list_ledger_events(&LedgerQuery { limit: 3, ..Default::default() })
            .unwrap();
        assert_eq!(newest.events.len(), 3);
        assert!(newest.events.windows(2).all(|pair| pair[0].id < pair[1].id));
        let older = db
            .list_ledger_events(&LedgerQuery {
                before: newest.next_cursor,
                limit: 3,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(older.events.len(), 3);
        assert!(older.events.last().unwrap().id < newest.events.first().unwrap().id);
    }

    #[test]
    fn ledger_pagination_follows_timeline_not_insertion_order() {
        let db = Database::open_in_memory().unwrap();
        for occurred_at_ms in [100, 400, 200, 300] {
            let mut draft = LedgerEventDraft::from_audit_event(
                "OPERATOR_NOTE_EDITED",
                serde_json::json!({"correlation_id": format!("at-{occurred_at_ms}")}),
            );
            draft.occurred_at_ms = occurred_at_ms;
            db.append_ledger_event(&draft).unwrap();
        }

        let newest = db
            .list_ledger_events(&LedgerQuery { limit: 2, ..Default::default() })
            .unwrap();
        assert_eq!(
            newest
                .events
                .iter()
                .map(|event| event.occurred_at_ms)
                .collect::<Vec<_>>(),
            vec![300, 400]
        );
        let older = db
            .list_ledger_events(&LedgerQuery {
                before: newest.next_cursor,
                limit: 2,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            older
                .events
                .iter()
                .map(|event| event.occurred_at_ms)
                .collect::<Vec<_>>(),
            vec![100, 200]
        );
    }

    #[test]
    fn ledger_overview_counts_identifier_scope_and_current_channel_state() {
        let db = Database::open_in_memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO channels
                    (channel_id, user_channel_id, expected_usd, stable_sats, native_sats, updated_at)
                 VALUES ('physical', 'stable-42', 99.0, 90, 9, 77)",
                [],
            )
            .unwrap();
        }
        for (index, completeness) in [
            LedgerCompleteness::Observed,
            LedgerCompleteness::Reconstructed,
            LedgerCompleteness::Legacy,
            LedgerCompleteness::Gap,
        ]
        .into_iter()
        .enumerate()
        {
            let mut draft = LedgerEventDraft::from_audit_event(
                "PAYMENT_NOTE",
                serde_json::json!({"user_channel_id": "stable-42"}),
            );
            draft.occurred_at_ms = 1_000 + index as i64;
            draft.completeness = completeness;
            draft.category = if index == 0 { "payment" } else { "system" }.to_owned();
            db.append_ledger_event(&draft).unwrap();
        }
        db.append_ledger_event(&LedgerEventDraft::from_audit_event(
            "PAYMENT_NOTE",
            serde_json::json!({"user_channel_id": "stable-420"}),
        ))
        .unwrap();

        let page = db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("stable-42".to_owned()),
                category: Some("payment".to_owned()),
                limit: 2,
                ..Default::default()
            })
            .unwrap();
        let overview = page.overview;
        assert_eq!(overview.total_events, 4);
        assert_eq!(overview.matching_events, 1);
        assert_eq!(overview.oldest_occurred_at_ms, Some(1_000));
        assert_eq!(overview.newest_occurred_at_ms, Some(1_003));
        assert_eq!(overview.observed_events, 1);
        assert_eq!(overview.reconstructed_events, 1);
        assert_eq!(overview.legacy_events, 1);
        assert_eq!(overview.gap_events, 1);
        assert_eq!(
            overview.latest_accounting_source.as_deref(),
            Some("channels")
        );
        assert_eq!(overview.latest_accounting_at_ms, Some(77_000));
        let state = overview.latest_accounting.unwrap();
        assert_eq!(state.expected_usd, Some(99.0));
        assert_eq!(state.backing_sats, Some(90));
        assert_eq!(state.native_sats, Some(9));
        assert_eq!(state.live_receiver_sats, Some(99));
    }

    #[test]
    fn non_user_identifier_uses_newest_complete_exact_snapshot_only() {
        let db = Database::open_in_memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO channels
                    (channel_id, user_channel_id, expected_usd, stable_sats, native_sats, updated_at)
                 VALUES ('physical', 'stable-42', 99.0, 90, 9, 77)",
                [],
            )
            .unwrap();
        }
        let mut complete = LedgerEventDraft::from_audit_event(
            "CHANNEL_RECONSTRUCTED",
            serde_json::json!({"channel_id": "physical"}),
        );
        complete.occurred_at_ms = 5_000;
        complete.after = Some(AccountingSnapshot {
            expected_usd: Some(12.0),
            backing_sats: Some(12),
            native_sats: Some(3),
            live_receiver_sats: Some(15),
            btc_price: Some(80_000.0),
            ..Default::default()
        });
        db.append_ledger_event(&complete).unwrap();

        let mut newer_partial = LedgerEventDraft::from_audit_event(
            "CHANNEL_NOTE",
            serde_json::json!({"channel_id": "physical"}),
        );
        newer_partial.occurred_at_ms = 6_000;
        newer_partial.after = Some(AccountingSnapshot {
            backing_sats: Some(100),
            ..Default::default()
        });
        db.append_ledger_event(&newer_partial).unwrap();

        let overview = db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("physical".to_owned()),
                limit: 10,
                ..Default::default()
            })
            .unwrap()
            .overview;
        assert_eq!(overview.latest_accounting_source.as_deref(), Some("ledger"));
        assert_eq!(overview.latest_accounting_at_ms, Some(5_000));
        assert_eq!(overview.latest_accounting.unwrap().expected_usd, Some(12.0));
    }

    #[test]
    fn legacy_jsonl_imports_valid_rows_once_and_leaves_malformed_raw() {
        let db = Database::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit_log.txt");
        let raw = concat!(
            "{\"ts\":\"2025-01-01T00:00:00Z\",\"event\":\"CHANNEL_READY\",\"data\":{\"user_channel_id\":\"42\",\"channel_id\":\"aa\"}}\n",
            "not-json\n",
            "{\"ts\":\"2025-01-02T00:00:00Z\",\"event\":\"PAYMENT_SETTLED\",\"data\":{\"payment_id\":\"pay\"}}\n",
            "{\"data\":{}}\n"
        );
        std::fs::write(&path, raw).unwrap();

        let report = db.import_legacy_audit_log(&path).unwrap();
        assert_eq!(report.imported, 2);
        assert_eq!(report.skipped, 2);
        assert!(!report.already_imported);
        let replay = db.import_legacy_audit_log(&path).unwrap();
        assert!(replay.already_imported);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);

        let page = db
            .list_ledger_events(&LedgerQuery {
                completeness: Some("legacy".to_owned()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.events.len(), 2);
        assert!(page.events.iter().all(|event| event.completeness == LedgerCompleteness::Legacy));
        assert_eq!(db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("42".to_owned()),
                limit: 10,
                ..Default::default()
            })
            .unwrap()
            .events
            .len(), 1);
    }

    #[test]
    fn legacy_jsonl_import_skips_invalid_utf8_without_failing() {
        let db = Database::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit_log.txt");
        let mut raw = br#"{"event":"CHANNEL_READY","data":{"user_channel_id":"42"}}
"#
        .to_vec();
        raw.extend_from_slice(b"\xff\xfe\n");
        std::fs::write(&path, raw).unwrap();

        let report = db.import_legacy_audit_log(&path).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn stability_payment_ledger_failure_rolls_back_payment_and_accounting() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER inject_stability_ledger_failure
                 BEFORE INSERT ON ledger_events
                 BEGIN SELECT RAISE(ABORT, 'injected ledger failure'); END;",
            )
            .unwrap();
        }

        assert!(db
            .record_pending_stability_payment(
                "payment", 1_000_000, Some(1.0), 100_000.0, "counterparty", "physical",
                "stable", 10.0, 10_000, 9_000, 5_000, None,
            )
            .is_err());
        assert!(!db.payment_exists("payment").unwrap());
        assert_eq!(db.load_channel("stable").unwrap().unwrap().backing_sats, 10_000);
    }

    #[test]
    fn failed_settlement_ledger_failure_rolls_back_the_rollback() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        db.record_stability_settlement_with_rollback(
            "payment", "stable", "physical", 10_000, 9_000, 5_000, 10.0, 0,
            1_000_000, "lsp_to_user", "counterparty", None,
        )
        .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER inject_rollback_ledger_failure
                 BEFORE INSERT ON ledger_events
                 BEGIN SELECT RAISE(ABORT, 'injected ledger failure'); END;",
            )
            .unwrap();
        }

        assert!(db.rollback_failed_stability_settlement("payment").is_err());
        assert_eq!(db.load_channel("stable").unwrap().unwrap().backing_sats, 9_000);
        let outcome: String = db.conn.lock().unwrap().query_row(
            "SELECT outcome FROM settlement_payments WHERE payment_id = 'payment'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(outcome, "pending");
    }

    #[test]
    fn generic_ledger_append_rolls_back_event_when_reference_insert_fails() {
        let db = Database::open_in_memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER inject_reference_failure
                 BEFORE INSERT ON ledger_event_refs
                 BEGIN SELECT RAISE(ABORT, 'injected reference failure'); END;",
            )
            .unwrap();
        let draft = LedgerEventDraft::from_audit_event(
            "PAYMENT_SUCCESSFUL",
            serde_json::json!({"payment_id": "atomic-payment"}),
        );

        assert!(db.append_ledger_event(&draft).is_err());
        let event_count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM ledger_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(event_count, 0);
    }

    #[test]
    fn reconstructed_snapshots_suppress_only_consecutive_duplicates() {
        let db = Database::open_in_memory().unwrap();
        let snapshot = |state: &str| {
            LedgerEventDraft::from_audit_event(
                "PEER_RECONSTRUCTED",
                serde_json::json!({"node_id": "peer", "state": state}),
            )
        };

        assert!(db
            .append_reconstructed_event_if_changed("peer", "peer", "A", &snapshot("A"))
            .unwrap());
        assert!(!db
            .append_reconstructed_event_if_changed("peer", "peer", "A", &snapshot("A"))
            .unwrap());
        assert!(db
            .append_reconstructed_event_if_changed("peer", "peer", "B", &snapshot("B"))
            .unwrap());
        assert!(db
            .append_reconstructed_event_if_changed("peer", "peer", "A", &snapshot("A"))
            .unwrap());

        let event_count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE event_type = 'PEER_RECONSTRUCTED'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 3);
    }

    #[test]
    fn unchanged_channel_saves_do_not_append_accounting_events() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE channels SET updated_at = 42 WHERE user_channel_id = 'stable'",
                [],
            )
            .unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        assert!(db
            .save_channel_preserving_backing("physical", "stable", 10.0, 5_000, None)
            .unwrap());
        let unchanged_updated_at: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT updated_at FROM channels WHERE user_channel_id = 'stable'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(unchanged_updated_at, 42);
        db.save_channel("physical", "stable", 11.0, 11_000, 4_000, None)
            .unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();

        let event_count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM ledger_events
                 WHERE event_type = 'CHANNEL_ACCOUNTING_STATE_COMMITTED'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 3);
    }

    #[test]
    fn desktop_stability_success_is_terminal_and_atomic() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        db.record_pending_stability_payment(
            "payment",
            1_000_000,
            Some(1.0),
            100_000.0,
            "counterparty",
            "physical",
            "stable",
            10.0,
            10_000,
            9_000,
            5_000,
            None,
        )
        .unwrap();
        assert!(db
            .complete_pending_stability_payment("payment", "hash", Some(25))
            .unwrap());

        let payment_status: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM payments WHERE payment_id = 'payment'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(payment_status, "completed");
        let terminal = db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("stable".to_owned()),
                status: Some("completed".to_owned()),
                limit: 20,
                ..Default::default()
            })
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.event_type == "STABILITY_PAYMENT_SETTLED")
            .unwrap();
        assert_eq!(terminal.detail["payment_id"], "payment");
    }

    #[test]
    fn terminal_ledger_failure_leaves_settlements_pending() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        db.record_pending_stability_payment(
            "desktop-payment",
            1_000_000,
            Some(1.0),
            100_000.0,
            "counterparty",
            "physical",
            "stable",
            10.0,
            10_000,
            9_000,
            5_000,
            None,
        )
        .unwrap();
        db.record_stability_settlement_with_rollback(
            "lsp-payment",
            "stable",
            "physical",
            9_000,
            8_000,
            5_000,
            10.0,
            0,
            1_000_000,
            "outbound",
            "counterparty",
            None,
        )
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER inject_terminal_ledger_failure
                 BEFORE INSERT ON ledger_events
                 WHEN NEW.event_type IN ('PAYMENT_SETTLED', 'STABILITY_PAYMENT_SETTLED')
                 BEGIN SELECT RAISE(ABORT, 'injected terminal failure'); END;",
            )
            .unwrap();

        assert!(db
            .complete_pending_stability_payment("desktop-payment", "hash", None)
            .is_err());
        assert!(db
            .mark_settlement_succeeded("lsp-payment", Some(1_000_000), None, Some("outbound"))
            .is_err());
        let desktop_status: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM payments WHERE payment_id = 'desktop-payment'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let lsp_outcome: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT outcome FROM settlement_payments WHERE payment_id = 'lsp-payment'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(desktop_status, "pending");
        assert_eq!(lsp_outcome, "pending");
    }

    #[test]
    fn injected_ledger_failure_rolls_back_accounting_mutation() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER inject_ledger_failure
                 BEFORE INSERT ON ledger_events
                 BEGIN SELECT RAISE(ABORT, 'injected ledger failure'); END;",
            )
            .unwrap();
        }
        assert!(db
            .save_channel("physical", "stable", 99.0, 99_000, 1_000, None)
            .is_err());
        let channel = db.load_channel("stable").unwrap().unwrap();
        assert_eq!(channel.expected_usd, 10.0);
        assert_eq!(channel.backing_sats, 10_000);
        assert_eq!(channel.native_sats, 5_000);
    }

    #[test]
    fn money_moving_terminal_events_have_channel_refs_and_balanced_snapshots() {
        let db = Database::open_in_memory().unwrap();
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        assert!(db
            .apply_sync_if_newer("stable", 1, 12.0, 12_000, 3_000)
            .unwrap());
        assert!(db
            .record_payment_and_maybe_update_backing(
                Some("stability-payment"),
                "stability",
                "received",
                1_000_000,
                Some(1.0),
                Some(100_000.0),
                "completed",
                Some("stable"),
                Some(1_000),
            )
            .unwrap()
            .is_new);
        assert!(db
            .persist_outgoing_reconciliation(
                "outgoing-payment",
                None,
                Some(25),
                "physical",
                "stable",
                11.0,
                11_000,
                5_000,
                None,
                Some(100_000.0),
            )
            .unwrap());
        db.record_payment(
            Some("splice-tx"),
            "splice_out",
            "sent",
            1_000_000,
            None,
            Some(100_000.0),
            None,
            "pending",
            Some("splice-tx"),
            None,
        )
        .unwrap();
        assert!(db
            .persist_splice_reconciliation(
                "splice-tx",
                "physical-after-splice",
                "stable",
                10.0,
                10_000,
                6_000,
                None,
            )
            .unwrap());
        let events = db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("stable".to_owned()),
                limit: 50,
                ..Default::default()
            })
            .unwrap()
            .events;
        for event_type in [
            "SYNC_V1_APPLIED",
            "STABILITY_PAYMENT_RECORDED",
            "PAYMENT_OUTGOING_RECONCILED",
            "SPLICE_RECONCILED",
        ] {
            let event = events.iter().find(|event| event.event_type == event_type).unwrap();
            assert!(event.refs.iter().any(|reference| reference.role == "user_channel_id"));
            for snapshot in [event.before.as_ref().unwrap(), event.after.as_ref().unwrap()] {
                assert_eq!(
                    snapshot.backing_sats.unwrap() + snapshot.native_sats.unwrap(),
                    snapshot.live_receiver_sats.unwrap(),
                    "unbalanced snapshot for {event_type}"
                );
            }
        }
    }
}
