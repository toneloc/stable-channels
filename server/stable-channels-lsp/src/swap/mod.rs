//! Stablecoin onboarding swaps (#391): a FixedFloat-compatible exchange receives USDT/USDC and pays
//! the wallet's LSPS2 JIT bolt11, after which onboarding continues unchanged. Native port of
//! OpenReceive's Lightning Swap Connect design (reference only, never a dependency): its pay-in
//! asset table, swap states and refund rules, restricted to the USD stablecoin rails.

pub mod address;
pub mod lsc_uri;
pub mod provider;
pub mod store;

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ldk_node::lightning_invoice::Currency;
use tokio::sync::{Mutex, OnceCell};

use crate::config::SwapConfig;
use provider::{Catalog, FixedFloatClient, ProviderError};
use store::{SwapRow, SwapStore};

/// How long the `/ccies` + rates catalog is reused before refetching. Limits move with the market,
/// so this stays short (OpenReceive's rates cache is minutes, not hours).
const CATALOG_TTL: Duration = Duration::from_secs(300);

/// Swap state, OpenReceive's kernel vocabulary minus `creating_provider_order` (we only persist a
/// row after the provider answered). `Completed` is deliberately NOT terminal: provider
/// completion is not wallet settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapState {
    AwaitingDeposit,
    Confirming,
    Exchanging,
    PayingInvoice,
    Completed,
    Expired,
    RefundRequired,
    RefundPending,
    Refunded,
    Attention,
    Failed,
}

impl SwapState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingDeposit => "awaiting_deposit",
            Self::Confirming => "confirming",
            Self::Exchanging => "exchanging",
            Self::PayingInvoice => "paying_invoice",
            Self::Completed => "completed",
            Self::Expired => "expired",
            Self::RefundRequired => "refund_required",
            Self::RefundPending => "refund_pending",
            Self::Refunded => "refunded",
            Self::Attention => "attention",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }

    /// Terminal states stop provider polling (kernel-tables.json `swap.states[].terminal`).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Expired | Self::Refunded | Self::Attention | Self::Failed
        )
    }

    pub const ALL: [Self; 11] = [
        Self::AwaitingDeposit,
        Self::Confirming,
        Self::Exchanging,
        Self::PayingInvoice,
        Self::Completed,
        Self::Expired,
        Self::RefundRequired,
        Self::RefundPending,
        Self::Refunded,
        Self::Attention,
        Self::Failed,
    ];
}

/// Which way a deposit can be lost on a rail (OpenReceive `SwapDepositRisk`). Stablecoins are
/// never `pinned`: a token address also accepts the chain's native coin and other tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositRisk {
    /// `0x…` is the same string on every EVM chain: wrong network AND wrong token both lose funds.
    ChainAmbiguous,
    /// The address pins the chain, but any other asset sent to it is lost.
    AssetOnly,
}

impl DepositRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChainAmbiguous => "chain_ambiguous",
            Self::AssetOnly => "asset_only",
        }
    }
}

/// The USD stablecoin rails of OpenReceive's pay-in table (kernel-tables.json `swap.pay_in_assets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayInAsset {
    UsdtTron,
    UsdtSol,
    UsdcSol,
    UsdtEth,
    UsdcEth,
}

impl PayInAsset {
    pub const ALL: [Self; 5] = [
        Self::UsdtTron,
        Self::UsdtSol,
        Self::UsdcSol,
        Self::UsdtEth,
        Self::UsdcEth,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsdtTron => "USDT_TRON",
            Self::UsdtSol => "USDT_SOL",
            Self::UsdcSol => "USDC_SOL",
            Self::UsdtEth => "USDT_ETH",
            Self::UsdcEth => "USDC_ETH",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == value)
    }

    pub fn coin(self) -> &'static str {
        match self {
            Self::UsdtTron | Self::UsdtSol | Self::UsdtEth => "USDT",
            Self::UsdcSol | Self::UsdcEth => "USDC",
        }
    }

    /// Kernel network code, matched against provider spellings by `network_matches`.
    pub fn network(self) -> &'static str {
        match self {
            Self::UsdtTron => "TRX",
            Self::UsdtSol | Self::UsdcSol => "SOL",
            Self::UsdtEth | Self::UsdcEth => "ETH",
        }
    }

    pub fn network_label(self) -> &'static str {
        match self {
            Self::UsdtTron => "Tron",
            Self::UsdtSol | Self::UsdcSol => "Solana",
            Self::UsdtEth | Self::UsdcEth => "Ethereum",
        }
    }

    pub fn deposit_risk(self) -> DepositRisk {
        match self.network() {
            "ETH" => DepositRisk::ChainAmbiguous,
            _ => DepositRisk::AssetOnly,
        }
    }

    /// Payer-facing warning (title, text). Every stablecoin rail gets one; the text names the
    /// mistake that is actually reachable on this rail.
    pub fn network_warning(self, deposit_amount: &str) -> (&'static str, String) {
        let emphasis = format!(
            "{} {} on the {} network",
            deposit_amount,
            self.coin(),
            self.network_label()
        );
        let text = match self.deposit_risk() {
            DepositRisk::ChainAmbiguous => format!(
                "Be sure you are sending exactly {emphasis}. This 0x address also exists on other \
                 EVM networks: if you send the wrong currency or send on the wrong network, your \
                 funds will be lost!"
            ),
            DepositRisk::AssetOnly => format!(
                "Be sure you are sending exactly {emphasis}. This address only accepts {} on {}: \
                 if you send any other currency, your funds will be lost!",
                self.coin(),
                self.network_label()
            ),
        };
        ("Wrong currency or network = lost funds", text)
    }
}

fn normalize_network(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Provider network spelling vs kernel network code (OpenReceive `swapNetworkMatches`).
pub fn network_matches(expected: &str, actual: &str) -> bool {
    let actual = normalize_network(actual);
    if actual == normalize_network(expected) {
        return true;
    }
    match expected {
        "TRX" => matches!(actual.as_str(), "TRON" | "TRC20" | "TRC"),
        "ETH" => matches!(actual.as_str(), "ETHEREUM" | "ERC20" | "ERC"),
        "SOL" => actual == "SOLANA",
        _ => false,
    }
}

pub fn is_lightning_network(value: &str) -> bool {
    matches!(
        normalize_network(value).as_str(),
        "LN" | "LIGHTNING" | "LIGHTNINGNETWORK" | "BTCLN" | "BTCBOLT11"
    )
}

/// Configured providers plus the attempt store. Providers are ordered primary, backup; the backup
/// is used only when the primary is unreachable, never to widen the asset list (OpenReceive rule).
pub struct OnboardingSwaps {
    pub store: SwapStore,
    pub providers: Vec<FixedFloatClient>,
    /// Invoices must be on the LSP's own network; a provider would pay any network it supports.
    pub currency: Currency,
    /// This LSP's node id, fetched once from LDK Server: JIT invoices route-hint through it.
    pub lsp_node_id: OnceCell<String>,
    pub rate_limit: std::sync::Mutex<CreateRateLimit>,
    catalog: Mutex<Option<(Instant, String, Catalog)>>,
    /// Serializes create so two concurrent requests for one payment_hash cannot open two orders.
    pub create_lock: Mutex<()>,
}

/// Provider orders each user may open per window, and all users together. A real user opens one
/// or two; the limits only bound what a scripted client can make the provider (and us) store.
const CREATE_WINDOW_SECS: u64 = 3600;
const CREATES_PER_NODE: usize = 5;
const CREATES_GLOBAL: usize = 120;

/// In-memory sliding-window limiter for new provider orders (resets on restart, which is fine:
/// it bounds bursts, not lifetime usage).
pub struct CreateRateLimit {
    window_secs: u64,
    per_node: usize,
    global: usize,
    recent: VecDeque<(u64, String)>,
}

impl CreateRateLimit {
    pub fn new(window_secs: u64, per_node: usize, global: usize) -> Self {
        Self {
            window_secs,
            per_node,
            global,
            recent: VecDeque::new(),
        }
    }

    /// Record one create for `node_id` if both limits allow it.
    pub fn try_acquire(&mut self, node_id: &str, now: u64) -> Result<(), &'static str> {
        let cutoff = now.saturating_sub(self.window_secs);
        while self.recent.front().is_some_and(|(at, _)| *at <= cutoff) {
            self.recent.pop_front();
        }
        if self.recent.len() >= self.global {
            return Err("too many swaps right now, try again later");
        }
        if self.recent.iter().filter(|(_, n)| n == node_id).count() >= self.per_node {
            return Err("too many swaps for this wallet, try again later");
        }
        self.recent.push_back((now, node_id.to_string()));
        Ok(())
    }
}

/// `[node] network` → invoice currency. None for a network the LSP cannot be running on.
pub fn currency_for_network(network: &str) -> Option<Currency> {
    match network {
        "bitcoin" | "mainnet" => Some(Currency::Bitcoin),
        "testnet" | "testnet4" => Some(Currency::BitcoinTestnet),
        "signet" => Some(Currency::Signet),
        "regtest" => Some(Currency::Regtest),
        _ => None,
    }
}

impl OnboardingSwaps {
    pub fn new(store: SwapStore, providers: Vec<FixedFloatClient>, currency: Currency) -> Self {
        Self {
            store,
            providers,
            currency,
            lsp_node_id: OnceCell::new(),
            rate_limit: std::sync::Mutex::new(CreateRateLimit::new(
                CREATE_WINDOW_SECS,
                CREATES_PER_NODE,
                CREATES_GLOBAL,
            )),
            catalog: Mutex::new(None),
            create_lock: Mutex::new(()),
        }
    }

    /// Build from the optional `[swap]` section. `Ok(None)` means the feature is disabled.
    /// Errors never contain the LSC credentials.
    pub fn from_config(
        cfg: Option<&SwapConfig>,
        data_dir: &Path,
        network: &str,
    ) -> Result<Option<Arc<Self>>, String> {
        let Some(cfg) = cfg else { return Ok(None) };
        let currency = currency_for_network(network)
            .ok_or_else(|| format!("[swap] unsupported node network {network:?}"))?;
        let mut providers: Vec<FixedFloatClient> = Vec::new();
        for (name, uri) in [
            ("lsc_uri_primary", Some(&cfg.lsc_uri_primary)),
            ("lsc_uri_backup", cfg.lsc_uri_backup.as_ref()),
        ] {
            let Some(uri) = uri.filter(|u| !u.trim().is_empty()) else {
                continue;
            };
            let conn = lsc_uri::parse_lsc_uri(uri)
                .map_err(|e| format!("[swap] {name} is invalid: {e}"))?;
            if providers.iter().any(|p| p.id == conn.provider_id) {
                return Err(format!("[swap] {name} duplicates another LSC provider id"));
            }
            providers.push(FixedFloatClient::new(
                &conn.provider_id,
                &conn.base_url,
                &conn.key,
                &conn.secret,
            ));
        }
        if providers.is_empty() {
            return Ok(None);
        }
        let store = SwapStore::open(data_dir).map_err(|e| format!("onboarding_swaps.db: {e}"))?;
        Ok(Some(Arc::new(Self::new(store, providers, currency))))
    }

    pub fn provider(&self, id: &str) -> Option<&FixedFloatClient> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// The live catalog and the provider that served it: primary first, backup on failure.
    pub async fn catalog(&self) -> Result<(String, Catalog), ProviderError> {
        let mut cached = self.catalog.lock().await;
        if let Some((at, id, catalog)) = cached.as_ref() {
            if at.elapsed() < CATALOG_TTL {
                return Ok((id.clone(), catalog.clone()));
            }
        }
        let mut last_err = ProviderError::Unavailable("no swap provider configured".into());
        for provider in &self.providers {
            match provider.catalog().await {
                Ok(catalog) => {
                    *cached = Some((Instant::now(), provider.id.clone(), catalog.clone()));
                    return Ok((provider.id.clone(), catalog));
                }
                Err(e) => {
                    tracing::warn!("[swap] provider {} catalog failed: {}", provider.id, e);
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    /// Drop the cached catalog so the next request re-resolves (and may fail over to the backup).
    pub async fn invalidate_catalog(&self) {
        *self.catalog.lock().await = None;
    }

    /// Fetch the provider's current status for `row`, persist and audit any change, and return
    /// the resulting state. Never touches settlement: provider `completed` is only a state.
    pub async fn refresh(
        &self,
        row: &SwapRow,
    ) -> Result<(SwapState, Option<String>), ProviderError> {
        let provider = self.provider(&row.provider_id).ok_or_else(|| {
            ProviderError::Unavailable(format!("provider {} not configured", row.provider_id))
        })?;
        let current = (row.provider_state, row.state_reason.clone());
        let Some(status) = provider
            .order(&row.provider_order_id, &row.provider_token)
            .await?
        else {
            return Ok(current);
        };
        let reason = status
            .refund_reason
            .or(status.attention_reason)
            .map(str::to_string);
        if (status.state, &reason) != (current.0, &current.1) {
            self.store
                .update_state(&row.payment_hash, status.state, reason.as_deref())
                .map_err(|e| ProviderError::Unavailable(format!("store: {e}")))?;
            stable_channels::audit::audit_event(
                "ONBOARDING_SWAP_STATE",
                serde_json::json!({
                    "payment_hash": row.payment_hash,
                    "provider_id": row.provider_id,
                    "provider_order_id": row.provider_order_id,
                    "from": current.0.as_str(),
                    "to": status.state.as_str(),
                    "reason": reason,
                }),
            );
        }
        Ok((status.state, reason))
    }
}

// Declared here, not with the other modules: the ledger's audit-event scan treats everything after
// the first `#[cfg(test)] mod` as test code.
#[cfg(test)]
pub mod fake;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_round_trip_and_completed_is_not_terminal() {
        for state in SwapState::ALL {
            assert_eq!(SwapState::parse(state.as_str()), Some(state));
        }
        assert!(!SwapState::Completed.is_terminal());
        assert!(!SwapState::RefundRequired.is_terminal());
        assert!(SwapState::Refunded.is_terminal());
        assert_eq!(SwapState::parse("creating_provider_order"), None);
    }

    #[test]
    fn rate_limit_is_per_node_and_global_within_window() {
        let mut limit = CreateRateLimit::new(60, 2, 3);
        assert!(limit.try_acquire("a", 100).is_ok());
        assert!(limit.try_acquire("a", 101).is_ok());
        assert!(limit.try_acquire("a", 102).unwrap_err().contains("wallet"));
        assert!(limit.try_acquire("b", 103).is_ok());
        // Global cap reached even though "c" has never created.
        assert!(limit
            .try_acquire("c", 104)
            .unwrap_err()
            .contains("right now"));
        // Once the first two slide out of the window, both caps free up.
        assert!(limit.try_acquire("a", 161).is_ok());
        assert!(limit.try_acquire("c", 161).is_ok());
    }

    #[test]
    fn node_network_maps_to_invoice_currency() {
        assert_eq!(currency_for_network("bitcoin"), Some(Currency::Bitcoin));
        assert_eq!(currency_for_network("signet"), Some(Currency::Signet));
        assert_eq!(currency_for_network("regtest"), Some(Currency::Regtest));
        assert_eq!(currency_for_network("liquid"), None);
    }

    #[test]
    fn deposit_risk_matches_openreceive() {
        assert_eq!(
            PayInAsset::UsdtEth.deposit_risk(),
            DepositRisk::ChainAmbiguous
        );
        assert_eq!(
            PayInAsset::UsdcEth.deposit_risk(),
            DepositRisk::ChainAmbiguous
        );
        assert_eq!(PayInAsset::UsdtTron.deposit_risk(), DepositRisk::AssetOnly);
        assert_eq!(PayInAsset::UsdtSol.deposit_risk(), DepositRisk::AssetOnly);
        assert_eq!(PayInAsset::UsdcSol.deposit_risk(), DepositRisk::AssetOnly);
    }

    #[test]
    fn every_rail_warns_with_risk_specific_text() {
        let (title, eth) = PayInAsset::UsdtEth.network_warning("15.01");
        assert_eq!(title, "Wrong currency or network = lost funds");
        assert!(eth.contains("exactly 15.01 USDT on the Ethereum network"));
        assert!(eth.contains("other EVM networks"));
        let (_, tron) = PayInAsset::UsdtTron.network_warning("15.01");
        assert!(tron.contains("only accepts USDT on Tron"));
        assert!(!tron.contains("EVM"));
    }

    #[test]
    fn provider_network_spellings() {
        assert!(network_matches("TRX", "TRC20"));
        assert!(network_matches("ETH", "ERC20"));
        assert!(network_matches("SOL", "Solana"));
        assert!(!network_matches("ETH", "BSC"));
        assert!(is_lightning_network("Lightning"));
        assert!(is_lightning_network("BTCLN"));
        assert!(!is_lightning_network("BTC"));
    }
}
