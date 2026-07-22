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
    pub new_expected_usd: f64,
    pub btc_price: f64,
    pub new_backing_sats: Option<u64>,
    pub action: String,
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

        // Forwarded-payment dedup: tracks fingerprints of forwards already audited (live or backfilled)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS forwarded_seen (fingerprint TEXT PRIMARY KEY)",
            [],
        )?;

        Ok(())
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
        Ok(())
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
        Ok(updated > 0)
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
        if sync_version == 0
            || sync_version > i64::MAX as u64
            || !expected_usd.is_finite()
            || expected_usd < 0.0
            || backing_sats > i64::MAX as u64
            || native_sats > i64::MAX as u64
        {
            return Ok(false);
        }
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
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
            let exists: bool = conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM channels WHERE user_channel_id = ?1
                 )",
                params![user_channel_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
        }
        Ok(updated == 1)
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
        conn.execute(
            "UPDATE channels
             SET closed_at = strftime('%s', 'now'),
                 updated_at = strftime('%s', 'now')
             WHERE user_channel_id = ?1 AND closed_at IS NULL",
            params![user_channel_id],
        )?;
        Ok(())
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

    /// Look up a still-pending trade by its payment_id, for restart recovery. Only returns rows
    /// that have either an exact allocation or a non-zero legacy target.
    pub fn get_pending_trade_by_payment_id(
        &self,
        payment_id: &str,
    ) -> SqliteResult<Option<PendingTradeRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, new_expected_usd, btc_price, new_backing_sats, action FROM trades
             WHERE payment_id = ?1 AND status = 'pending'
               AND (new_backing_sats IS NOT NULL OR new_expected_usd > 0.0)
             ORDER BY id DESC LIMIT 1",
            params![payment_id],
            |row| {
                Ok(PendingTradeRow {
                    id: row.get(0)?,
                    new_expected_usd: row.get(1)?,
                    btc_price: row.get(2)?,
                    new_backing_sats: row.get::<_, Option<i64>>(3)?.map(|v| v.max(0) as u64),
                    action: row.get(4)?,
                })
            },
        )
        .optional()
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

    /// Get recent trades across all channels
    pub fn get_recent_trades(&self, limit: usize) -> SqliteResult<Vec<TradeRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, channel_id, action, amount_usd, amount_btc, btc_price, fee_usd,
                    payment_id, status, created_at
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
        let result: SqliteResult<bool> = (|| {
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

            let fee_msat = fee_msat.map(|fee| fee as i64);
            if let Some((id, _)) = payment_row {
                conn.execute(
                    "UPDATE payments SET payment_id = COALESCE(payment_id, ?1),
                                         status = 'completed',
                                         fee_msat = COALESCE(?2, fee_msat),
                                         stable_reconciled = 1
                     WHERE id = ?3",
                    params![event_id, fee_msat, id],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO payments
                        (payment_id, payment_type, direction, amount_msat, btc_price, status,
                         fee_msat, stable_reconciled)
                     VALUES (?1, 'lightning', 'sent', 0, ?2, 'completed', ?3, 1)",
                    params![event_id, btc_price, fee_msat.unwrap_or(0)],
                )?;
            }

            Ok(true)
        })();

        match result {
            Ok(applied) => match conn.execute_batch("COMMIT") {
                Ok(()) => Ok(applied),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(e)
                }
            },
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
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
            let mut new_backing = None;
            let mut clamped = false;
            if let Some(delta) = backing_delta_sats {
                // user_channel_id must be set when a backing update is requested.
                let ucid = user_channel_id.ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "user_channel_id required for backing update".to_string(),
                    )
                })?;
                let current: Option<i64> = conn
                    .query_row(
                        "SELECT stable_sats FROM channels WHERE user_channel_id = ?1",
                        params![ucid],
                        |row| row.get(0),
                    )
                    .optional()?;
                // Distinct missing-channel-row error — see doc comment.
                let current = current.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
                let target = current.saturating_add(delta);
                let updated = target.max(0);
                clamped = target < 0;
                conn.execute(
                    "UPDATE channels SET stable_sats = ?1, updated_at = strftime('%s', 'now') WHERE user_channel_id = ?2",
                    params![updated, ucid],
                )?;
                new_backing = Some(updated);
            }
            Ok(PaymentPersistence {
                is_new: true,
                new_backing,
                clamped,
            })
        })();
        match result {
            Ok(persistence) => match conn.execute_batch("COMMIT") {
                Ok(()) => Ok(persistence),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(e)
                }
            },
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
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
        let result: SqliteResult<bool> = (|| {
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
            Ok(true)
        })();

        match result {
            Ok(applied) => match conn.execute_batch("COMMIT") {
                Ok(()) => Ok(applied),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(e)
                }
            },
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
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

    /// Record a forward's fingerprint. Returns true if newly inserted (not seen before).
    pub fn record_forwarded_seen(&self, fingerprint: &str) -> SqliteResult<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "INSERT OR IGNORE INTO forwarded_seen (fingerprint) VALUES (?1)",
            params![fingerprint],
        )?;
        Ok(n == 1)
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
        assert!(db.record_forwarded_seen(&fp).unwrap());   // first insert -> true
        assert!(!db.record_forwarded_seen(&fp).unwrap());  // repeat -> false
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
}
