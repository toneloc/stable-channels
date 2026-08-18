// ============================================================================
// NETWORK CONSTANTS
// ============================================================================

/// Satoshis in one Bitcoin
pub const SATS_IN_BTC: u64 = 100_000_000;

/// Custom TLV type for stable channel messages
pub const STABLE_CHANNEL_TLV_TYPE: u64 = 13377331;

/// Authenticated metadata for a stability-payment keysend.
pub const SIGNED_STABILITY_TLV_TYPE: u64 = 13377333;

/// Maximum signed stability metadata accepted before parsing.
pub const MAX_SIGNED_STABILITY_TLV_VALUE_BYTES: usize = 8 * 1024;

/// Trade message type identifier
pub const TRADE_MESSAGE_TYPE: &str = "TRADE_V1";

/// Sync message type identifier (LSP → user expected_usd sync after stable deductions)
pub const SYNC_MESSAGE_TYPE: &str = "SYNC_V1";

/// Signed rejection message returned for correlated desktop trades.
pub const TRADE_REJECTED_MESSAGE_TYPE: &str = "TRADE_REJECTED_V1";

/// Signed stability-payment message type identifier.
pub const STABILITY_PAYMENT_MESSAGE_TYPE: &str = "STABILITY_PAYMENT_V1";

// ============================================================================
// DEFAULT CONFIGURATION VALUES
// ============================================================================

/// Default network
pub const DEFAULT_NETWORK: &str = "bitcoin";

/// Default user node alias
pub const DEFAULT_USER_ALIAS: &str = "user";

/// Default user port
pub const DEFAULT_USER_PORT: u16 = 9736;

/// Default LSP node alias
pub const DEFAULT_LSP_ALIAS: &str = "lsp";

/// Default LSP port
pub const DEFAULT_LSP_PORT: u16 = 9735;

/// Default chain source URL
pub const DEFAULT_CHAIN_URL: &str = "https://blockstream.info/api";
pub const FALLBACK_CHAIN_URL: &str = "https://mempool.space/api";

/// Default LSP public key
pub const DEFAULT_LSP_PUBKEY: &str =
    "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850";

/// Default LSP address
pub const DEFAULT_LSP_ADDRESS: &str = "stablechannels.com:9735";

// ============================================================================
// TIMING CONSTANTS
// ============================================================================

/// Price cache refresh interval (in seconds)
pub const PRICE_CACHE_REFRESH_SECS: u64 = 15;

/// Per-feed connect timeout — fail fast on an unreachable or geo-blocked host so feed
/// diversity provides the retry rather than serially waiting on one dead host.
pub const PRICE_FETCH_TIMEOUT_SECS: u64 = 3;

/// Per-feed overall request budget (connect + TLS + response body). Must exceed the connect
/// timeout: on a congested or filtered link a feed can consume most of the connect budget and
/// still need time to deliver its body, which the now-required 3-feed quorum depends on.
pub const PRICE_FETCH_TOTAL_TIMEOUT_SECS: u64 = 8;

/// Background sync intervals (in seconds)
pub const ONCHAIN_WALLET_SYNC_INTERVAL_SECS: u64 = 120;
pub const LIGHTNING_WALLET_SYNC_INTERVAL_SECS: u64 = 60;
pub const FEE_RATE_CACHE_UPDATE_INTERVAL_SECS: u64 = 1200;

/// Invoice expiration time (in seconds)
pub const INVOICE_EXPIRY_SECS: u32 = 3600;

/// Balance update interval for UI (in seconds)
pub const BALANCE_UPDATE_INTERVAL_SECS: u64 = 30;

/// Stability check interval (in seconds)
pub const STABILITY_CHECK_INTERVAL_SECS: u64 = 60;

/// A correlated trade becomes locally uncertain after this long, but remains late-resolvable.
pub const TRADE_RESULT_TIMEOUT_SECS: u64 = 15 * 60;

/// The LSP retries durable result delivery throughout this window.
pub const TRADE_RESPONSE_RETRY_WINDOW_SECS: u64 = 14 * 24 * 60 * 60;

/// Detailed signed response bytes may be pruned after this retention period.
pub const TRADE_RESPONSE_DETAIL_RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

// ============================================================================
// BUSINESS LOGIC CONSTANTS
// ============================================================================

/// Risk level thresholds
pub const MAX_RISK_LEVEL: i32 = 100;

/// Stability check thresholds
pub const STABILITY_THRESHOLD_PERCENT: f64 = 0.1; // 0.1% from par
pub const STABILITY_THRESHOLD_USD: f64 = 0.25; // minimum $0.25 drift to trigger payment

/// Minimum seconds between stability payments on the same channel (cooldown)
pub const STABILITY_PAYMENT_COOLDOWN_SECS: u64 = 120;

/// Maximum lifetime of a newly-created stability settlement authorization.
pub const STABILITY_PAYMENT_AUTH_TTL_SECS: u64 = 14 * 24 * 60 * 60;

/// Small allowance for peers whose system clocks are not perfectly aligned.
pub const STABILITY_PAYMENT_CLOCK_SKEW_SECS: u64 = 60;

/// Minimum USD amount to display in UI
pub const MIN_DISPLAY_USD: f64 = 2.0;

/// Auto-sweep: minimum on-chain sats to trigger splice_in
pub const AUTO_SWEEP_MIN_SATS: u64 = 10_000;

/// Stable-channel trade fee paid to the LSP as the TRADE_V1 keysend amount.
///
/// Shared by wallet fee construction and the LSP's server-side amount validation.
pub const STABLE_CHANNEL_TRADE_FEE_RATE: f64 = 0.01;

/// Maximum difference between the wallet's signed trade quote and the LSP's local price.
/// Enforced by the LSP; the wallet accepts an explicit rejection if their trusted prices differ.
pub const MAX_TRADE_QUOTE_DEVIATION_PERCENT: f64 = 0.5;

/// LDK channel-config defaults for outbound forwarding fees.
pub const LIGHTNING_DEFAULT_FORWARDING_FEE_BASE_MSAT: u64 = 1_000;
pub const LIGHTNING_DEFAULT_FORWARDING_FEE_PROPORTIONAL_MILLIONTHS: u64 = 0;

/// Approximate virtual bytes used for on-chain fee estimates shown before send.
pub const ESTIMATED_ONCHAIN_SEND_VBYTES: u64 = 140;
pub const ESTIMATED_ONCHAIN_SEND_ALL_VBYTES: u64 = 250;
pub const ESTIMATED_CHANNEL_CLOSE_VBYTES: u64 = 180;

// ============================================================================
// CHANNEL CONSTANTS
// ============================================================================

/// Channel opening parameters
pub const DEFAULT_CHANNEL_LIFETIME: u32 = 2016;
pub const DEFAULT_MAX_CLIENT_TO_SELF_DELAY: u32 = 1024;

/// Payment size limits
pub const MIN_PAYMENT_SIZE_MSAT: u64 = 0;
pub const MAX_PAYMENT_SIZE_MSAT: u64 = 100_000_000_000;

/// Channel over-provisioning (in ppm)
pub const CHANNEL_OVER_PROVISIONING_PPM: u32 = 1_000_000;

/// Channel opening fee (in ppm)
pub const CHANNEL_OPENING_FEE_PPM: u32 = 0;
pub const MIN_CHANNEL_OPENING_FEE_MSAT: u64 = 0;
pub const MIN_CHANNEL_LIFETIME: u32 = 100;

/// JIT channel fee limit (in ppm)
pub const MAX_PROPORTIONAL_LSP_FEE_LIMIT_PPM_MSAT: u64 = 10_000_000;

// ============================================================================
// PRICE FEED CONFIGURATION
// ============================================================================

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceFeedConfig {
    pub name: String,
    pub url_format: String,
    pub json_path: Vec<String>,
}

impl PriceFeedConfig {
    pub fn new(name: &str, url_format: &str, json_path: Vec<&str>) -> PriceFeedConfig {
        PriceFeedConfig {
            name: name.to_string(),
            url_format: url_format.to_string(),
            json_path: json_path.iter().map(|&s| s.to_string()).collect(),
        }
    }
}

pub fn get_default_price_feeds() -> Vec<PriceFeedConfig> {
    vec![
        PriceFeedConfig::new(
            "Bitstamp",
            "https://www.bitstamp.net/api/v2/ticker/btcusd/",
            vec!["last"],
        ),
        PriceFeedConfig::new(
            "Kraken",
            "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD",
            vec!["result", "XXBTZUSD", "c"],
        ),
        PriceFeedConfig::new(
            "Coinbase",
            "https://api.coinbase.com/v2/prices/BTC-USD/spot",
            vec!["data", "amount"],
        ),
        PriceFeedConfig::new(
            "Bitfinex",
            "https://api-pub.bitfinex.com/v2/ticker/tBTCUSD",
            vec!["6"],
        ),
        PriceFeedConfig::new(
            "Gemini",
            "https://api.gemini.com/v1/pubticker/btcusd",
            vec!["last"],
        ),
        PriceFeedConfig::new(
            "Bullish",
            "https://api.exchange.bullish.com/trading-api/v1/markets/BTCUSD/tick",
            vec!["last"],
        ),
    ]
}

pub fn get_fallback_usdt_price_feeds() -> Vec<PriceFeedConfig> {
    vec![
        PriceFeedConfig::new(
            "Binance BTC/USDT",
            "https://api.binance.com/api/v3/ticker/price?symbol=BTCUSDT",
            vec!["price"],
        ),
        PriceFeedConfig::new(
            "Bybit BTC/USDT",
            "https://api.bybit.com/v5/market/tickers?category=spot&symbol=BTCUSDT",
            vec!["result", "list", "0", "lastPrice"],
        ),
        PriceFeedConfig::new(
            "Huobi BTC/USDT",
            "https://api.huobi.pro/market/detail/merged?symbol=btcusdt",
            vec!["tick", "close"],
        ),
        PriceFeedConfig::new(
            "KuCoin BTC/USDT",
            "https://api.kucoin.com/api/v1/market/orderbook/level1?symbol=BTC-USDT",
            vec!["data", "price"],
        ),
        PriceFeedConfig::new(
            "Gate.io BTC/USDT",
            "https://api.gateio.ws/api/v4/spot/tickers?currency_pair=BTC_USDT",
            vec!["0", "last"],
        ),
        PriceFeedConfig::new(
            "MEXC BTC/USDT",
            "https://api.mexc.com/api/v3/ticker/price?symbol=BTCUSDT",
            vec!["price"],
        ),
        PriceFeedConfig::new(
            "Luno BTC/USDT",
            "https://api.luno.com/api/1/ticker?pair=XBTUSDT",
            vec!["last_trade"],
        ),
        PriceFeedConfig::new(
            "CoinDCX BTC/USDT",
            "https://public.coindcx.com/market_data/trade_history?pair=B-BTC_USDT&limit=1",
            vec!["0", "p"],
        ),
        PriceFeedConfig::new(
            "BTCTurk BTC/USDT",
            "https://api.btcturk.com/api/v2/ticker?pairSymbol=BTCUSDT",
            vec!["data", "0", "last"],
        ),
    ]
}

pub fn get_usdt_usd_price_feeds() -> Vec<PriceFeedConfig> {
    vec![
        PriceFeedConfig::new(
            "Coinbase USDT/USD",
            "https://api.coinbase.com/v2/prices/USDT-USD/spot",
            vec!["data", "amount"],
        ),
        PriceFeedConfig::new(
            "Kraken USDT/USD",
            "https://api.kraken.com/0/public/Ticker?pair=USDTUSD",
            vec!["result", "USDTZUSD", "c"],
        ),
        PriceFeedConfig::new(
            "Bitstamp USDT/USD",
            "https://www.bitstamp.net/api/v2/ticker/usdtusd/",
            vec!["last"],
        ),
        PriceFeedConfig::new(
            "Bitfinex USDT/USD",
            "https://api-pub.bitfinex.com/v2/ticker/tUSTUSD",
            vec!["6"],
        ),
        PriceFeedConfig::new(
            "CoinGecko USDT/USD",
            "https://api.coingecko.com/api/v3/simple/price?ids=tether&vs_currencies=usd",
            vec!["tether", "usd"],
        ),
        // Disjoint-host peg sources: the four exchange peg feeds above share hosts with the
        // direct-USD tier, so without these the fallback's peg gate would fail exactly when
        // the primary tier is unreachable — the outage the fallback exists to survive.
        PriceFeedConfig::new(
            "Crypto.com USDT/USD",
            "https://api.crypto.com/exchange/v1/public/get-tickers?instrument_name=USDT_USD",
            vec!["result", "data", "0", "a"],
        ),
        PriceFeedConfig::new(
            "OKX USDT/USD",
            "https://www.okx.com/api/v5/market/ticker?instId=USDT-USD",
            vec!["data", "0", "last"],
        ),
    ]
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

use dirs::data_dir;
use std::path::PathBuf;

/// Get the user data directory
pub fn get_user_data_dir() -> PathBuf {
    data_dir()
        .expect("Could not determine user data dir")
        .join("StableChannels")
        .join(DEFAULT_USER_ALIAS)
}

/// Get the LSP data directory
pub fn get_lsp_data_dir() -> PathBuf {
    data_dir()
        .expect("Could not determine LSP data dir")
        .join("StableChannels")
        .join(DEFAULT_LSP_ALIAS)
}

/// Get the audit log path for a given mode ("user" or "lsp")
pub fn audit_log_path_for(mode: &str) -> String {
    let base_dir = match mode {
        "user" => get_user_data_dir(),
        "lsp" => get_lsp_data_dir(),
        _ => panic!("Invalid mode for audit log path"),
    };
    base_dir
        .join("audit_log.txt")
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sats_in_btc_constant() {
        assert_eq!(SATS_IN_BTC, 100_000_000);
    }

    #[test]
    fn test_default_price_feeds_not_empty() {
        let feeds = get_default_price_feeds();
        assert_eq!(feeds.len(), 6);
        assert!(feeds.iter().all(|feed| !feed.url_format.contains("USDT")));
        assert_eq!(get_fallback_usdt_price_feeds().len(), 9);
        assert_eq!(get_usdt_usd_price_feeds().len(), 7);
    }

    #[test]
    fn usdt_peg_gate_survives_direct_usd_host_outage() {
        // The USDT fallback's peg gate needs 3 agreeing feeds. If too many peg feeds share
        // hosts with the direct-USD tier, the fallback fails exactly when the primary tier
        // is unreachable — the outage it exists to survive.
        fn host(url: &str) -> String {
            url.split('/').nth(2).unwrap_or("").to_string()
        }
        let usd_hosts: std::collections::HashSet<String> = get_default_price_feeds()
            .iter()
            .map(|feed| host(&feed.url_format))
            .collect();
        let disjoint = get_usdt_usd_price_feeds()
            .iter()
            .filter(|feed| !usd_hosts.contains(&host(&feed.url_format)))
            .count();
        // 3 = MIN_AGREEING_PEG_FEEDS in price_feeds.rs
        assert!(
            disjoint >= 3,
            "peg gate needs >=3 feeds on hosts disjoint from the direct-USD tier; got {disjoint}"
        );
    }

    #[test]
    fn test_price_feed_config_new() {
        let feed = PriceFeedConfig::new("Test", "https://test.com", vec!["a", "b"]);
        assert_eq!(feed.name, "Test");
        assert_eq!(feed.json_path, vec!["a", "b"]);
    }

    #[test]
    fn test_get_user_data_dir() {
        let path = get_user_data_dir();
        assert!(path.to_string_lossy().contains("StableChannels"));
        assert!(path.to_string_lossy().contains("user"));
    }

    #[test]
    fn test_get_lsp_data_dir() {
        let path = get_lsp_data_dir();
        assert!(path.to_string_lossy().contains("StableChannels"));
        assert!(path.to_string_lossy().contains("lsp"));
    }

    #[test]
    fn test_audit_log_path_for_user() {
        let path = audit_log_path_for("user");
        assert!(path.contains("audit_log.txt"));
    }

    #[test]
    fn test_audit_log_path_for_lsp() {
        let path = audit_log_path_for("lsp");
        assert!(path.contains("audit_log.txt"));
    }

    #[test]
    #[should_panic(expected = "Invalid mode")]
    fn test_audit_log_path_invalid_mode() {
        audit_log_path_for("invalid");
    }

    #[test]
    fn test_stability_threshold_is_reasonable() {
        assert!(STABILITY_THRESHOLD_PERCENT > 0.0);
        assert!(STABILITY_THRESHOLD_PERCENT < 10.0);
    }

    #[test]
    fn test_max_risk_level_is_positive() {
        assert!(MAX_RISK_LEVEL > 0);
    }
}
