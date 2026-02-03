// ============================================================================
// NETWORK CONSTANTS
// ============================================================================

/// Satoshis in one Bitcoin
pub const SATS_IN_BTC: u64 = 100_000_000;

/// Custom TLV type for stable channel messages
pub const STABLE_CHANNEL_TLV_TYPE: u64 = 13377331;

/// Trade message type identifier
pub const TRADE_MESSAGE_TYPE: &str = "TRADE_V1";

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
pub const DEFAULT_LSP_PORT: u16 = 9737;

/// Default chain source URL
pub const DEFAULT_CHAIN_URL: &str = "https://blockstream.info/api";

/// Default LSP public key
pub const DEFAULT_LSP_PUBKEY: &str = "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850";

/// Default LSP address
pub const DEFAULT_LSP_ADDRESS: &str = "100.25.168.115:9737";

/// Default gateway public key
pub const DEFAULT_GATEWAY_PUBKEY: &str = "03da1c27ca77872ac5b3e568af30673e599a47a5e4497f85c7b5da42048807b3ed";

/// Default gateway address
pub const DEFAULT_GATEWAY_ADDRESS: &str = "213.174.156.80:9735";

// ============================================================================
// TIMING CONSTANTS
// ============================================================================

/// Price cache refresh interval (in seconds)
pub const PRICE_CACHE_REFRESH_SECS: u64 = 5;

/// Price fetch retry delay (in milliseconds)
pub const PRICE_FETCH_RETRY_DELAY_MS: u64 = 300;

/// Price fetch maximum retry attempts
pub const PRICE_FETCH_MAX_RETRIES: usize = 3;

/// Background sync intervals (in seconds)
pub const ONCHAIN_WALLET_SYNC_INTERVAL_SECS: u64 = 160;
pub const LIGHTNING_WALLET_SYNC_INTERVAL_SECS: u64 = 60;
pub const FEE_RATE_CACHE_UPDATE_INTERVAL_SECS: u64 = 1200;

/// Invoice expiration time (in seconds)
pub const INVOICE_EXPIRY_SECS: u32 = 3600;

/// Balance update interval for UI (in seconds)
pub const BALANCE_UPDATE_INTERVAL_SECS: u64 = 30;

/// Stability check interval (in seconds)
pub const STABILITY_CHECK_INTERVAL_SECS: u64 = 60;

// ============================================================================
// BUSINESS LOGIC CONSTANTS
// ============================================================================

/// Risk level thresholds
pub const MAX_RISK_LEVEL: i32 = 100;

/// Stability check thresholds
pub const STABILITY_THRESHOLD_PERCENT: f64 = 0.1; // 0.1% from par

/// Minimum USD amount to display in UI
pub const MIN_DISPLAY_USD: f64 = 2.0;

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
            "CoinGecko",
            "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd",
            vec!["bitcoin", "usd"],
        ),
        PriceFeedConfig::new(
            "Kraken",
            "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD",
            vec!["result", "XXBTZUSD", "c"],
        ),
        PriceFeedConfig::new(
            "Coinbase",
            "https://api.coinbase.com/v2/prices/spot?currency=USD",
            vec!["data", "amount"],
        ),
        PriceFeedConfig::new(
            "Blockchain.com",
            "https://blockchain.info/ticker",
            vec!["USD", "last"],
        ),
    ]
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

use std::path::PathBuf;
use dirs::data_dir;

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
    base_dir.join("audit_log.txt").to_string_lossy().into_owned()
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
        assert!(!feeds.is_empty());
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