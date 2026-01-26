use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::lightning::ln::types::ChannelId;
use std::{ops::{Div, Sub}, time::{SystemTime, UNIX_EPOCH}};
use serde::{Deserialize, Serialize};

// Custom serialization for ChannelId
mod channel_id_serde {
    use super::ChannelId;
    use serde::{Deserialize, Deserializer, Serializer, Serialize};

    pub fn serialize<S>(channel_id: &ChannelId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize the inner bytes
        let bytes = channel_id.0;
        bytes.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ChannelId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = <[u8; 32]>::deserialize(deserializer)?;
        Ok(ChannelId(bytes))
    }
}

// Custom serialization for PublicKey
mod pubkey_serde {
    use ldk_node::bitcoin::secp256k1::PublicKey;
    use serde::{Deserialize, Deserializer, Serializer, Serialize};
    use std::str::FromStr;

    pub fn serialize<S>(pubkey: &PublicKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as a string
        let pubkey_str = pubkey.to_string();
        pubkey_str.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PublicKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let pubkey_str = String::deserialize(deserializer)?;
        PublicKey::from_str(&pubkey_str).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Bitcoin {
    pub sats: u64, // Stored in Satoshis for precision
}

impl Default for Bitcoin {
    fn default() -> Self {
        Self { sats: 0 }
    }
}

impl Bitcoin {
    const SATS_IN_BTC: u64 = crate::constants::SATS_IN_BTC;

    pub fn from_sats(sats: u64) -> Self {
        Self { sats }
    }

    pub fn from_btc(btc: f64) -> Self {
        let sats = (btc * Self::SATS_IN_BTC as f64).round() as u64;
        Self::from_sats(sats)
    }

    pub fn to_btc(self) -> f64 {
        self.sats as f64 / Self::SATS_IN_BTC as f64
    }

    pub fn from_usd(usd: USD, btcusd_price: f64) -> Self {
        let btc = usd.0 / btcusd_price;
        Bitcoin::from_btc(btc)
    }

}

impl Sub for Bitcoin {
    type Output = Bitcoin;

    fn sub(self, other: Bitcoin) -> Bitcoin {
        Bitcoin::from_sats(self.sats.saturating_sub(other.sats))
    }
}

impl std::fmt::Display for Bitcoin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let btc_value = self.to_btc();

        // Format the value to 8 decimal places with spaces
        let formatted_btc = format!("{:.8}", btc_value);
        let with_spaces = formatted_btc
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 4 || i == 7 { format!(" {}", c) } else { c.to_string() })
            .collect::<String>();

        write!(f, "{} BTC", with_spaces)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct USD(pub f64);

impl Default for USD {
    fn default() -> Self {
        Self(0.0)
    }
}

impl USD {
    pub fn from_bitcoin(btc: Bitcoin, btcusd_price: f64) -> Self {
        Self(btc.to_btc() * btcusd_price)
    }

    pub fn from_f64(amount: f64) -> Self {
        Self(amount)
    }

    pub fn to_msats(self, btcusd_price: f64) -> u64 {
        let btc_value = self.0 / btcusd_price;
        let sats = btc_value * Bitcoin::SATS_IN_BTC as f64;
        let millisats = sats * 1000.0;
        millisats.abs().floor() as u64
    }
}

impl Sub for USD {
    type Output = USD;

    fn sub(self, other: USD) -> USD {
        USD(self.0 - other.0)
    }
}

impl Div<f64> for USD {
    type Output = USD;

    fn div(self, scalar: f64) -> USD {
        USD(self.0 / scalar)
    }
}

impl Div for USD {
    type Output = f64;

    fn div(self, other: USD) -> f64 {
        self.0 / other.0
    }
}

impl std::fmt::Display for USD {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "${:.2}", self.0)
    }
}

/// Allocation represents a portfolio weight vector for a channel.
/// Weights are between 0.0 and 1.0 and must sum to 1.0.
/// Example: { usd_weight: 0.25, btc_weight: 0.75 } means 25% USD, 75% BTC exposure.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Allocation {
    pub usd_weight: f64,
    pub btc_weight: f64,
}

impl Default for Allocation {
    fn default() -> Self {
        // Default to 100% USD stability (legacy behavior)
        Self {
            usd_weight: 1.0,
            btc_weight: 0.0,
        }
    }
}

impl Allocation {
    /// Create a new allocation with the given weights.
    /// Weights will be normalized to sum to 1.0.
    pub fn new(usd_weight: f64, btc_weight: f64) -> Result<Self, &'static str> {
        if usd_weight < 0.0 || btc_weight < 0.0 {
            return Err("Weights cannot be negative");
        }
        if usd_weight > 1.0 || btc_weight > 1.0 {
            return Err("Weights cannot exceed 1.0");
        }

        let sum = usd_weight + btc_weight;
        if sum == 0.0 {
            return Err("Weights cannot both be zero");
        }

        // Normalize to sum to 1.0
        Ok(Self {
            usd_weight: usd_weight / sum,
            btc_weight: btc_weight / sum,
        })
    }

    /// Create allocation from BTC percentage (0-100)
    pub fn from_btc_percent(btc_pct: u8) -> Self {
        let btc_pct = btc_pct.min(100) as f64 / 100.0;
        Self {
            usd_weight: 1.0 - btc_pct,
            btc_weight: btc_pct,
        }
    }

    /// Returns true if the allocation is valid (weights sum to ~1.0)
    pub fn is_valid(&self) -> bool {
        let sum = self.usd_weight + self.btc_weight;
        (sum - 1.0).abs() < 0.001 && self.usd_weight >= 0.0 && self.btc_weight >= 0.0
    }

    /// Get USD percentage (0-100)
    pub fn usd_percent(&self) -> u8 {
        (self.usd_weight * 100.0).round() as u8
    }

    /// Get BTC percentage (0-100)
    pub fn btc_percent(&self) -> u8 {
        (self.btc_weight * 100.0).round() as u8
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StableChannel {
    #[serde(with = "channel_id_serde")]
    pub channel_id: ChannelId,
    pub is_stable_receiver: bool,
    #[serde(with = "pubkey_serde")]
    pub counterparty: PublicKey,
    pub expected_usd: USD,
    pub expected_btc: Bitcoin,
    pub stable_receiver_btc: Bitcoin,
    pub stable_provider_btc: Bitcoin,
    pub stable_receiver_usd: USD,
    pub stable_provider_usd: USD,
    pub risk_level: i32,
    pub timestamp: i64,
    pub formatted_datetime: String,
    pub payment_made: bool,
    pub sc_dir: String,
    pub latest_price: f64,
    pub prices: String,
    pub onchain_btc: Bitcoin,
    pub onchain_usd: USD,
    pub note: Option<String>,
    /// Per-channel allocation weights (defaults to 100% USD for backward compatibility)
    #[serde(default)]
    pub allocation: Allocation,
    /// Native BTC exposure (the portion of the channel that floats with BTC price)
    #[serde(default)]
    pub native_channel_btc: Bitcoin,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bitcoin conversion tests
    #[test]
    fn test_bitcoin_from_sats() {
        let btc = Bitcoin::from_sats(100_000_000);
        assert_eq!(btc.to_btc(), 1.0);
    }

    #[test]
    fn test_bitcoin_from_btc() {
        let btc = Bitcoin::from_btc(1.5);
        assert_eq!(btc.sats, 150_000_000);
    }

    #[test]
    fn test_bitcoin_from_usd() {
        let usd = USD::from_f64(100_000.0);
        let btc = Bitcoin::from_usd(usd, 100_000.0); // $100k/BTC
        assert_eq!(btc.to_btc(), 1.0);
    }

    #[test]
    fn test_bitcoin_subtraction_saturating() {
        let a = Bitcoin::from_sats(100);
        let b = Bitcoin::from_sats(150);
        let result = a - b;
        assert_eq!(result.sats, 0); // Should saturate, not underflow
    }

    #[test]
    fn test_bitcoin_display_format() {
        let btc = Bitcoin::from_sats(12345678);
        let formatted = format!("{}", btc);
        assert!(formatted.contains("BTC"));
    }

    // USD conversion tests
    #[test]
    fn test_usd_from_bitcoin() {
        let btc = Bitcoin::from_btc(1.0);
        let usd = USD::from_bitcoin(btc, 50_000.0);
        assert_eq!(usd.0, 50_000.0);
    }

    #[test]
    fn test_usd_to_msats() {
        let usd = USD::from_f64(100.0);
        let msats = usd.to_msats(100_000.0); // $100k/BTC
        // $100 at $100k = 0.001 BTC = 100,000 sats = 100,000,000 msats
        assert_eq!(msats, 100_000_000);
    }

    #[test]
    fn test_usd_subtraction() {
        let a = USD::from_f64(100.0);
        let b = USD::from_f64(30.0);
        let result = a - b;
        assert_eq!(result.0, 70.0);
    }

    #[test]
    fn test_usd_division_by_scalar() {
        let usd = USD::from_f64(100.0);
        let result = usd / 4.0;
        assert_eq!(result.0, 25.0);
    }

    // Allocation tests
    #[test]
    fn test_allocation_new_normalizes() {
        let alloc = Allocation::new(0.5, 0.5).unwrap();
        assert_eq!(alloc.usd_weight, 0.5);
        assert_eq!(alloc.btc_weight, 0.5);
    }

    #[test]
    fn test_allocation_new_normalizes_uneven() {
        // Weights must be <= 1.0, then they get normalized to sum to 1.0
        let alloc = Allocation::new(0.2, 0.6).unwrap();
        assert!((alloc.usd_weight - 0.25).abs() < 0.001);
        assert!((alloc.btc_weight - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_allocation_rejects_negative() {
        let result = Allocation::new(-0.5, 0.5);
        assert!(result.is_err());
    }

    #[test]
    fn test_allocation_rejects_both_zero() {
        let result = Allocation::new(0.0, 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_allocation_from_btc_percent() {
        let alloc = Allocation::from_btc_percent(75);
        assert_eq!(alloc.btc_percent(), 75);
        assert_eq!(alloc.usd_percent(), 25);
    }

    #[test]
    fn test_allocation_from_btc_percent_clamps() {
        let alloc = Allocation::from_btc_percent(150); // over 100
        assert_eq!(alloc.btc_percent(), 100);
    }

    #[test]
    fn test_allocation_is_valid() {
        let valid = Allocation { usd_weight: 0.6, btc_weight: 0.4 };
        let invalid = Allocation { usd_weight: 0.5, btc_weight: 0.3 };
        assert!(valid.is_valid());
        assert!(!invalid.is_valid());
    }

    #[test]
    fn test_allocation_default_is_100_usd() {
        let alloc = Allocation::default();
        assert_eq!(alloc.usd_weight, 1.0);
        assert_eq!(alloc.btc_weight, 0.0);
    }

    #[test]
    fn test_stable_channel_default() {
        let sc = StableChannel::default();
        assert!(sc.is_stable_receiver);
        assert_eq!(sc.expected_usd.0, 0.0);
        assert_eq!(sc.risk_level, 0);
        assert!(sc.allocation.is_valid());
    }
}

// Implement manual Default for StableChannel
impl Default for StableChannel {
    fn default() -> Self {
        Self {
            channel_id: ChannelId::from_bytes([0; 32]),
            is_stable_receiver: true,
            counterparty: PublicKey::from_slice(&[2; 33]).unwrap_or_else(|_| {
                // This is a fallback that should never be reached,
                // but provides a valid default public key if needed
                PublicKey::from_slice(&[
                    0x02, 0x50, 0x86, 0x3A, 0xD6, 0x4A, 0x87, 0xAE, 0x8A, 0x2F, 0xE8, 0x3C, 0x1A,
                    0xF1, 0xA8, 0x40, 0x3C, 0xB5, 0x3F, 0x53, 0xE4, 0x86, 0xD8, 0x51, 0x1D, 0xAD,
                    0x8A, 0x04, 0x88, 0x7E, 0x5B, 0x23, 0x52,
                ]).unwrap()
            }),
            expected_usd: USD(0.0),
            expected_btc: Bitcoin::from_sats(0),
            stable_receiver_btc: Bitcoin::from_sats(0),
            stable_provider_btc: Bitcoin::from_sats(0),
            stable_receiver_usd: USD(0.0),
            stable_provider_usd: USD(0.0),
            risk_level: 0,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            formatted_datetime: "".to_string(),
            payment_made: false,
            sc_dir: ".data".to_string(),
            latest_price: 0.0,
            prices: "".to_string(),
            onchain_btc: Bitcoin::from_sats(0),
            onchain_usd: USD(0.0),
            note: Some("".to_string()),
            allocation: Allocation::default(),
            native_channel_btc: Bitcoin::from_sats(0),
        }
    }
}