//! In-memory stable-channel manager, backed by the shared sqlite channels table.

use std::path::PathBuf;
use std::sync::Arc;

use ldk_server_client::client::LdkServerClient;
use ldk_server_client::ldk_server_grpc::api::ListChannelsRequest;
use ldk_server_client::ldk_server_grpc::types::Channel;
use stable_channels::db::Database;
use stable_channels::types::{Bitcoin, StableChannel, USD};
use tracing::{error, info};

/// In-memory list of stable channels plus a handle to the shared sqlite channels table.
pub struct StableChannelManager {
    pub stable_channels: Vec<StableChannel>,
    db: Arc<Database>,
    data_dir: PathBuf,
}

/// Outcome of an `edit_stable_channel` call.
#[derive(Debug, PartialEq)]
pub struct EditOutcome {
    pub ok: bool,
    pub status: String,
}

impl StableChannelManager {
    pub fn new(db: Arc<Database>, data_dir: PathBuf) -> Self {
        Self {
            stable_channels: Vec::new(),
            db,
            data_dir,
        }
    }

    /// Validate, patch expected_usd/note (Some sets, None keeps prior, both-None-no-prior rejected), persist, and update the cache.
    pub async fn edit_stable_channel(
        &mut self,
        channel_id: &str,
        expected_usd_in: Option<f64>,
        note_in: Option<String>,
        ldk_server: &LdkServerClient,
        btc_price: f64,
    ) -> EditOutcome {
        let channels_resp = match ldk_server.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r,
            Err(e) => {
                error!("[stable] list_channels gRPC failed: {}", e);
                return EditOutcome {
                    ok: false,
                    status: format!("list_channels failed: {}", e),
                };
            }
        };

        let Some(channel) = channels_resp
            .channels
            .into_iter()
            .find(|c| c.channel_id == channel_id)
        else {
            return EditOutcome {
                ok: false,
                status: format!("No channel matching: {}", channel_id),
            };
        };

        // Snapshot of any existing record for patch fallback.
        let user_channel_id_str = channel.user_channel_id.clone();
        let prior = self
            .stable_channels
            .iter()
            .find(|sc| format!("{}", sc.user_channel_id) == user_channel_id_str);

        let prior_target = prior.map(|p| p.expected_usd.0);
        let prior_note = prior.and_then(|p| p.note.clone());

        let expected_usd_f = match (expected_usd_in, prior_target) {
            (Some(v), _) => v,
            (None, Some(prev)) => prev,
            (None, None) => 0.0,
        };
        let note = match (note_in.clone(), prior_note) {
            (Some(s), _) => Some(s),
            (None, Some(prev)) => Some(prev),
            (None, None) => None,
        };

        if expected_usd_in.is_none() && note_in.is_none() && prior.is_none() {
            return EditOutcome {
                ok: false,
                status: "No changes provided".to_string(),
            };
        }

        let expected_usd = USD::from_f64(expected_usd_f);
        let expected_btc = Bitcoin::from_usd(expected_usd, btc_price);

        let unspendable = channel.unspendable_punishment_reserve.unwrap_or(0);
        let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable;
        let their_balance_sats = channel.channel_value_sats.saturating_sub(our_balance_sats);

        let stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
        let stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
        let stable_provider_usd = USD::from_bitcoin(stable_provider_btc, btc_price);
        let stable_receiver_usd = USD::from_bitcoin(stable_receiver_btc, btc_price);

        let backing_sats = if btc_price > 0.0 {
            ((expected_usd_f / btc_price) * 100_000_000.0) as u64
        } else {
            0
        };
        let native_sats = their_balance_sats.saturating_sub(backing_sats);

        let user_channel_id_u128 = u128::from_str_radix(
            user_channel_id_str.trim_start_matches("0x"),
            16,
        )
        .unwrap_or(0);

        let new_sc = build_stable_channel(
            &channel,
            user_channel_id_u128,
            expected_usd,
            expected_btc,
            stable_provider_btc,
            stable_receiver_btc,
            stable_provider_usd,
            stable_receiver_usd,
            backing_sats,
            native_sats,
            note.clone(),
            btc_price,
            self.data_dir.clone(),
        );

        if let Err(e) = self.db.save_channel(
            &channel.channel_id,
            &user_channel_id_str,
            expected_usd_f,
            backing_sats,
            native_sats,
            note.as_deref(),
        ) {
            return EditOutcome {
                ok: false,
                status: format!("DB write failed: {}", e),
            };
        }

        self.stable_channels
            .retain(|sc| format!("{}", sc.user_channel_id) != user_channel_id_str);
        self.stable_channels.push(new_sc);

        info!(
            "[stable] edited channel={} user_channel_id={} expected_usd={}",
            channel_id, user_channel_id_str, expected_usd_f
        );

        EditOutcome {
            ok: true,
            status: format!("Set expected_usd={} on channel {}", expected_usd_f, channel_id),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_stable_channel(
    channel: &Channel,
    user_channel_id: u128,
    expected_usd: USD,
    expected_btc: Bitcoin,
    stable_provider_btc: Bitcoin,
    stable_receiver_btc: Bitcoin,
    stable_provider_usd: USD,
    stable_receiver_usd: USD,
    backing_sats: u64,
    native_sats: u64,
    note: Option<String>,
    btc_price: f64,
    sc_dir: PathBuf,
) -> StableChannel {
    let channel_id_bytes = parse_channel_id_hex(&channel.channel_id);
    let counterparty = parse_pubkey_hex(&channel.counterparty_node_id);

    StableChannel {
        channel_id: ldk_node::lightning::ln::types::ChannelId::from_bytes(channel_id_bytes),
        user_channel_id,
        counterparty,
        is_stable_receiver: false,
        expected_usd,
        expected_btc,
        stable_receiver_btc,
        stable_receiver_usd,
        stable_provider_btc,
        stable_provider_usd,
        latest_price: btc_price,
        risk_level: 0,
        payment_made: false,
        timestamp: 0,
        formatted_datetime: String::new(),
        sc_dir: sc_dir.to_string_lossy().to_string(),
        prices: String::new(),
        onchain_btc: Bitcoin::from_sats(0),
        onchain_usd: USD(0.0),
        note,
        native_channel_btc: Bitcoin::from_sats(0),
        backing_sats,
        native_sats,
        last_stability_payment: 0,
    }
}

fn parse_channel_id_hex(s: &str) -> [u8; 32] {
    let mut buf = [0u8; 32];
    if let Ok(bytes) = hex::decode(s) {
        let n = bytes.len().min(32);
        buf[..n].copy_from_slice(&bytes[..n]);
    }
    buf
}

fn parse_pubkey_hex(s: &str) -> ldk_node::bitcoin::secp256k1::PublicKey {
    use std::str::FromStr;
    ldk_node::bitcoin::secp256k1::PublicKey::from_str(s).unwrap_or_else(|_| {
        let mut buf = [2u8; 33];
        buf[1] = 0;
        ldk_node::bitcoin::secp256k1::PublicKey::from_slice(&buf)
            .expect("static dummy pubkey is valid")
    })
}

#[cfg(test)]
mod tests {
    // Unit-testing edit_stable_channel without a real LdkServerClient is awkward
    // because LdkServerClient does not expose a trait we can mock. For Bucket 2
    // we cover the patch semantics indirectly through Task 14's Mutinynet smoke test.
}
