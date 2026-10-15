//! sqlite store for onboarding swap attempts (`onboarding_swaps.db` in the data dir), one row per
//! JIT invoice payment_hash. `provider_token` is the provider credential for the order: it is read
//! only to call the provider and is never part of `SwapRow`'s public projection.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{PayInAsset, SwapState};

pub struct SwapRow {
    pub payment_hash: String,
    pub node_id: String,
    pub bolt11: String,
    pub amount_msat: u64,
    pub pay_in_asset: PayInAsset,
    pub provider_id: String,
    pub provider_order_id: String,
    pub provider_token: String,
    pub deposit_address: String,
    pub deposit_amount: String,
    pub deposit_memo: Option<String>,
    pub provider_state: SwapState,
    /// refund_reason on the refund path, attention_reason in `attention`.
    pub state_reason: Option<String>,
    pub created_at: u64,
    /// Provider deposit deadline (unix seconds).
    pub expires_at: u64,
    pub settled_at: Option<u64>,
    pub refund_address: Option<String>,
}

pub struct SwapStore {
    conn: Mutex<Connection>,
}

const COLUMNS: &str = "payment_hash, node_id, bolt11, amount_msat, pay_in_asset, provider_id, \
    provider_order_id, provider_token, deposit_address, deposit_amount, deposit_memo, \
    provider_state, state_reason, created_at, expires_at, settled_at, refund_address";

impl SwapStore {
    pub fn open(data_dir: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(data_dir.join("onboarding_swaps.db"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS onboarding_swaps (
                payment_hash      TEXT PRIMARY KEY,
                node_id           TEXT NOT NULL,
                bolt11            TEXT NOT NULL,
                amount_msat       INTEGER NOT NULL,
                pay_in_asset      TEXT NOT NULL,
                provider_id       TEXT NOT NULL,
                provider_order_id TEXT NOT NULL,
                provider_token    TEXT NOT NULL,
                deposit_address   TEXT NOT NULL,
                deposit_amount    TEXT NOT NULL,
                deposit_memo      TEXT,
                provider_state    TEXT NOT NULL,
                state_reason      TEXT,
                created_at        INTEGER NOT NULL,
                expires_at        INTEGER NOT NULL,
                settled_at        INTEGER,
                refund_address    TEXT
            )",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn insert(&self, row: &SwapRow) -> rusqlite::Result<()> {
        self.conn().execute(
            &format!("INSERT INTO onboarding_swaps ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)"),
            params![
                row.payment_hash,
                row.node_id,
                row.bolt11,
                row.amount_msat as i64,
                row.pay_in_asset.as_str(),
                row.provider_id,
                row.provider_order_id,
                row.provider_token,
                row.deposit_address,
                row.deposit_amount,
                row.deposit_memo,
                row.provider_state.as_str(),
                row.state_reason,
                row.created_at as i64,
                row.expires_at as i64,
                row.settled_at.map(|t| t as i64),
                row.refund_address,
            ],
        )?;
        Ok(())
    }

    pub fn get(&self, payment_hash: &str) -> rusqlite::Result<Option<SwapRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM onboarding_swaps WHERE payment_hash = ?1"),
                params![payment_hash],
                read_row,
            )
            .optional()
    }

    pub fn update_state(
        &self,
        payment_hash: &str,
        state: SwapState,
        reason: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE onboarding_swaps SET provider_state = ?2, state_reason = ?3 WHERE payment_hash = ?1",
            params![payment_hash, state.as_str(), reason],
        )?;
        Ok(())
    }

    /// Settlement is final: returns false when the row was already settled (or is unknown).
    pub fn mark_settled(&self, payment_hash: &str, now: u64) -> rusqlite::Result<bool> {
        let changed = self.conn().execute(
            "UPDATE onboarding_swaps SET settled_at = ?2 WHERE payment_hash = ?1 AND settled_at IS NULL",
            params![payment_hash, now as i64],
        )?;
        Ok(changed == 1)
    }

    pub fn set_refund_address(&self, payment_hash: &str, address: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE onboarding_swaps SET refund_address = ?2 WHERE payment_hash = ?1",
            params![payment_hash, address],
        )?;
        Ok(())
    }

    /// Rows the poller still owes a provider refresh: not settled, not terminal, not abandoned.
    pub fn pollable(&self, created_after: u64) -> rusqlite::Result<Vec<SwapRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM onboarding_swaps
             WHERE settled_at IS NULL AND created_at >= ?1 ORDER BY created_at"
        ))?;
        let rows = stmt.query_map(params![created_after as i64], read_row)?;
        rows.filter(|r| {
            r.as_ref()
                .map_or(true, |row| !row.provider_state.is_terminal())
        })
        .collect()
    }
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<SwapRow> {
    let enum_err = |i: usize, v: String| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, v.into())
    };
    let asset: String = row.get(4)?;
    let state: String = row.get(11)?;
    Ok(SwapRow {
        payment_hash: row.get(0)?,
        node_id: row.get(1)?,
        bolt11: row.get(2)?,
        amount_msat: row.get::<_, i64>(3)? as u64,
        pay_in_asset: PayInAsset::parse(&asset).ok_or_else(|| enum_err(4, asset.clone()))?,
        provider_id: row.get(5)?,
        provider_order_id: row.get(6)?,
        provider_token: row.get(7)?,
        deposit_address: row.get(8)?,
        deposit_amount: row.get(9)?,
        deposit_memo: row.get(10)?,
        provider_state: SwapState::parse(&state).ok_or_else(|| enum_err(11, state.clone()))?,
        state_reason: row.get(12)?,
        created_at: row.get::<_, i64>(13)? as u64,
        expires_at: row.get::<_, i64>(14)? as u64,
        settled_at: row.get::<_, Option<i64>>(15)?.map(|t| t as u64),
        refund_address: row.get(16)?,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tempfile::tempdir;

    pub(crate) fn sample_row(payment_hash: &str) -> SwapRow {
        SwapRow {
            payment_hash: payment_hash.to_string(),
            node_id: "02node".into(),
            bolt11: "lnbc1".into(),
            amount_msat: 150_000_000,
            pay_in_asset: PayInAsset::UsdtTron,
            provider_id: "ff-io".into(),
            provider_order_id: "ABC123".into(),
            provider_token: "secret-token".into(),
            deposit_address: "T9y".into(),
            deposit_amount: "15.01".into(),
            deposit_memo: None,
            provider_state: SwapState::AwaitingDeposit,
            state_reason: None,
            created_at: 1_000,
            expires_at: 1_600,
            settled_at: None,
            refund_address: None,
        }
    }

    #[test]
    fn insert_get_round_trips_and_rejects_duplicates() {
        let dir = tempdir().unwrap();
        let store = SwapStore::open(dir.path()).unwrap();
        store.insert(&sample_row("aa")).unwrap();
        let row = store.get("aa").unwrap().unwrap();
        assert_eq!(row.pay_in_asset, PayInAsset::UsdtTron);
        assert_eq!(row.provider_token, "secret-token");
        assert_eq!(row.amount_msat, 150_000_000);
        assert!(store.insert(&sample_row("aa")).is_err());
        assert!(store.get("bb").unwrap().is_none());
    }

    #[test]
    fn settlement_is_final_and_stops_polling() {
        let dir = tempdir().unwrap();
        let store = SwapStore::open(dir.path()).unwrap();
        store.insert(&sample_row("aa")).unwrap();
        store.insert(&sample_row("bb")).unwrap();
        store
            .update_state("bb", SwapState::Refunded, Some("underpaid"))
            .unwrap();
        assert_eq!(store.pollable(0).unwrap().len(), 1);

        assert!(store.mark_settled("aa", 2_000).unwrap());
        assert!(!store.mark_settled("aa", 3_000).unwrap());
        assert_eq!(store.get("aa").unwrap().unwrap().settled_at, Some(2_000));
        assert!(store.pollable(0).unwrap().is_empty());
    }

    #[test]
    fn pollable_skips_abandoned_rows() {
        let dir = tempdir().unwrap();
        let store = SwapStore::open(dir.path()).unwrap();
        store.insert(&sample_row("aa")).unwrap();
        assert_eq!(store.pollable(1_000).unwrap().len(), 1);
        assert!(store.pollable(1_001).unwrap().is_empty());
    }
}
