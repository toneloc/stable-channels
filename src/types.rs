use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::lightning::ln::types::ChannelId;
use std::ops::{Div, Sub};
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
    const SATS_IN_BTC: u64 = 100_000_000;

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

        write!(f, "{}btc", with_spaces)
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
    pub prices: String
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
            timestamp: 0,
            formatted_datetime: "".to_string(),
            payment_made: false,
            sc_dir: ".data".to_string(),
            latest_price: 0.0,
            prices: "".to_string(),
        }
    }
}