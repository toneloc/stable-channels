use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::lightning::ln::ChannelId;
use ldk_node::lightning::offers::offer::Offer;
use std::ops::{Div, Sub};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Bitcoin {
    pub sats: u64, // Stored in Satoshis for precision
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
            .map(|(i, c)| if i == 4 || i == 7 { format!(" {}", c) } else { c.to_string() })
            .collect::<String>();

        write!(f, "{}btc", with_spaces)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct USD(pub f64);

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

#[derive(Clone, Debug)]
pub struct StableChannel {
    pub channel_id: ChannelId,
    pub is_stable_receiver: bool,
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
    pub counterparty_offer: Offer,
}