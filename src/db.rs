//! SQLite database layer for Stable Channels user data.
//!
//! This module provides isolated database operations for storing:
//! - Channel settings (expected_usd, notes)
//! - Trade history
//! - Price history (for charts and analytics)

use rusqlite::{Connection, Result as SqliteResult, params};
use std::path::Path;
use std::sync::{Arc, Mutex};
use chrono::{Utc, Duration as ChronoDuration};

/// Database file name
pub const DB_FILENAME: &str = "stablechannels.db";

/// Thread-safe database handle
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
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
                usd_weight REAL NOT NULL DEFAULT 1.0,
                btc_weight REAL NOT NULL DEFAULT 0.0,
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
                asset_type TEXT NOT NULL DEFAULT 'BTC',
                amount_usd REAL NOT NULL,
                amount_btc REAL NOT NULL DEFAULT 0.0,
                btc_price REAL NOT NULL,
                fee_usd REAL NOT NULL DEFAULT 0.0,
                old_btc_percent INTEGER,
                new_btc_percent INTEGER,
                payment_id TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Migration: Add asset_type column to existing trades table if missing
        let _ = conn.execute(
            "ALTER TABLE trades ADD COLUMN asset_type TEXT NOT NULL DEFAULT 'BTC'",
            [],
        ); // Ignore error if column already exists

        // Migration: Add amount_btc column to existing trades table if missing
        let _ = conn.execute(
            "ALTER TABLE trades ADD COLUMN amount_btc REAL NOT NULL DEFAULT 0.0",
            [],
        ); // Ignore error if column already exists

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

        Ok(())
    }

    // =========================================================================
    // Channel Operations
    // =========================================================================

    /// Save or update channel settings
    pub fn save_channel(
        &self,
        channel_id: &str,
        expected_usd: f64,
        note: Option<&str>,
    ) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO channels (channel_id, usd_weight, btc_weight, expected_usd, note)
             VALUES (?1, 1.0, 0.0, ?2, ?3)
             ON CONFLICT(channel_id) DO UPDATE SET
                expected_usd = ?2,
                note = ?3,
                updated_at = strftime('%s', 'now')",
            params![channel_id, expected_usd, note],
        )?;
        Ok(())
    }

    /// Load channel settings
    pub fn load_channel(&self, channel_id: &str) -> SqliteResult<Option<ChannelRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT channel_id, expected_usd, note
             FROM channels WHERE channel_id = ?1"
        )?;

        let mut rows = stmt.query(params![channel_id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(ChannelRecord {
                channel_id: row.get(0)?,
                expected_usd: row.get(1)?,
                note: row.get(2)?,
            }))
        } else {
            Ok(None)
        }
    }

    // =========================================================================
    // Trade Operations
    // =========================================================================

    /// Record a trade
    pub fn record_trade(
        &self,
        channel_id: &str,
        action: &str,
        asset_type: &str,
        amount_usd: f64,
        amount_btc: f64,
        btc_price: f64,
        fee_usd: f64,
        old_btc_percent: Option<u8>,
        new_btc_percent: Option<u8>,
        payment_id: Option<&str>,
        status: &str,
    ) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO trades (channel_id, action, asset_type, amount_usd, amount_btc, btc_price, fee_usd,
                                 old_btc_percent, new_btc_percent, payment_id, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                channel_id, action, asset_type, amount_usd, amount_btc, btc_price, fee_usd,
                old_btc_percent.map(|v| v as i32),
                new_btc_percent.map(|v| v as i32),
                payment_id, status
            ],
        )?;
        Ok(conn.last_insert_rowid())
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

    /// Get recent trades for a channel
    pub fn get_recent_trades(&self, channel_id: &str, limit: usize) -> SqliteResult<Vec<TradeRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, channel_id, action, asset_type, amount_usd, amount_btc, btc_price, fee_usd,
                    old_btc_percent, new_btc_percent, payment_id, status, created_at
             FROM trades
             WHERE channel_id = ?1
             ORDER BY id DESC
             LIMIT ?2"
        )?;

        let rows = stmt.query_map(params![channel_id, limit as i64], |row| {
            Ok(TradeRecord {
                id: row.get(0)?,
                channel_id: row.get(1)?,
                action: row.get(2)?,
                asset_type: row.get(3)?,
                amount_usd: row.get(4)?,
                amount_btc: row.get(5)?,
                btc_price: row.get(6)?,
                fee_usd: row.get(7)?,
                old_btc_percent: row.get::<_, Option<i32>>(8)?.map(|v| v as u8),
                new_btc_percent: row.get::<_, Option<i32>>(9)?.map(|v| v as u8),
                payment_id: row.get(10)?,
                status: row.get(11)?,
                created_at: row.get(12)?,
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

    /// Get price history for the last N hours
    pub fn get_price_history(&self, hours: u32) -> SqliteResult<Vec<PriceRecord>> {
        let conn = self.conn.lock().unwrap();
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 - (hours as i64 * 3600);

        let mut stmt = conn.prepare(
            "SELECT id, price, source, timestamp
             FROM price_history
             WHERE timestamp > ?1
             ORDER BY timestamp ASC"
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
            .as_secs() as i64 - 86400; // 24 hours ago

        // Get the price closest to 24 hours ago
        let mut stmt = conn.prepare(
            "SELECT price FROM price_history
             WHERE timestamp <= ?1
             ORDER BY timestamp DESC
             LIMIT 1"
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
            .as_secs() as i64 - (days_to_keep as i64 * 86400);

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
    pub fn bulk_insert_daily_prices(&self, prices: &[(String, f64, f64, f64, f64, Option<f64>)]) -> SqliteResult<usize> {
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
             ORDER BY date ASC"
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
        let mut stmt = conn.prepare(
            "SELECT date FROM daily_prices ORDER BY date DESC LIMIT 1"
        )?;

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
        let mut stmt = conn.prepare(
            "SELECT date FROM daily_prices ORDER BY date ASC LIMIT 1"
        )?;

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

    /// Record a payment
    /// payment_type should be "stability" or "manual"
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
    ) -> SqliteResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![payment_id, payment_type, direction, amount_msat as i64, amount_usd, btc_price, counterparty, status],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get recent payments
    pub fn get_recent_payments(&self, limit: usize) -> SqliteResult<Vec<PaymentRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, counterparty, status, created_at
             FROM payments
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
}

// =============================================================================
// Record Types
// =============================================================================

#[derive(Debug, Clone)]
pub struct ChannelRecord {
    pub channel_id: String,
    pub expected_usd: f64,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TradeRecord {
    pub id: i64,
    pub channel_id: String,
    pub action: String,
    pub asset_type: String,
    pub amount_usd: f64,
    pub amount_btc: f64,
    pub btc_price: f64,
    pub fee_usd: f64,
    pub old_btc_percent: Option<u8>,
    pub new_btc_percent: Option<u8>,
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
    fn test_save_and_load_channel() {
        let db = Database::open_in_memory().unwrap();

        db.save_channel("test_channel_123", 100.0, Some("test note"))
            .unwrap();

        let loaded = db.load_channel("test_channel_123").unwrap().unwrap();
        assert_eq!(loaded.channel_id, "test_channel_123");
        assert!((loaded.expected_usd - 100.0).abs() < 0.001);
        assert_eq!(loaded.note, Some("test note".to_string()));
    }

    #[test]
    fn test_channel_upsert() {
        let db = Database::open_in_memory().unwrap();

        db.save_channel("ch1", 50.0, None).unwrap();
        db.save_channel("ch1", 100.0, Some("updated")).unwrap();

        let loaded = db.load_channel("ch1").unwrap().unwrap();
        assert!((loaded.expected_usd - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_record_and_get_trades() {
        let db = Database::open_in_memory().unwrap();

        db.record_trade(
            "ch1", "buy", "BTC", 25.0, 0.00025, 100000.0, 0.25,
            Some(50), Some(75), Some("pay123"), "completed"
        ).unwrap();

        db.record_trade(
            "ch1", "sell", "BTC", 10.0, 0.000099, 101000.0, 0.10,
            Some(75), Some(65), Some("pay456"), "completed"
        ).unwrap();

        let trades = db.get_recent_trades("ch1", 10).unwrap();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].action, "sell"); // Most recent first
        assert_eq!(trades[0].asset_type, "BTC");
        assert_eq!(trades[1].action, "buy");
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
}
