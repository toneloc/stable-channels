//! In-memory stable-channel manager, backed by the shared sqlite channels table.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use ldk_server_client::client::LdkServerClient;
use ldk_server_client::error::LdkServerError;
use ldk_server_client::ldk_server_grpc::api::{
    GetBalancesRequest, GetBalancesResponse, ListChannelsRequest, ListChannelsResponse,
    ListForwardedPaymentsRequest, ListForwardedPaymentsResponse, ListPeersRequest,
    ListPeersResponse, SignMessageRequest, SignMessageResponse, SpontaneousSendRequest,
    SpontaneousSendResponse, VerifySignatureRequest, VerifySignatureResponse,
};
use ldk_server_client::ldk_server_grpc::events::ChannelStateChangeReason;
use ldk_server_client::ldk_server_grpc::types::{Channel, CustomTlvRecord};
use stable_channels::db::Database;
use stable_channels::types::{Bitcoin, StableChannel, USD};
use tracing::{error, info};

const MAX_TRADE_QUOTE_DEVIATION_PERCENT: f64 = 0.5;

/// Return each peer's own spendable-plus-reserve balance from the fields LDK exposes for that
/// peer. `channel_value - local_balance` is not the remote balance: on outbound channels it also
/// assigns the funder's current commitment fee to the remote peer.
fn channel_peer_balances(channel: &Channel) -> (u64, u64) {
    let local_sats = (channel.outbound_capacity_msat / 1000)
        .saturating_add(channel.unspendable_punishment_reserve.unwrap_or(0));
    let remote_sats = (channel.inbound_capacity_msat / 1000)
        .saturating_add(channel.counterparty_unspendable_punishment_reserve);
    (local_sats, remote_sats)
}

/// Reproduce the wallet's trade-fee calculation from the allocation transition. Buys reduce the
/// target by the gross amount. Sells increase it by the net amount, so the gross amount must be
/// recovered before applying the one-percent fee. The wallet pays whole sats, with a one-msat
/// minimum for a zero-sat fee.
fn expected_trade_fee_msat(
    old_expected_usd: f64,
    new_expected_usd: f64,
    quote_price: f64,
) -> Option<u64> {
    let fee_rate = stable_channels::constants::STABLE_CHANNEL_TRADE_FEE_RATE;
    if !old_expected_usd.is_finite()
        || old_expected_usd < 0.0
        || !new_expected_usd.is_finite()
        || new_expected_usd < 0.0
        || !quote_price.is_finite()
        || quote_price <= 0.0
        || !fee_rate.is_finite()
        || !(0.0..1.0).contains(&fee_rate)
    {
        return None;
    }

    let target_delta = (new_expected_usd - old_expected_usd).abs();
    let gross_usd = if new_expected_usd > old_expected_usd {
        target_delta / (1.0 - fee_rate)
    } else {
        target_delta
    };
    let fee_sats = gross_usd * fee_rate / quote_price * 100_000_000.0;
    if !fee_sats.is_finite() || fee_sats < 0.0 || fee_sats > (u64::MAX / 1000) as f64 {
        return None;
    }

    Some((fee_sats as u64).saturating_mul(1000).max(1))
}

fn trade_fee_tolerance_msat(expected_msat: u64, has_signed_quote: bool) -> u64 {
    if has_signed_quote {
        return 0;
    }

    // Transitional legacy wallets did not sign their quote. Admit the same maximum price skew as
    // signed trades, plus one sat for whole-sat rounding, while still rejecting material underpay.
    ((expected_msat as f64 * MAX_TRADE_QUOTE_DEVIATION_PERCENT / 100.0).ceil() as u64)
        .max(1000)
}

/// Tiny trait of the gRPC methods the manager calls, so run_tick and handlers can be unit-tested with a fake.
#[async_trait]
pub trait LdkServerCalls: Send + Sync {
    async fn list_channels(
        &self,
        req: ListChannelsRequest,
    ) -> Result<ListChannelsResponse, LdkServerError>;
    async fn spontaneous_send(
        &self,
        req: SpontaneousSendRequest,
    ) -> Result<SpontaneousSendResponse, LdkServerError>;
    async fn sign_message(
        &self,
        req: SignMessageRequest,
    ) -> Result<SignMessageResponse, LdkServerError>;
    async fn verify_signature(
        &self,
        req: VerifySignatureRequest,
    ) -> Result<VerifySignatureResponse, LdkServerError>;
    async fn list_forwarded_payments(
        &self,
        _req: ListForwardedPaymentsRequest,
    ) -> Result<ListForwardedPaymentsResponse, LdkServerError> {
        Ok(ListForwardedPaymentsResponse::default())
    }
    async fn get_balances(
        &self,
        _req: GetBalancesRequest,
    ) -> Result<GetBalancesResponse, LdkServerError> {
        Ok(GetBalancesResponse::default())
    }
    async fn list_peers(
        &self,
        _req: ListPeersRequest,
    ) -> Result<ListPeersResponse, LdkServerError> {
        Ok(ListPeersResponse::default())
    }
}

#[async_trait]
impl LdkServerCalls for LdkServerClient {
    async fn list_channels(
        &self,
        req: ListChannelsRequest,
    ) -> Result<ListChannelsResponse, LdkServerError> {
        LdkServerClient::list_channels(self, req).await
    }
    async fn spontaneous_send(
        &self,
        req: SpontaneousSendRequest,
    ) -> Result<SpontaneousSendResponse, LdkServerError> {
        LdkServerClient::spontaneous_send(self, req).await
    }
    async fn sign_message(
        &self,
        req: SignMessageRequest,
    ) -> Result<SignMessageResponse, LdkServerError> {
        LdkServerClient::sign_message(self, req).await
    }
    async fn verify_signature(
        &self,
        req: VerifySignatureRequest,
    ) -> Result<VerifySignatureResponse, LdkServerError> {
        LdkServerClient::verify_signature(self, req).await
    }
    async fn list_forwarded_payments(
        &self,
        req: ListForwardedPaymentsRequest,
    ) -> Result<ListForwardedPaymentsResponse, LdkServerError> {
        LdkServerClient::list_forwarded_payments(self, req).await
    }
    async fn get_balances(
        &self,
        req: GetBalancesRequest,
    ) -> Result<GetBalancesResponse, LdkServerError> {
        LdkServerClient::get_balances(self, req).await
    }
    async fn list_peers(
        &self,
        req: ListPeersRequest,
    ) -> Result<ListPeersResponse, LdkServerError> {
        LdkServerClient::list_peers(self, req).await
    }
}

/// In-memory list of stable channels plus a handle to the shared sqlite channels table.
pub struct StableChannelManager {
    pub stable_channels: Vec<StableChannel>,
    db: Arc<Database>,
    data_dir: PathBuf,
    /// Per-channel consecutive low-balance tick count for the balance-truth backstop debounce (ignores transient in-flight HTLCs).
    spend_debounce: std::collections::HashMap<u128, u8>,
    /// Per-channel last logged stability outcome + value, so run_tick only audits on state-change.
    stability_throttle: std::collections::HashMap<u128, (String, f64)>,
    /// Persisted allocations still awaiting their one-time startup SYNC. Tracking each channel
    /// independently prevents one incoherent channel from blocking every other wallet.
    startup_sync_pending: std::collections::HashSet<u128>,
    startup_sync_initialized: bool,
}

/// Outcome of an `edit_stable_channel` call.
#[derive(Debug, PartialEq)]
pub struct EditOutcome {
    pub ok: bool,
    pub status: String,
}

impl StableChannelManager {
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub fn new(db: Arc<Database>, data_dir: PathBuf) -> Self {
        Self {
            stable_channels: Vec::new(),
            db,
            data_dir,
            spend_debounce: std::collections::HashMap::new(),
            stability_throttle: std::collections::HashMap::new(),
            startup_sync_pending: std::collections::HashSet::new(),
            startup_sync_initialized: false,
        }
    }

    /// Validate, patch expected_usd/note (Some sets, None keeps prior, both-None-no-prior rejected), persist, and update the cache.
    pub async fn edit_stable_channel(
        &mut self,
        channel_id: &str,
        expected_usd_in: Option<f64>,
        note_in: Option<String>,
        ldk_server: &dyn LdkServerCalls,
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

        let (our_balance_sats, their_balance_sats) = channel_peer_balances(&channel);

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

        let user_channel_id_u128 = parse_user_channel_id(&user_channel_id_str).unwrap_or(0);

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

        stable_channels::audit::audit_event(
            "STABLE_EDITED",
            serde_json::json!({
                "channel_id": channel_id,
                "user_channel_id": user_channel_id_str,
                "target_usd": expected_usd_f,
                "note": note,
            }),
        );

        EditOutcome {
            ok: true,
            status: format!("Set expected_usd={} on channel {}", expected_usd_f, channel_id),
        }
    }

    /// Remove the stable_channel record from in-memory state when a channel closes, and soft-close the DB row (preserved for forensics, excluded from future reconcile/tick reads).
    pub fn handle_channel_closed(
        &mut self,
        channel_id: String,
        user_channel_id: String,
        counterparty_node_id: Option<String>,
        funding_txo: Option<String>,
        closure_initiator: i32,
        reason: Option<ChannelStateChangeReason>,
    ) {
        let target = parse_user_channel_id(&user_channel_id);
        self.stable_channels.retain(|sc| {
            if let Some(t) = target {
                sc.user_channel_id != t
            } else {
                format!("{}", sc.user_channel_id) != user_channel_id
            }
        });
        if let Some(t) = target {
            self.spend_debounce.remove(&t);
            self.stability_throttle.remove(&t);
        }
        if let Err(e) = self.db.mark_channel_closed(&user_channel_id) {
            tracing::error!(
                "[stable] handle_channel_closed: db.mark_channel_closed failed for {}: {}",
                user_channel_id, e
            );
            stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({ "op": "mark_channel_closed", "user_channel_id": user_channel_id, "channel_id": channel_id, "error": e.to_string() }),
            );
        }
        stable_channels::audit::audit_event(
            "CHANNEL_CLOSED",
            crate::channel_close::close_audit_data(
                &channel_id,
                &user_channel_id,
                counterparty_node_id.as_deref(),
                funding_txo.as_deref(),
                closure_initiator,
                reason.as_ref(),
            ),
        );
    }

    /// Rebuild the in-memory stable-channel list at startup from sqlite joined with the live snapshot, dropping vanished channels.
    pub async fn reconcile_from_grpc(
        &mut self,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r.channels,
            Err(e) => {
                tracing::error!("[stable] reconcile: list_channels failed: {}", e);
                stable_channels::audit::audit_event(
                    "LDK_CALL_FAILED",
                    serde_json::json!({ "op": "list_channels", "context": "reconcile", "error": e.to_string() }),
                );
                return;
            }
        };

        // Map from u128 user_channel_id (parsed from decimal) -> Channel snapshot.
        let mut by_user_channel_id: std::collections::HashMap<u128, Channel> =
            std::collections::HashMap::new();
        for c in &channels {
            if let Some(uid) = parse_user_channel_id(&c.user_channel_id) {
                by_user_channel_id.insert(uid, c.clone());
            }
        }

        // Load persisted stable-channel records from sqlite.
        let records = match self.db.load_all_channels() {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("[stable] reconcile: db.load_all_channels failed: {}", e);
                stable_channels::audit::audit_event(
                    "DB_READ_FAILED",
                    serde_json::json!({ "op": "load_all_channels", "context": "reconcile", "error": e.to_string() }),
                );
                return;
            }
        };

        // Rebuild the in-memory Vec from the persisted records joined with the live snapshot.
        let mut rebuilt: Vec<StableChannel> = Vec::new();
        for record in &records {
            // Parse user_channel_id the same (decimal) way for db records and live channels so they match.
            let live = parse_user_channel_id(&record.user_channel_id)
                .and_then(|uid| by_user_channel_id.get(&uid).map(|c| (uid, c)));

            let Some((user_channel_id_u128, c)) = live else {
                // Channel not in current live snapshot — soft-close in DB so
                // forensics survive a transient gRPC blip. If the channel
                // comes back on a future reconcile or save_channel call,
                // closed_at is cleared automatically.
                if let Err(e) = self.db.mark_channel_closed(&record.user_channel_id) {
                    tracing::error!(
                        "[stable] reconcile: db.mark_channel_closed({}) failed: {}",
                        record.user_channel_id, e
                    );
                    stable_channels::audit::audit_event(
                        "DB_WRITE_FAILED",
                        serde_json::json!({ "op": "mark_channel_closed", "context": "reconcile", "user_channel_id": record.user_channel_id, "error": e.to_string() }),
                    );
                }
                stable_channels::audit::audit_event(
                    "CHANNEL_MARKED_CLOSED_AT_STARTUP",
                    serde_json::json!({ "user_channel_id": record.user_channel_id }),
                );
                continue;
            };

            // Balances come from the live channel. expected_usd/backing/native/note are the persisted intent.
            let (our_sats, their_sats) = channel_peer_balances(c);

            let stable_provider_btc = Bitcoin::from_sats(our_sats);
            let stable_receiver_btc = Bitcoin::from_sats(their_sats);
            let stable_provider_usd = USD::from_bitcoin(stable_provider_btc, btc_price);
            let stable_receiver_usd = USD::from_bitcoin(stable_receiver_btc, btc_price);

            let expected_usd = USD::from_f64(record.expected_usd);
            let expected_btc = Bitcoin::from_usd(expected_usd, btc_price);

            let mut sc = build_stable_channel(
                c,
                user_channel_id_u128,
                expected_usd,
                expected_btc,
                stable_provider_btc,
                stable_receiver_btc,
                stable_provider_usd,
                stable_receiver_usd,
                record.backing_sats,
                record.native_sats,
                record.note.clone(),
                btc_price,
                self.data_dir.clone(),
            );
            stable_channels::stable::recompute_native(&mut sc);
            rebuilt.push(sc);
        }

        self.stable_channels = rebuilt;
        info!(
            "[stable] reconciled {} stable channel(s) from sqlite",
            self.stable_channels.len()
        );

        if !self.startup_sync_initialized && !self.stable_channels.is_empty() {
            self.startup_sync_pending
                .extend(self.stable_channels.iter().map(|sc| sc.user_channel_id));
            self.startup_sync_initialized = true;
        }
        self.retry_startup_sync(ldk).await;
    }

    async fn retry_startup_sync(&mut self, ldk: &dyn LdkServerCalls) {
        if self.startup_sync_pending.is_empty() {
            return;
        }
        let live_ids: std::collections::HashSet<u128> = self
            .stable_channels
            .iter()
            .map(|sc| sc.user_channel_id)
            .collect();
        self.startup_sync_pending
            .retain(|uid| live_ids.contains(uid));

        let syncs: Vec<_> = self
            .stable_channels
            .iter()
            .filter(|sc| {
                self.startup_sync_pending.contains(&sc.user_channel_id)
                    && sc.stable_receiver_btc.sats >= sc.backing_sats
            })
            .map(|sc| {
                (
                    sc.user_channel_id,
                    sc.channel_id.to_string(),
                    sc.expected_usd.0,
                    sc.backing_sats,
                    sc.counterparty.to_string(),
                )
            })
            .collect();
        for (uid, channel_id, expected_usd, backing_sats, counterparty) in syncs {
            if self
                .send_sync_message(
                    ldk,
                    uid,
                    &channel_id,
                    expected_usd,
                    backing_sats,
                    &counterparty,
                )
                .await
            {
                self.startup_sync_pending.remove(&uid);
            }
        }
    }

    /// Self-heal: if the in-memory list is empty (startup/reconnect reconcile skipped on a cold price cache), rebuild it from truth; a populated list is left untouched so a transient empty snapshot can't wipe it.
    pub async fn reconcile_if_empty(&mut self, ldk: &dyn LdkServerCalls, btc_price: f64) {
        if self.stable_channels.is_empty() {
            self.reconcile_from_grpc(ldk, btc_price).await;
        } else {
            self.retry_startup_sync(ldk).await;
        }
    }

    /// On ChannelStateChanged Ready, auto-register the channel as stable at expected_usd=0 if untracked (operator sets a target via EditStableChannel).
    pub async fn handle_channel_ready(
        &mut self,
        channel_id: String,
        user_channel_id: String,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let Some(target_uid) = parse_user_channel_id(&user_channel_id) else {
            stable_channels::audit::audit_event(
                "CHANNEL_READY_UID_UNPARSEABLE",
                serde_json::json!({
                    "channel_id": channel_id,
                    "user_channel_id": user_channel_id,
                }),
            );
            return;
        };
        if self
            .stable_channels
            .iter()
            .any(|sc| sc.user_channel_id == target_uid)
        {
            self.handle_channel_ready_splice(target_uid, ldk, btc_price)
                .await;
            return;
        }

        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r.channels,
            Err(e) => {
                tracing::error!(
                    "[stable] handle_channel_ready: list_channels failed: {}",
                    e
                );
                stable_channels::audit::audit_event(
                    "LDK_CALL_FAILED",
                    serde_json::json!({ "op": "list_channels", "context": "handle_channel_ready", "user_channel_id": user_channel_id, "channel_id": channel_id, "error": e.to_string() }),
                );
                return;
            }
        };
        let Some(c) = channels.into_iter().find(|c| c.channel_id == channel_id) else {
            tracing::warn!(
                "[stable] handle_channel_ready: channel {} not found in list_channels",
                channel_id
            );
            return;
        };

        let (our_sats, their_sats) = channel_peer_balances(&c);

        let new_sc = StableChannel {
            channel_id: ldk_node::lightning::ln::types::ChannelId::from_bytes(
                parse_channel_id_hex(&c.channel_id),
            ),
            user_channel_id: target_uid,
            counterparty: parse_pubkey_hex(&c.counterparty_node_id),
            is_stable_receiver: false,
            expected_usd: USD::from_f64(0.0),
            expected_btc: Bitcoin::from_sats(0),
            stable_receiver_btc: Bitcoin::from_sats(their_sats),
            stable_receiver_usd: USD::from_bitcoin(Bitcoin::from_sats(their_sats), btc_price),
            stable_provider_btc: Bitcoin::from_sats(our_sats),
            stable_provider_usd: USD::from_bitcoin(Bitcoin::from_sats(our_sats), btc_price),
            latest_price: btc_price,
            risk_level: 0,
            payment_made: false,
            timestamp: 0,
            formatted_datetime: String::new(),
            sc_dir: self.data_dir.to_string_lossy().to_string(),
            prices: String::new(),
            onchain_btc: Bitcoin::from_sats(0),
            onchain_usd: USD(0.0),
            note: None,
            native_channel_btc: Bitcoin::from_sats(0),
            backing_sats: 0,
            native_sats: their_sats,
            last_stability_payment: 0,
        };

        if let Err(e) = self.db.save_channel(
            &c.channel_id,
            &format!("{}", target_uid),
            0.0,
            0,
            their_sats,
            None,
        ) {
            tracing::error!(
                "[stable] handle_channel_ready: db.save_channel failed: {}",
                e
            );
            stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({ "op": "save_channel", "context": "handle_channel_ready", "channel_id": channel_id, "user_channel_id": user_channel_id, "error": e.to_string() }),
            );
            return;
        }
        self.stable_channels.push(new_sc);
        stable_channels::audit::audit_event(
            "CHANNEL_READY_TRACKED",
            serde_json::json!({
                "channel_id": channel_id,
                "user_channel_id": user_channel_id,
            }),
        );
    }

    /// On PaymentReceived, route a STABLE_CHANNEL_TLV to the trade handler. A plain payment (no
    /// such TLV) is left to run_tick + reconcile_from_grpc to catch up.
    pub async fn handle_payment_received(
        &mut self,
        custom_records: Vec<CustomTlvRecord>,
        payment_id: Option<String>,
        amount_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        for rec in &custom_records {
            if rec.type_num != stable_channels::constants::STABLE_CHANNEL_TLV_TYPE {
                continue;
            }
            if rec.value.len() > crate::messages::MAX_TLV_VALUE_BYTES {
                stable_channels::audit::audit_event(
                    "TRADE_PARSE_SIGNED_FAILED",
                    serde_json::json!({ "reason": "oversize", "len": rec.value.len() }),
                );
                return;
            }
            let Ok(raw) = std::str::from_utf8(rec.value.as_ref()) else {
                stable_channels::audit::audit_event(
                    "TRADE_PARSE_SIGNED_FAILED",
                    serde_json::json!({ "reason": "utf8" }),
                );
                return;
            };
            stable_channels::audit::audit_event(
                "MESSAGE_RECEIVED",
                serde_json::json!({ "tlv": stable_channels::constants::STABLE_CHANNEL_TLV_TYPE, "payment_id": payment_id.clone() }),
            );
            let raw = raw.to_string();
            if crate::messages::parse_envelope(&raw).is_some() {
                if let Some(pid) = payment_id.as_deref() {
                    if let Err(e) = self.db.record_settlement(pid, "sync") {
                        tracing::error!("[stable] record_settlement (inbound sync) failed: {}", e);
                        stable_channels::audit::audit_event(
                            "DB_WRITE_FAILED",
                            serde_json::json!({ "op": "record_settlement", "kind": "sync", "payment_id": pid, "error": e.to_string() }),
                        );
                    }
                }
                self.handle_trade_message(&raw, amount_msat, ldk, btc_price)
                    .await;
            } else {
                if let Some(pid) = payment_id.as_deref() {
                    if let Err(e) = self.db.record_settlement(pid, "stability") {
                        tracing::error!("[stable] record_settlement (inbound stability) failed: {}", e);
                        stable_channels::audit::audit_event(
                            "DB_WRITE_FAILED",
                            serde_json::json!({ "op": "record_settlement", "kind": "stability", "payment_id": pid, "error": e.to_string() }),
                        );
                    }
                }
                // Tagged-but-not-envelope = a user's stability payment. Reconcile the
                // books NOW: with stale backing_sats the channel still reads above par
                // (double-charge risk) and the balance-truth backstop would misread the
                // user's payment as an unreconciled spend and deduct expected_usd.
                self.reconcile_incoming_stability(payment_id.as_deref(), amount_msat, ldk, btc_price)
                    .await;
            }
            return;
        }
        // No stable TLV: plain receipt — emit audit so it's visible in the log.
        stable_channels::audit::audit_event(
            "PAYMENT_RECEIVED",
            serde_json::json!({ "payment_id": payment_id, "amount_msat": amount_msat }),
        );
    }

    /// Settle the books for an inbound stability payment (user above par paid the LSP).
    ///
    /// The user's side of the channel just dropped by the payment amount, so their
    /// stable value is back at par — reset `backing_sats` to equilibrium
    /// (expected_usd at the current price), exactly mirroring the reset done after
    /// the LSP *sends* a stability payment. `native_sats` is the remainder, so the
    /// user's non-stable sats are untouched by the settlement.
    ///
    /// The event carries no channel id, so the channel is attributed by amount:
    /// the tracked channel whose live user-side balance dropped by the payment
    /// amount (±1 sat for msat rounding) since the last tick snapshot. If the match
    /// is not unique, nothing is mutated and the miss is audited — the tick +
    /// backstop path then handles it as before, but visibly.
    async fn reconcile_incoming_stability(
        &mut self,
        payment_id: Option<&str>,
        amount_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let amount_sats = amount_msat.unwrap_or(0) / 1000;
        if amount_sats == 0 {
            // Sub-sat keysends are control traffic (sync/trade carriers), not settlements.
            return;
        }
        if btc_price <= 0.0 {
            stable_channels::audit::audit_event(
                "STABILITY_RECEIVE_UNATTRIBUTED",
                serde_json::json!({ "payment_id": payment_id, "amount_msat": amount_msat, "reason": "price_cold" }),
            );
            return;
        }
        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r.channels,
            Err(e) => {
                stable_channels::audit::audit_event(
                    "STABILITY_RECEIVE_UNATTRIBUTED",
                    serde_json::json!({ "payment_id": payment_id, "amount_msat": amount_msat, "reason": "list_channels_failed", "error": e.to_string() }),
                );
                return;
            }
        };

        // Attribute by balance drop: (index, live channel, live user-side sats).
        let mut matches: Vec<(usize, &Channel, u64)> = Vec::new();
        for (i, sc) in self.stable_channels.iter().enumerate() {
            if sc.expected_usd.0 < 0.01 {
                continue;
            }
            let Some(c) = channels.iter().find(|c| {
                parse_user_channel_id(&c.user_channel_id) == Some(sc.user_channel_id)
            }) else {
                continue;
            };
            let (_, their_sats) = channel_peer_balances(c);
            let drop = sc.stable_receiver_btc.sats.saturating_sub(their_sats);
            if drop > 0 && drop.abs_diff(amount_sats) <= 1 {
                matches.push((i, c, their_sats));
            }
        }

        if matches.len() != 1 {
            stable_channels::audit::audit_event(
                "STABILITY_RECEIVE_UNATTRIBUTED",
                serde_json::json!({
                    "payment_id": payment_id,
                    "amount_msat": amount_msat,
                    "reason": "no_unique_match",
                    "candidates": matches.len(),
                }),
            );
            return;
        }
        let (idx, live, their_sats) = matches[0];
        let channel_id = live.channel_id.clone();
        let sc = &mut self.stable_channels[idx];
        let uid = sc.user_channel_id;

        sc.latest_price = btc_price;
        sc.stable_receiver_btc = Bitcoin::from_sats(their_sats);
        sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, btc_price);
        // Equilibrium reset, clamped so backing can never exceed the live balance
        // (which would immediately re-trigger the backstop we're protecting against).
        let equilibrium = ((sc.expected_usd.0 / btc_price) * 100_000_000.0) as u64;
        sc.backing_sats = equilibrium.min(their_sats);
        sc.native_sats = their_sats.saturating_sub(sc.backing_sats);
        stable_channels::stable::recompute_native(sc);
        // The drop is settled; make sure the backstop forgets any ticks it counted.
        self.spend_debounce.remove(&uid);

        if let Err(e) = self.db.save_channel(
            &channel_id,
            &format!("{}", uid),
            self.stable_channels[idx].expected_usd.0,
            self.stable_channels[idx].backing_sats,
            self.stable_channels[idx].native_sats,
            self.stable_channels[idx].note.as_deref(),
        ) {
            tracing::error!("[stable] reconcile_incoming save_channel failed: {}", e);
            stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({ "op": "save_channel", "context": "reconcile_incoming", "user_channel_id": format!("{}", uid), "channel_id": channel_id, "error": e.to_string() }),
            );
        }
        stable_channels::audit::audit_event(
            "STABILITY_RECEIVED_RECONCILED",
            serde_json::json!({
                "channel_id": channel_id,
                "user_channel_id": format!("{}", uid),
                "payment_id": payment_id,
                "amount_msat": amount_msat,
                "new_backing_sats": self.stable_channels[idx].backing_sats,
                "new_native_sats": self.stable_channels[idx].native_sats,
            }),
        );
    }

    /// 60s tick: per stable channel, skip below threshold/cooldown/zero-target, then SpontaneousSend a connected peer or push an offline one.
    pub async fn run_tick(
        &mut self,
        ldk: &dyn LdkServerCalls,
        push: &std::sync::Arc<tokio::sync::Mutex<crate::push::PushService>>,
        btc_price: f64,
    ) {
        if btc_price <= 0.0 {
            return;
        }
        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r.channels,
            Err(e) => {
                tracing::warn!("[stable] run_tick: list_channels failed: {}", e);
                return;
            }
        };
        let mut by_user_channel_id: std::collections::HashMap<u128, Channel> =
            std::collections::HashMap::new();
        for c in &channels {
            if let Some(uid) = parse_user_channel_id(&c.user_channel_id) {
                by_user_channel_id.insert(uid, c.clone());
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let percent_threshold = stable_channels::constants::STABILITY_THRESHOLD_PERCENT;
        let dollar_threshold = stable_channels::constants::STABILITY_THRESHOLD_USD;
        let cooldown = stable_channels::constants::STABILITY_PAYMENT_COOLDOWN_SECS as i64;

        // Accepted USD and sat allocations for backstop SYNCs sent after the iter_mut borrow ends.
        let mut backstop_syncs: Vec<(u128, String, f64, u64, String)> = Vec::new();
        const BACKSTOP_DEBOUNCE_TICKS: u8 = 2;

        for sc in self.stable_channels.iter_mut() {
            if sc.expected_usd.0 < 0.01 {
                continue;
            }
            let Some(c) = by_user_channel_id.get(&sc.user_channel_id) else { continue; };

            let (our_sats, their_sats) = channel_peer_balances(c);
            sc.stable_provider_btc = Bitcoin::from_sats(our_sats);
            sc.stable_receiver_btc = Bitcoin::from_sats(their_sats);
            sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, btc_price);
            sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, btc_price);
            sc.latest_price = btc_price;

            // Balance-truth backstop: live balance below backing means a spend went unreconciled (no PaymentForwarded) — deduct + SYNC. Debounced since outbound_capacity excludes in-flight HTLCs.
            let uid = sc.user_channel_id;
            if their_sats < sc.backing_sats {
                let count = {
                    let cnt = self.spend_debounce.entry(uid).or_insert(0);
                    *cnt = cnt.saturating_add(1);
                    *cnt
                };
                if count >= BACKSTOP_DEBOUNCE_TICKS {
                    self.spend_debounce.remove(&uid);
                    if let Some(usd_deducted) =
                        stable_channels::stable::reconcile_outgoing(sc, btc_price)
                    {
                        stable_channels::audit::audit_event(
                            "BACKSTOP_STABLE_DEDUCTED",
                            serde_json::json!({
                                "channel_id": c.channel_id,
                                "user_channel_id": format!("{}", uid),
                                "their_sats": their_sats,
                                "usd_deducted": usd_deducted,
                                "new_expected_usd": sc.expected_usd.0,
                                "new_backing_sats": sc.backing_sats,
                            }),
                        );
                        if let Err(e) = self.db.save_channel(
                            &c.channel_id,
                            &format!("{}", uid),
                            sc.expected_usd.0,
                            sc.backing_sats,
                            sc.native_sats,
                            sc.note.as_deref(),
                        ) {
                            tracing::error!("[stable] backstop save_channel failed: {}", e);
                            stable_channels::audit::audit_event(
                                "DB_WRITE_FAILED",
                                serde_json::json!({ "op": "save_channel", "context": "backstop", "user_channel_id": format!("{}", uid), "channel_id": c.channel_id, "error": e.to_string() }),
                            );
                        }
                        backstop_syncs.push((
                            uid,
                            c.channel_id.clone(),
                            sc.expected_usd.0,
                            sc.backing_sats,
                            sc.counterparty.to_string(),
                        ));
                    }
                }
            } else {
                self.spend_debounce.remove(&uid);
            }

            let stable_usd_value = if sc.backing_sats > 0 {
                (sc.backing_sats as f64 / 100_000_000.0) * btc_price
            } else {
                sc.stable_receiver_usd.0
            };
            let target = sc.expected_usd.0;
            let percent_from_par = (((stable_usd_value - target) / target) * 100.0).abs();
            let dollars_from_par = (stable_usd_value - target).abs();

            if percent_from_par < percent_threshold
                || dollars_from_par < dollar_threshold
            {
                continue;
            }
            if sc.risk_level > stable_channels::constants::MAX_RISK_LEVEL {
                let (lo, lv) = self.stability_throttle.get(&sc.user_channel_id).cloned().unwrap_or_default();
                if stability_should_log(&lo, "high_risk", lv, stable_usd_value, target, dollar_threshold, percent_threshold, false) {
                    stable_channels::audit::audit_event(
                        "STABILITY_SKIP_HIGH_RISK",
                        serde_json::json!({
                            "channel_id": sc.channel_id.to_string(),
                            "user_channel_id": format!("{}", sc.user_channel_id),
                            "risk_level": sc.risk_level,
                        }),
                    );
                    self.stability_throttle.insert(sc.user_channel_id, ("high_risk".to_string(), stable_usd_value));
                }
                continue;
            }
            if now - sc.last_stability_payment < cooldown {
                let (lo, lv) = self.stability_throttle.get(&sc.user_channel_id).cloned().unwrap_or_default();
                if stability_should_log(&lo, "cooldown", lv, stable_usd_value, target, dollar_threshold, percent_threshold, false) {
                    stable_channels::audit::audit_event(
                        "STABILITY_COOLDOWN",
                        serde_json::json!({
                            "channel_id": sc.channel_id.to_string(),
                            "user_channel_id": format!("{}", sc.user_channel_id),
                            "seconds_since_last": now - sc.last_stability_payment,
                            "cooldown_secs": cooldown,
                        }),
                    );
                    self.stability_throttle.insert(sc.user_channel_id, ("cooldown".to_string(), stable_usd_value));
                }
                continue;
            }

            let is_receiver_below_expected = stable_usd_value < target;
            let direction = if is_receiver_below_expected {
                "lsp_to_user"
            } else {
                "user_to_lsp"
            };
            let amount_sats = ((dollars_from_par / btc_price) * 100_000_000.0) as u64;
            let amount_msat = amount_sats.saturating_mul(1000);

            if c.is_usable {
                if is_receiver_below_expected {
                    let send_req = SpontaneousSendRequest {
                        amount_msat,
                        node_id: sc.counterparty.to_string(),
                        route_parameters: None,
                        // Tag as a stability payment so receiving clients can identify it.
                        custom_tlvs: vec![CustomTlvRecord {
                            type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
                            value: vec![1u8].into(),
                        }],
                    };
                    let channel_id_clone = c.channel_id.clone();
                    let user_channel_id_clone = c.user_channel_id.clone();
                    let expected_usd_for_db = sc.expected_usd.0;
                    let note_for_db = sc.note.clone();
                    match ldk.spontaneous_send(send_req).await {
                        Ok(resp) => {
                            if !resp.payment_id.is_empty() {
                                if let Err(e) = self.db.record_settlement_with_channel(
                                    &resp.payment_id,
                                    "stability",
                                    &user_channel_id_clone,
                                ) {
                                    tracing::error!(
                                        "[stable] record_settlement (stability) failed: {}",
                                        e
                                    );
                                    stable_channels::audit::audit_event(
                                        "DB_WRITE_FAILED",
                                        serde_json::json!({ "op": "record_settlement", "kind": "stability", "payment_id": resp.payment_id.clone(), "user_channel_id": user_channel_id_clone.clone(), "channel_id": channel_id_clone.clone(), "error": e.to_string() }),
                                    );
                                }
                            }
                            sc.last_stability_payment = now;
                            // Reset backing_sats to equilibrium so the next tick doesn't re-pay the same drift forever. Native is recomputed on the next balance refresh.
                            sc.backing_sats =
                                ((sc.expected_usd.0 / btc_price) * 100_000_000.0) as u64;
                            let backing = sc.backing_sats;
                            let native = sc.native_sats;
                            if let Err(e) = self.db.save_channel(
                                &channel_id_clone,
                                &user_channel_id_clone,
                                expected_usd_for_db,
                                backing,
                                native,
                                note_for_db.as_deref(),
                            ) {
                                tracing::error!(
                                    "[stable] run_tick: db.save_channel failed: {}",
                                    e
                                );
                                stable_channels::audit::audit_event(
                                    "DB_WRITE_FAILED",
                                    serde_json::json!({ "op": "save_channel", "context": "run_tick_post_send", "channel_id": channel_id_clone, "user_channel_id": user_channel_id_clone, "error": e.to_string() }),
                                );
                            }
                            stable_channels::audit::audit_event(
                                "STABILITY_PAYMENT_SENT",
                                serde_json::json!({
                                    "channel_id": channel_id_clone,
                                    "user_channel_id": user_channel_id_clone.clone(),
                                    "direction": direction,
                                    "amount_msat": amount_msat,
                                    "payment_id": resp.payment_id,
                                    "counterparty": sc.counterparty.to_string(),
                                    "expected_usd": expected_usd_for_db,
                                    "new_backing_sats": sc.backing_sats,
                                }),
                            );
                            self.stability_throttle.insert(sc.user_channel_id, ("payment_sent".to_string(), stable_usd_value));
                        },
                        Err(e) => {
                            tracing::warn!(
                                "[stable] run_tick: spontaneous_send failed: {}",
                                e
                            );
                            stable_channels::audit::audit_event(
                                "STABILITY_PAYMENT_FAILED",
                                serde_json::json!({
                                    "channel_id": channel_id_clone,
                                    "user_channel_id": user_channel_id_clone.clone(),
                                    "direction": direction,
                                    "error": e.to_string(),
                                }),
                            );
                            self.stability_throttle.insert(sc.user_channel_id, ("payment_failed".to_string(), stable_usd_value));
                            // Do not bump last_stability_payment so retry can fire.
                        },
                    }
                } else {
                    // User above par: CHECK_ONLY. The LSP can only push value, not pull, so do nothing here (no cooldown bump).
                    let (lo, lv) = self.stability_throttle.get(&sc.user_channel_id).cloned().unwrap_or_default();
                    if stability_should_log(&lo, "check_only", lv, stable_usd_value, target, dollar_threshold, percent_threshold, true) {
                        stable_channels::audit::audit_event(
                            "STABILITY_CHECK_ONLY",
                            serde_json::json!({
                                "channel_id": c.channel_id,
                                "user_channel_id": c.user_channel_id.clone(),
                                "direction": direction,
                                "stable_usd_value": stable_usd_value,
                                "expected_usd": target,
                            }),
                        );
                        self.stability_throttle.insert(sc.user_channel_id, ("check_only".to_string(), stable_usd_value));
                    }
                }
            } else {
                let mut p = push.lock().await;
                p.notify(&sc.counterparty.to_string(), direction);
                drop(p);
                let key = format!("push_queued:{}", direction);
                let (lo, lv) = self.stability_throttle.get(&sc.user_channel_id).cloned().unwrap_or_default();
                if stability_should_log(&lo, &key, lv, stable_usd_value, target, dollar_threshold, percent_threshold, true) {
                    stable_channels::audit::audit_event(
                        "STABILITY_PUSH_QUEUED",
                        serde_json::json!({
                            "channel_id": c.channel_id,
                            "user_channel_id": c.user_channel_id.clone(),
                            "node_id": sc.counterparty.to_string(),
                            "direction": direction,
                            "stable_usd_value": stable_usd_value,
                            "expected_usd": target,
                        }),
                    );
                    self.stability_throttle.insert(sc.user_channel_id, (key, stable_usd_value));
                }
            }
        }

        for (uid, channel_id, expected_usd, backing_sats, counterparty) in backstop_syncs {
            let sent = self
                .send_sync_message(
                    ldk,
                    uid,
                    &channel_id,
                    expected_usd,
                    backing_sats,
                    &counterparty,
                )
                .await;
            if !sent {
                self.startup_sync_pending.insert(uid);
            }
        }
    }

    /// Sign a SYNC_V1 payload and keysend it (1 msat) to the counterparty in custom TLV 13377331.
    /// Best effort: a send failure is audited and returned to the caller. Allocation state is
    /// unchanged, while the monotonic sync version is durably reserved before signing.
    pub async fn send_sync_message(
        &self,
        ldk: &dyn LdkServerCalls,
        user_channel_id: u128,
        channel_id: &str,
        expected_usd: f64,
        backing_sats: u64,
        counterparty: &str,
    ) -> bool {
        let sync_version = match self.db.next_sync_version(&format!("{}", user_channel_id)) {
            Ok(version) => version,
            Err(e) => {
                stable_channels::audit::audit_event(
                    "SYNC_MESSAGE_FAILED",
                    serde_json::json!({
                        "user_channel_id": format!("{}", user_channel_id),
                        "channel_id": channel_id,
                        "stage": "reserve_version",
                        "error": e.to_string(),
                    }),
                );
                return false;
            }
        };
        let payload = crate::messages::build_sync_payload(
            channel_id,
            &format!("{}", user_channel_id),
            expected_usd,
            backing_sats,
            sync_version,
        );
        let signature = match ldk
            .sign_message(SignMessageRequest {
                message: payload.as_bytes().to_vec().into(),
            })
            .await
        {
            Ok(r) => r.signature,
            Err(e) => {
                stable_channels::audit::audit_event(
                    "SYNC_MESSAGE_FAILED",
                    serde_json::json!({
                        "user_channel_id": format!("{}", user_channel_id),
                        "channel_id": channel_id,
                        "stage": "sign",
                        "error": e.to_string(),
                    }),
                );
                return false;
            }
        };
        let envelope = crate::messages::build_envelope(payload, signature);
        let req = SpontaneousSendRequest {
            amount_msat: 1,
            node_id: counterparty.to_string(),
            route_parameters: None,
            custom_tlvs: vec![CustomTlvRecord {
                type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
                value: envelope.into_bytes().into(),
            }],
        };
        match ldk.spontaneous_send(req).await {
            Ok(resp) => {
                if !resp.payment_id.is_empty() {
                    if let Err(e) = self.db.record_settlement_with_channel(
                        &resp.payment_id,
                        "sync",
                        &format!("{}", user_channel_id),
                    ) {
                        tracing::error!(
                            "[stable] record_settlement (outbound sync) failed: {}",
                            e
                        );
                        stable_channels::audit::audit_event(
                            "DB_WRITE_FAILED",
                            serde_json::json!({ "op": "record_settlement", "kind": "sync", "payment_id": resp.payment_id, "user_channel_id": format!("{}", user_channel_id), "error": e.to_string() }),
                        );
                    }
                }
                stable_channels::audit::audit_event(
                    "SYNC_MESSAGE_SENT",
                    serde_json::json!({
                        "user_channel_id": format!("{}", user_channel_id),
                        "channel_id": channel_id,
                        "expected_usd": expected_usd,
                        "backing_sats": backing_sats,
                        "sync_version": sync_version,
                    }),
                );
                true
            },
            Err(e) => {
                stable_channels::audit::audit_event(
                    "SYNC_MESSAGE_FAILED",
                    serde_json::json!({
                        "user_channel_id": format!("{}", user_channel_id),
                        "channel_id": channel_id,
                        "stage": "send",
                        "error": e.to_string(),
                    }),
                );
                false
            }
        }
    }

    /// On a forward out of a stable channel, reconcile the spend: native BTC first, overflow reduces `expected_usd`.
    pub async fn handle_payment_forwarded(
        &mut self,
        prev_user_channel_id: String,
        next_user_channel_id: Option<String>,
        prev_channel_id: String,
        next_channel_id: String,
        prev_node_id: String,
        next_node_id: String,
        outbound_amount_forwarded_msat: u64,
        fee_msat: u64,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let total_sats = outbound_amount_forwarded_msat.saturating_add(fee_msat) / 1000;
        stable_channels::audit::audit_event(
            "PAYMENT_FORWARDED",
            serde_json::json!({
                "prev_user_channel_id": prev_user_channel_id,
                "next_user_channel_id": next_user_channel_id,
                "prev_channel_id": prev_channel_id,
                "next_channel_id": next_channel_id,
                "prev_node_id": prev_node_id,
                "next_node_id": next_node_id,
                "forwarded_msat": outbound_amount_forwarded_msat,
                "fee_msat": fee_msat,
                "total_sats": total_sats,
            }),
        );

        let Some(target_uid) = parse_user_channel_id(&prev_user_channel_id) else {
            return;
        };
        if !self
            .stable_channels
            .iter()
            .any(|sc| sc.user_channel_id == target_uid)
        {
            return; // forward was not on a stable channel
        }

        // gRPC ForwardedPayment carries no balance, so reconstruct the pre-forward balance as live-post + total.
        let live = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r,
            Err(e) => {
                error!("[forwarded] list_channels gRPC failed: {}", e);
                stable_channels::audit::audit_event(
                    "LDK_CALL_FAILED",
                    serde_json::json!({ "op": "list_channels", "context": "handle_payment_forwarded", "user_channel_id": prev_user_channel_id.clone(), "error": e.to_string() }),
                );
                return;
            }
        };
        let Some(chan) = live
            .channels
            .into_iter()
            .find(|c| parse_user_channel_id(&c.user_channel_id) == Some(target_uid))
        else {
            return; // channel vanished from the server
        };
        let (_, post_user_sats) = channel_peer_balances(&chan);
        let channel_id_hex = chan.channel_id.clone();

        let persisted = {
            let Some(sc) = self
                .stable_channels
                .iter_mut()
                .find(|sc| sc.user_channel_id == target_uid)
            else {
                return;
            };
            if sc.expected_usd.0 <= 0.0 || btc_price <= 0.0 {
                return;
            }

            // Refresh tracked balance to the live value so native_channel_btc stays consistent with native_sats.
            sc.stable_receiver_btc = Bitcoin::from_sats(post_user_sats);
            sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, btc_price);

            let native_before = sc.native_sats;
            let old_expected = sc.expected_usd.0;
            let user_sats_before = post_user_sats.saturating_add(total_sats);
            let counterparty_hex = sc.counterparty.to_string();

            let deducted = if let Some(usd_deducted) = stable_channels::stable::reconcile_forwarded(
                sc,
                user_sats_before,
                total_sats,
                btc_price,
            ) {
                let stable_sats_spent = total_sats.saturating_sub(native_before);
                stable_channels::audit::audit_event(
                    "STABLE_SPEND_DEDUCTED",
                    serde_json::json!({
                        "channel_id": channel_id_hex,
                        "user_channel_id": format!("{}", sc.user_channel_id),
                        "total_sats_spent": total_sats,
                        "native_sats_spent": native_before,
                        "stable_sats_spent": stable_sats_spent,
                        "usd_deducted": usd_deducted,
                        "old_expected_usd": old_expected,
                        "new_expected_usd": sc.expected_usd.0,
                        "btc_price": btc_price,
                    }),
                );
                info!(
                    "[forwarded] channel user_id={} spent {} sats ({} native, {} stable), expected_usd ${:.2} -> ${:.2}",
                    sc.user_channel_id, total_sats, native_before, stable_sats_spent,
                    old_expected, sc.expected_usd.0
                );
                true
            } else {
                // Fully covered by native BTC: reflect the spend in the buffer.
                sc.native_sats = post_user_sats.saturating_sub(sc.backing_sats);
                stable_channels::stable::recompute_native(sc);
                false
            };

            (
                format!("{}", sc.user_channel_id),
                sc.expected_usd.0,
                sc.backing_sats,
                sc.native_sats,
                sc.note.clone(),
                counterparty_hex,
                deducted,
            )
        };

        let (ucid_str, expected_usd_f, backing_sats, native_sats, note, counterparty_hex, deducted) =
            persisted;
        if let Err(e) = self.db.save_channel(
            &channel_id_hex,
            &ucid_str,
            expected_usd_f,
            backing_sats,
            native_sats,
            note.as_deref(),
        ) {
            error!("[forwarded] db.save_channel failed: {}", e);
            stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({ "op": "save_channel", "channel_id": channel_id_hex, "context": "handle_payment_forwarded", "user_channel_id": ucid_str, "error": e.to_string() }),
            );
        }
        if deducted {
            let sent = self
                .send_sync_message(
                    ldk,
                    target_uid,
                    &channel_id_hex,
                    expected_usd_f,
                    backing_sats,
                    &counterparty_hex,
                )
                .await;
            if !sent {
                self.startup_sync_pending.insert(target_uid);
            }
        }
    }

    /// Post-confirmation splice reconcile: refresh the new balance, infer any stable-spend overflow
    /// via reconcile_outgoing, persist, and SYNC the wallet if stable value was deducted.
    async fn handle_channel_ready_splice(
        &mut self,
        uid: u128,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r.channels,
            Err(e) => {
                error!("[splice] list_channels gRPC failed: {}", e);
                stable_channels::audit::audit_event(
                    "LDK_CALL_FAILED",
                    serde_json::json!({ "op": "list_channels", "context": "handle_channel_ready_splice", "user_channel_id": format!("{}", uid), "error": e.to_string() }),
                );
                return;
            }
        };
        let Some(c) = channels
            .into_iter()
            .find(|c| parse_user_channel_id(&c.user_channel_id) == Some(uid))
        else {
            return;
        };
        let (our_sats, their_sats) = channel_peer_balances(&c);
        let channel_id_hex = c.channel_id.clone();
        let new_channel_id_bytes = parse_channel_id_hex(&c.channel_id);

        let persisted = {
            let Some(sc) = self
                .stable_channels
                .iter_mut()
                .find(|sc| sc.user_channel_id == uid)
            else {
                return;
            };
            // Refresh receiver balance from the new snapshot but PRESERVE backing_sats so reconcile_outgoing can infer the overflow.
            sc.channel_id =
                ldk_node::lightning::ln::types::ChannelId::from_bytes(new_channel_id_bytes);
            sc.stable_provider_btc = Bitcoin::from_sats(our_sats);
            sc.stable_receiver_btc = Bitcoin::from_sats(their_sats);
            sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, btc_price);
            sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, btc_price);
            sc.latest_price = btc_price;
            stable_channels::stable::recompute_native(sc);

            let counterparty_hex = sc.counterparty.to_string();
            let usd_deducted = stable_channels::stable::reconcile_outgoing(sc, btc_price);
            if let Some(d) = usd_deducted {
                stable_channels::audit::audit_event(
                    "SPLICE_OUT_STABLE_DEDUCTED",
                    serde_json::json!({
                        "channel_id": channel_id_hex,
                        "user_channel_id": format!("{}", uid),
                        "usd_deducted": d,
                        "new_expected_usd": sc.expected_usd.0,
                    }),
                );
            }
            (
                format!("{}", sc.user_channel_id),
                sc.expected_usd.0,
                sc.backing_sats,
                sc.native_sats,
                sc.note.clone(),
                counterparty_hex,
                usd_deducted.is_some(),
            )
        };

        let (ucid_str, expected_usd_f, backing, native, note, counterparty_hex, deducted) =
            persisted;
        if let Err(e) = self.db.save_channel(
            &channel_id_hex,
            &ucid_str,
            expected_usd_f,
            backing,
            native,
            note.as_deref(),
        ) {
            error!("[splice] db.save_channel failed: {}", e);
            stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({ "op": "save_channel", "context": "handle_channel_ready_splice", "channel_id": channel_id_hex, "user_channel_id": ucid_str, "error": e.to_string() }),
            );
        }
        stable_channels::audit::audit_event(
            "CHANNEL_READY_SPLICE",
            serde_json::json!({ "channel_id": channel_id_hex, "user_channel_id": ucid_str, "deducted": deducted }),
        );
        if deducted {
            let sent = self
                .send_sync_message(
                    ldk,
                    uid,
                    &channel_id_hex,
                    expected_usd_f,
                    backing,
                    &counterparty_hex,
                )
                .await;
            if !sent {
                self.startup_sync_pending.insert(uid);
            }
        }
    }

    /// Parse a TRADE_V1 envelope, verify it against the channel counterparty, validate against
    /// balance, and apply the new USD target. Drops (with an audit line) on any failure.
    pub async fn handle_trade_message(
        &mut self,
        raw: &str,
        amount_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let Some(envelope) = crate::messages::parse_envelope(raw) else {
            stable_channels::audit::audit_event("TRADE_PARSE_SIGNED_FAILED", serde_json::json!({}));
            return;
        };
        let Some(payload) = crate::messages::parse_trade_payload(&envelope.payload) else {
            stable_channels::audit::audit_event("TRADE_PARSE_PAYLOAD_FAILED", serde_json::json!({}));
            return;
        };
        if payload.kind != stable_channels::constants::TRADE_MESSAGE_TYPE {
            stable_channels::audit::audit_event(
                "TRADE_UNHANDLED_TYPE",
                serde_json::json!({ "type": payload.kind, "user_channel_id": payload.user_channel_id.clone() }),
            );
            return;
        }
        if payload.expected_usd < 0.0 || !payload.expected_usd.is_finite() {
            stable_channels::audit::audit_event(
                "TRADE_INVALID_AMOUNT",
                serde_json::json!({ "expected_usd": payload.expected_usd, "user_channel_id": payload.user_channel_id.clone() }),
            );
            return;
        }
        stable_channels::audit::audit_event(
            "TRADE_PARSED_PAYLOAD_OK",
            serde_json::json!({
                "expected_usd": payload.expected_usd,
                "quote_price": payload.quote_price,
                "backing_sats": payload.backing_sats,
                "user_channel_id": payload.user_channel_id.clone(),
                "channel_id": payload.channel_id.clone(),
            }),
        );

        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r.channels,
            Err(e) => {
                error!("[trade] list_channels gRPC failed: {}", e);
                stable_channels::audit::audit_event(
                    "LDK_CALL_FAILED",
                    serde_json::json!({ "op": "list_channels", "context": "handle_trade_message", "user_channel_id": payload.user_channel_id.clone(), "channel_id": payload.channel_id.clone(), "error": e.to_string() }),
                );
                return;
            }
        };
        // channel_id is authoritative when present; user_channel_id is the fallback.
        let chan = channels.into_iter().find(|c| {
            if let Some(cid) = payload.channel_id.as_deref() {
                if c.channel_id == cid {
                    return true;
                }
            }
            if let Some(ucid) = payload.user_channel_id.as_deref() {
                let want = parse_user_channel_id(ucid);
                if want.is_some() && want == parse_user_channel_id(&c.user_channel_id) {
                    return true;
                }
            }
            false
        });
        let Some(chan) = chan else {
            stable_channels::audit::audit_event(
                "TRADE_CHANNEL_NOT_FOUND",
                serde_json::json!({
                    "channel_id": payload.channel_id,
                    "user_channel_id": payload.user_channel_id,
                }),
            );
            return;
        };

        let verify = ldk
            .verify_signature(VerifySignatureRequest {
                message: envelope.payload.as_bytes().to_vec().into(),
                signature: envelope.signature.clone(),
                public_key: chan.counterparty_node_id.clone(),
            })
            .await;
        let valid = matches!(verify, Ok(ref r) if r.valid);
        if !valid {
            stable_channels::audit::audit_event(
                "TRADE_SIGNATURE_INVALID",
                serde_json::json!({ "channel_id": chan.channel_id, "user_channel_id": chan.user_channel_id.clone() }),
            );
            return;
        }
        stable_channels::audit::audit_event(
            "TRADE_SIGNATURE_VALID",
            serde_json::json!({ "channel_id": chan.channel_id, "user_channel_id": chan.user_channel_id.clone() }),
        );

        // Replay protection: reject a signed trade with a stale `ts`; ts==0 means an un-upgraded wallet (no timestamp yet) — accepted until all wallets sign one.
        const TRADE_SIG_WINDOW_SECS: u64 = 300;
        if payload.ts != 0 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now.abs_diff(payload.ts) > TRADE_SIG_WINDOW_SECS {
                stable_channels::audit::audit_event(
                    "TRADE_STALE",
                    serde_json::json!({ "ts": payload.ts, "now": now, "channel_id": chan.channel_id, "user_channel_id": chan.user_channel_id.clone() }),
                );
                return;
            }
        }

        let Some(target_uid) = parse_user_channel_id(&chan.user_channel_id) else {
            stable_channels::audit::audit_event("TRADE_CHANNEL_UID_UNPARSEABLE", serde_json::json!({ "channel_id": chan.channel_id.clone(), "user_channel_id": chan.user_channel_id.clone() }));
            return;
        };
        let Some(current_expected_usd) = self
            .stable_channels
            .iter()
            .find(|sc| sc.user_channel_id == target_uid)
            .map(|sc| sc.expected_usd.0)
        else {
            stable_channels::audit::audit_event(
                "TRADE_STABLE_ENTRY_NOT_FOUND",
                serde_json::json!({ "channel_id": chan.channel_id.clone(), "user_channel_id": format!("{}", target_uid) }),
            );
            return;
        };

        let fee_price = payload.quote_price.unwrap_or(btc_price);
        let Some(expected_fee_msat) = expected_trade_fee_msat(
            current_expected_usd,
            payload.expected_usd,
            fee_price,
        ) else {
            stable_channels::audit::audit_event(
                "TRADE_FEE_INVALID",
                serde_json::json!({
                    "reason": "fee inputs are invalid",
                    "old_expected_usd": current_expected_usd,
                    "new_expected_usd": payload.expected_usd,
                    "fee_price": fee_price,
                    "amount_msat": amount_msat,
                    "channel_id": chan.channel_id.clone(),
                    "user_channel_id": chan.user_channel_id.clone(),
                }),
            );
            return;
        };
        let tolerance_msat =
            trade_fee_tolerance_msat(expected_fee_msat, payload.quote_price.is_some());
        let fee_matches = amount_msat
            .map(|actual| actual.abs_diff(expected_fee_msat) <= tolerance_msat)
            .unwrap_or(false);
        if !fee_matches {
            stable_channels::audit::audit_event(
                "TRADE_FEE_INVALID",
                serde_json::json!({
                    "reason": if amount_msat.is_some() { "incorrect amount" } else { "missing amount" },
                    "actual_fee_msat": amount_msat,
                    "expected_fee_msat": expected_fee_msat,
                    "tolerance_msat": tolerance_msat,
                    "old_expected_usd": current_expected_usd,
                    "new_expected_usd": payload.expected_usd,
                    "fee_price": fee_price,
                    "channel_id": chan.channel_id.clone(),
                    "user_channel_id": chan.user_channel_id.clone(),
                }),
            );
            return;
        }
        let (our_sats, their_sats) = channel_peer_balances(&chan);
        let new_expected = payload.expected_usd;
        let signed_allocation = match (payload.quote_price, payload.backing_sats) {
            (Some(quote_price), Some(backing_sats)) => {
                if payload.ts == 0
                    || !quote_price.is_finite()
                    || quote_price <= 0.0
                    || !btc_price.is_finite()
                    || btc_price <= 0.0
                {
                    stable_channels::audit::audit_event(
                        "TRADE_INVALID_QUOTE",
                        serde_json::json!({
                            "quote_price": quote_price,
                            "lsp_price": btc_price,
                            "ts": payload.ts,
                            "channel_id": chan.channel_id.clone(),
                            "user_channel_id": chan.user_channel_id.clone(),
                        }),
                    );
                    return;
                }

                // Both peers run their own price feed. Admit small observation-time differences,
                // but reject a quote far enough away to change the economic trade materially.
                let quote_deviation_percent =
                    ((quote_price - btc_price) / btc_price * 100.0).abs();
                if quote_deviation_percent > MAX_TRADE_QUOTE_DEVIATION_PERCENT {
                    stable_channels::audit::audit_event(
                        "TRADE_QUOTE_DEVIATION_EXCEEDED",
                        serde_json::json!({
                            "quote_price": quote_price,
                            "lsp_price": btc_price,
                            "deviation_percent": quote_deviation_percent,
                            "maximum_percent": MAX_TRADE_QUOTE_DEVIATION_PERCENT,
                            "channel_id": chan.channel_id.clone(),
                            "user_channel_id": chan.user_channel_id.clone(),
                        }),
                    );
                    return;
                }

                let signed_backing_usd = backing_sats as f64 / 100_000_000.0 * quote_price;
                let allocation_delta_usd = (signed_backing_usd - new_expected).abs();
                let zero_allocation_is_consistent = if new_expected < 0.01 {
                    backing_sats == 0
                } else {
                    backing_sats > 0
                };
                if backing_sats > their_sats
                    || !zero_allocation_is_consistent
                    || allocation_delta_usd
                        > stable_channels::constants::STABILITY_THRESHOLD_USD
                {
                    stable_channels::audit::audit_event(
                        "TRADE_ALLOCATION_INVALID",
                        serde_json::json!({
                            "signed_backing_sats": backing_sats,
                            "signed_backing_usd": signed_backing_usd,
                            "allocation_delta_usd": allocation_delta_usd,
                            "receiver_sats": their_sats,
                            "quote_price": quote_price,
                            "channel_id": chan.channel_id.clone(),
                            "user_channel_id": chan.user_channel_id.clone(),
                        }),
                    );
                    return;
                }

                Some((quote_price, backing_sats, quote_deviation_percent))
            }
            (None, None) => None,
            _ => {
                stable_channels::audit::audit_event(
                    "TRADE_ALLOCATION_INCOMPLETE",
                    serde_json::json!({
                        "quote_price": payload.quote_price,
                        "backing_sats": payload.backing_sats,
                        "channel_id": chan.channel_id.clone(),
                        "user_channel_id": chan.user_channel_id.clone(),
                    }),
                );
                return;
            }
        };

        let validation_price = signed_allocation
            .map(|(quote_price, _, _)| quote_price)
            .unwrap_or(btc_price);
        let receiver_usd =
            USD::from_bitcoin(Bitcoin::from_sats(their_sats), validation_price).0;
        // Epsilon absorbs f64 boundary rounding so a spend-driven push landing at ~receiver_usd is admitted; residual drift self-heals.
        let ceiling = receiver_usd + stable_channels::constants::STABILITY_THRESHOLD_USD;
        if new_expected > ceiling {
            stable_channels::audit::audit_event(
                "TRADE_EXCEEDS_BALANCE",
                serde_json::json!({ "requested_usd": new_expected, "receiver_usd": receiver_usd, "user_channel_id": format!("{}", target_uid), "channel_id": chan.channel_id.clone() }),
            );
            return;
        }
        let channel_id_hex = chan.channel_id.clone();

        let persisted = {
            let Some(sc) = self
                .stable_channels
                .iter_mut()
                .find(|sc| sc.user_channel_id == target_uid)
            else {
                stable_channels::audit::audit_event(
                    "TRADE_STABLE_ENTRY_NOT_FOUND",
                    serde_json::json!({ "channel_id": channel_id_hex, "user_channel_id": format!("{}", target_uid) }),
                );
                return;
            };
            sc.stable_provider_btc = Bitcoin::from_sats(our_sats);
            sc.stable_receiver_btc = Bitcoin::from_sats(their_sats);
            sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, btc_price);
            sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, btc_price);
            sc.latest_price = btc_price;
            if let Some((_, backing_sats, _)) = signed_allocation {
                stable_channels::stable::apply_trade_allocation(
                    sc,
                    new_expected,
                    backing_sats,
                );
            } else {
                stable_channels::stable::apply_trade(sc, new_expected, btc_price);
            }
            (
                format!("{}", sc.user_channel_id),
                sc.expected_usd.0,
                sc.backing_sats,
                sc.native_sats,
                sc.note.clone(),
                sc.counterparty.to_string(),
            )
        };

        let (ucid_str, expected_usd_f, backing, native, note, counterparty) = persisted;
        if let Err(e) = self.db.save_channel(
            &channel_id_hex,
            &ucid_str,
            expected_usd_f,
            backing,
            native,
            note.as_deref(),
        ) {
            error!("[trade] db.save_channel failed: {}", e);
            stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({ "op": "save_channel", "context": "handle_trade_message", "channel_id": channel_id_hex, "user_channel_id": ucid_str, "error": e.to_string() }),
            );
            return;
        }
        stable_channels::audit::audit_event(
            "TRADE_APPLIED",
            serde_json::json!({
                "channel_id": channel_id_hex,
                "user_channel_id": ucid_str,
                "new_expected_usd": expected_usd_f,
                "backing_sats": backing,
                "native_sats": native,
                "quote_price": signed_allocation.map(|(price, _, _)| price),
                "lsp_price": btc_price,
                "quote_deviation_percent": signed_allocation.map(|(_, _, deviation)| deviation),
            }),
        );
        let sent = self
            .send_sync_message(
                ldk,
                target_uid,
                &channel_id_hex,
                expected_usd_f,
                backing,
                &counterparty,
            )
            .await;
        if !sent {
            self.startup_sync_pending.insert(target_uid);
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

/// Parse an LDK Server user_channel_id (decimal u128::to_string) to u128, with a hex fallback for legacy values.
fn parse_user_channel_id(s: &str) -> Option<u128> {
    s.parse::<u128>()
        .ok()
        .or_else(|| u128::from_str_radix(s.trim_start_matches("0x"), 16).ok())
}

/// Whether a throttled stability event should log this tick: on outcome change, or (if tracking value) a significant value move.
pub(crate) fn stability_should_log(
    last_outcome: &str, outcome: &str,
    last_value: f64, value: f64, target: f64,
    usd_threshold: f64, pct_threshold: f64,
    track_value: bool,
) -> bool {
    if last_outcome != outcome { return true; }
    if !track_value { return false; }
    let d = (value - last_value).abs();
    d > usd_threshold && (d / target * 100.0) > pct_threshold
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
    use super::*;
    use ldk_server_client::error::LdkServerErrorCode;
    use ldk_server_client::ldk_server_grpc::api::{
        ListForwardedPaymentsRequest, ListForwardedPaymentsResponse, GetBalancesRequest,
        GetBalancesResponse, ListPeersRequest, ListPeersResponse,
    };
    use ldk_server_client::ldk_server_grpc::types::{
        Channel as GrpcChannel, ForwardedPayment as GrpcForwardedPayment, HtlcLocator,
        PendingSweepBalance as GrpcPendingSweepBalance, Peer as GrpcPeer,
    };
    use std::sync::Mutex as StdMutex;
    use tempfile::tempdir;

    static AUDIT_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub struct FakeLdkServer {
        pub channels: StdMutex<Vec<GrpcChannel>>,
        pub sends: StdMutex<Vec<SpontaneousSendRequest>>,
        pub send_should_fail: bool,
        pub verify_should_pass: bool,
        pub signature: String,
        pub sign_calls: StdMutex<Vec<Vec<u8>>>,
        pub forwarded: StdMutex<Vec<GrpcForwardedPayment>>,
        pub sweeps: StdMutex<Vec<GrpcPendingSweepBalance>>,
        pub peers: StdMutex<Vec<GrpcPeer>>,
    }

    impl FakeLdkServer {
        pub fn new(channels: Vec<GrpcChannel>) -> Self {
            Self {
                channels: StdMutex::new(channels),
                sends: StdMutex::new(Vec::new()),
                send_should_fail: false,
                verify_should_pass: true,
                signature: "fake-sig".to_string(),
                sign_calls: StdMutex::new(Vec::new()),
                forwarded: StdMutex::new(Vec::new()),
                sweeps: StdMutex::new(Vec::new()),
                peers: StdMutex::new(Vec::new()),
            }
        }
        pub fn with_send_failure(mut self) -> Self {
            self.send_should_fail = true;
            self
        }
        pub fn with_verify_failure(mut self) -> Self {
            self.verify_should_pass = false;
            self
        }
        pub fn with_forwarded(self, f: Vec<GrpcForwardedPayment>) -> Self { *self.forwarded.lock().unwrap() = f; self }
        pub fn with_sweeps(self, s: Vec<GrpcPendingSweepBalance>) -> Self { *self.sweeps.lock().unwrap() = s; self }
        pub fn with_peers(self, p: Vec<GrpcPeer>) -> Self { *self.peers.lock().unwrap() = p; self }
    }

    #[async_trait]
    impl LdkServerCalls for FakeLdkServer {
        async fn list_channels(
            &self,
            _req: ListChannelsRequest,
        ) -> Result<ListChannelsResponse, LdkServerError> {
            Ok(ListChannelsResponse {
                channels: self.channels.lock().unwrap().clone(),
            })
        }
        async fn spontaneous_send(
            &self,
            req: SpontaneousSendRequest,
        ) -> Result<SpontaneousSendResponse, LdkServerError> {
            if self.send_should_fail {
                return Err(LdkServerError::new(
                    LdkServerErrorCode::LightningError,
                    "fake send failure".to_string(),
                ));
            }
            self.sends.lock().unwrap().push(req);
            Ok(SpontaneousSendResponse {
                payment_id: "fake-payment-id".to_string(),
            })
        }
        async fn sign_message(
            &self,
            req: SignMessageRequest,
        ) -> Result<SignMessageResponse, LdkServerError> {
            self.sign_calls.lock().unwrap().push(req.message.to_vec());
            Ok(SignMessageResponse {
                signature: self.signature.clone(),
            })
        }
        async fn verify_signature(
            &self,
            _req: VerifySignatureRequest,
        ) -> Result<VerifySignatureResponse, LdkServerError> {
            Ok(VerifySignatureResponse {
                valid: self.verify_should_pass,
            })
        }
        async fn list_forwarded_payments(&self, _req: ListForwardedPaymentsRequest)
            -> Result<ListForwardedPaymentsResponse, LdkServerError> {
            Ok(ListForwardedPaymentsResponse { forwarded_payments: self.forwarded.lock().unwrap().clone(), next_page_token: None })
        }
        async fn get_balances(&self, _req: GetBalancesRequest)
            -> Result<GetBalancesResponse, LdkServerError> {
            Ok(GetBalancesResponse { pending_balances_from_channel_closures: self.sweeps.lock().unwrap().clone(), ..Default::default() })
        }
        async fn list_peers(&self, _req: ListPeersRequest)
            -> Result<ListPeersResponse, LdkServerError> {
            Ok(ListPeersResponse { peers: self.peers.lock().unwrap().clone() })
        }
    }

    #[tokio::test]
    async fn fake_serves_forwarded_and_peers_fixtures() {
        let fake = FakeLdkServer::new(vec![]).with_peers(vec![GrpcPeer {
            node_id: "02aa".into(),
            address: "1.2.3.4:9735".into(),
            is_persisted: true,
            is_connected: true,
        }]);
        let peers = fake.list_peers(ListPeersRequest {}).await.unwrap().peers;
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].node_id, "02aa");
        let fwd = fake.list_forwarded_payments(ListForwardedPaymentsRequest { page_token: None })
            .await.unwrap().forwarded_payments;
        assert!(fwd.is_empty());
    }

    pub fn make_channel(
        channel_id: &str,
        user_channel_id: &str,
        counterparty: &str,
        value_sats: u64,
        outbound_msat: u64,
        is_usable: bool,
    ) -> GrpcChannel {
        let remote_sats = value_sats.saturating_sub(outbound_msat / 1000);
        GrpcChannel {
            channel_id: channel_id.to_string(),
            counterparty_node_id: counterparty.to_string(),
            user_channel_id: user_channel_id.to_string(),
            unspendable_punishment_reserve: Some(0),
            counterparty_unspendable_punishment_reserve: 0,
            channel_value_sats: value_sats,
            outbound_capacity_msat: outbound_msat,
            inbound_capacity_msat: remote_sats.saturating_mul(1000),
            is_usable,
            is_channel_ready: true,
            is_outbound: true,
            ..Default::default()
        }
    }

    pub fn make_manager() -> StableChannelManager {
        let dir = tempdir().unwrap();
        let db_path = dir.path().to_path_buf();
        // Keep the temp dir alive for the test process so sqlite isn't backed by a deleted directory.
        std::mem::forget(dir);
        let db = stable_channels::db::Database::open(&db_path).unwrap();
        StableChannelManager::new(std::sync::Arc::new(db), db_path)
    }

    pub const COUNTERPARTY_HEX: &str =
        "02465ed5be53d04fde66c9418ff14a5f2267723810176c9212b722e542dc1afb1b";
    pub const USER_CHANNEL_ID_HEX: &str = "00000000000000000000000000000001";
    // A realistic 39-digit decimal user_channel_id. Parsed as hex it overflows u128 (the bug this guards).
    pub const USER_CHANNEL_ID_DECIMAL: &str = "189476124653200987495269098788434301048";
    pub const CHANNEL_ID_HEX: &str =
        "f9634c603646c60b0df9f07c3011708652125915c80300a9bb8fb37c9c0de05b";

    #[tokio::test]
    async fn handle_channel_closed_removes_record() {
        let mut mgr = make_manager();
        // Seed an existing record so handle_channel_closed has something to remove.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(10.0), Some("note".to_string()),
            &fake as &dyn LdkServerCalls, 100_000.0,
        ).await;
        assert_eq!(mgr.stable_channels.len(), 1);

        mgr.handle_channel_closed("".to_string(), USER_CHANNEL_ID_HEX.to_string(), None, None, 0, None);
        assert_eq!(mgr.stable_channels.len(), 0);
    }

    #[tokio::test]
    async fn reconcile_drops_channels_no_longer_on_server() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(10.0), None,
            &fake as &dyn LdkServerCalls, 100_000.0,
        ).await;
        assert_eq!(mgr.stable_channels.len(), 1);

        // LDK Server no longer reports the channel.
        let empty_server = FakeLdkServer::new(vec![]);
        mgr.reconcile_from_grpc(&empty_server as &dyn LdkServerCalls, 100_000.0).await;
        assert_eq!(mgr.stable_channels.len(), 0);
    }

    #[tokio::test]
    async fn reconcile_refreshes_known_channel() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(10.0), None,
            &fake as &dyn LdkServerCalls, 100_000.0,
        ).await;

        // Same channel, different balance: outbound drops from 50_000 to 30_000 sats.
        let fake2 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 30_000_000, true,
        )]);
        mgr.reconcile_from_grpc(&fake2 as &dyn LdkServerCalls, 100_000.0).await;
        assert_eq!(mgr.stable_channels.len(), 1);
        // outbound dropped from 50_000 to 30_000 sats; receiver got 20_000 more.
        assert_eq!(mgr.stable_channels[0].stable_receiver_btc.sats, 70_000);
    }

    #[tokio::test]
    async fn reconcile_hydrates_fresh_manager_from_db() {
        // Simulate a restart: empty in-memory Vec but a persisted stable channel row in sqlite.
        let mut mgr = make_manager();
        // Persist a row directly (bypass the in-memory Vec) to mimic a prior session.
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                25.0,
                40_000,
                10_000,
                Some("persisted"),
            )
            .unwrap();
        assert_eq!(mgr.stable_channels.len(), 0, "fresh manager starts empty");

        // The live LDK Server still reports the channel.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.reconcile_from_grpc(&fake as &dyn LdkServerCalls, 100_000.0).await;

        assert_eq!(mgr.stable_channels.len(), 1, "channel must be hydrated from db");
        let sc = &mgr.stable_channels[0];
        assert_eq!(sc.expected_usd.0, 25.0, "persisted expected_usd preserved");
        assert_eq!(sc.backing_sats, 40_000, "persisted backing_sats preserved");
        assert_eq!(sc.note.as_deref(), Some("persisted"), "persisted note preserved");
        assert_eq!(sc.counterparty.to_string(), COUNTERPARTY_HEX, "counterparty resolved from live channel");
        assert_eq!(
            fake.sends.lock().unwrap().len(),
            1,
            "startup hydration must resync persisted allocation"
        );
        assert!(mgr.startup_sync_pending.is_empty());
    }

    #[tokio::test]
    async fn startup_reconcile_retries_failed_sync() {
        let mut mgr = make_manager();
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                25.0,
                40_000,
                10_000,
                None,
            )
            .unwrap();
        let channels = vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )];

        let failing = FakeLdkServer::new(channels.clone()).with_send_failure();
        mgr.reconcile_from_grpc(&failing as &dyn LdkServerCalls, 100_000.0)
            .await;
        assert!(!mgr.startup_sync_pending.is_empty());

        let restored = FakeLdkServer::new(channels);
        mgr.reconcile_from_grpc(&restored as &dyn LdkServerCalls, 100_000.0)
            .await;
        assert_eq!(restored.sends.lock().unwrap().len(), 1);
        assert!(mgr.startup_sync_pending.is_empty());

        mgr.reconcile_from_grpc(&restored as &dyn LdkServerCalls, 100_000.0)
            .await;
        assert_eq!(
            restored.sends.lock().unwrap().len(),
            1,
            "successful startup sync is sent only once"
        );
    }

    #[tokio::test]
    async fn startup_reconcile_defers_sync_when_live_balance_is_below_backing() {
        let mut mgr = make_manager();
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                25.0,
                60_000,
                0,
                None,
            )
            .unwrap();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);

        mgr.reconcile_from_grpc(&fake as &dyn LdkServerCalls, 100_000.0)
            .await;

        assert!(fake.sends.lock().unwrap().is_empty());
        assert!(!mgr.startup_sync_pending.is_empty());
    }

    #[tokio::test]
    async fn startup_reconcile_syncs_coherent_channels_independently() {
        let mut mgr = make_manager();
        let second_channel_id = "22".repeat(32);
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                25.0,
                60_000,
                0,
                None,
            )
            .unwrap();
        mgr.db
            .save_channel(&second_channel_id, "2", 20.0, 40_000, 10_000, None)
            .unwrap();
        let fake = FakeLdkServer::new(vec![
            make_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                COUNTERPARTY_HEX,
                100_000,
                50_000_000,
                true,
            ),
            make_channel(
                &second_channel_id,
                "2",
                COUNTERPARTY_HEX,
                100_000,
                50_000_000,
                true,
            ),
        ]);

        mgr.reconcile_from_grpc(&fake as &dyn LdkServerCalls, 100_000.0)
            .await;

        assert_eq!(fake.sends.lock().unwrap().len(), 1);
        assert!(mgr
            .startup_sync_pending
            .contains(&USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap()));
        assert!(!mgr.startup_sync_pending.contains(&2));
    }

    #[tokio::test]
    async fn reconcile_if_empty_hydrates_then_leaves_populated_untouched() {
        // Simulate the cold-start skip: empty in-memory Vec, persisted row, live channel present.
        let mut mgr = make_manager();
        mgr.db.save_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, 25.0, 40_000, 10_000, Some("persisted")).unwrap();
        assert_eq!(mgr.stable_channels.len(), 0, "fresh manager starts empty");

        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        // Empty vec -> self-heal repopulates from truth.
        mgr.reconcile_if_empty(&fake as &dyn LdkServerCalls, 100_000.0).await;
        assert_eq!(mgr.stable_channels.len(), 1, "empty list is hydrated");

        // Populated vec -> guard skips reconcile, so a transient empty snapshot can't wipe it.
        let empty_server = FakeLdkServer::new(vec![]);
        mgr.reconcile_if_empty(&empty_server as &dyn LdkServerCalls, 100_000.0).await;
        assert_eq!(mgr.stable_channels.len(), 1, "populated list is left untouched");
    }

    #[tokio::test]
    async fn handle_channel_ready_auto_registers_new_channel() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;
        assert_eq!(mgr.stable_channels.len(), 1);
        assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
    }

    #[tokio::test]
    async fn handle_channel_ready_is_idempotent() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;
        assert_eq!(mgr.stable_channels.len(), 1);
    }

    #[tokio::test]
    async fn payment_received_trade_tlv_applies() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 8.0);
        let records = vec![CustomTlvRecord {
            type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
            value: env.into_bytes().into(),
        }];
        let fee_msat = expected_trade_fee_msat(0.0, 8.0, 100_000.0).unwrap();
        mgr.handle_payment_received(
            records,
            Some("pay_test_1".to_string()),
            Some(fee_msat),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 8.0).abs() < 1e-6);
        assert_eq!(
            mgr.db.list_settlements().unwrap(),
            vec![
                ("pay_test_1".to_string(), "sync".to_string()),
                ("fake-payment-id".to_string(), "sync".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn payment_received_no_tlv_is_noop() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![]);
        seed_channel(&mut mgr, 1u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 5.0, 5_000, 45_000, 50_000, 100_000.0);

        mgr.handle_payment_received(vec![], None, None, &fake as &dyn LdkServerCalls, 100_000.0).await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 5.0).abs() < 1e-6); // untouched
        assert!(mgr.db.list_settlements().unwrap().is_empty());
    }

    #[tokio::test]
    async fn payment_received_marker_records_settlement() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        let records = vec![CustomTlvRecord {
            type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
            value: vec![1u8].into(),
        }];
        let before = mgr.stable_channels[0].expected_usd.0;
        mgr.handle_payment_received(records, Some("pay_settlement_1".to_string()), None, &fake as &dyn LdkServerCalls, 100_000.0).await;

        // the 1-byte marker is not an envelope, so it records stability and applies no trade
        assert_eq!(
            mgr.db.list_settlements().unwrap(),
            vec![("pay_settlement_1".to_string(), "stability".to_string())]
        );
        assert_eq!(mgr.stable_channels[0].expected_usd.0, before);
    }

    // Seed a stable channel: 100k value, 50k user side, $10 at $100k/BTC, giving backing 10k + native 40k.
    async fn seed_forwarded_fixture() -> (StableChannelManager, FakeLdkServer) {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(10.0), None,
            &fake as &dyn LdkServerCalls, 100_000.0,
        ).await;
        assert_eq!(mgr.stable_channels.len(), 1);
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);
        assert_eq!(mgr.stable_channels[0].native_sats, 40_000);
        (mgr, fake)
    }

    #[tokio::test]
    async fn handle_payment_forwarded_deducts_stable_when_spend_exceeds_native() {
        let (mut mgr, fake) = seed_forwarded_fixture().await;
        // Forward 45k out: 40k native + 5k stable. Post-forward user side = 5_000 (LSP 95_000).
        *fake.channels.lock().unwrap() = vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX,
            100_000, 95_000_000, true,
        )];

        mgr.handle_payment_forwarded(
            USER_CHANNEL_ID_DECIMAL.to_string(),
            Some("next-ucid-1".to_string()),
            "prev-chan-1".to_string(),
            "next-chan-1".to_string(),
            "prev-node-1".to_string(),
            "next-node-1".to_string(),
            45_000_000, // outbound_amount_forwarded_msat
            0,          // fee_msat
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;

        // 5_000 overflow sats * $100k / 1e8 = $5.00 deducted: $10 -> $5.
        let exp = mgr.stable_channels[0].expected_usd.0;
        assert!((exp - 5.0).abs() < 0.01, "expected_usd should drop to ~5.0, got {}", exp);
        // native_sats and native_channel_btc must agree after reconcile.
        assert_eq!(
            mgr.stable_channels[0].native_channel_btc.sats,
            mgr.stable_channels[0].native_sats,
            "native_channel_btc must match native_sats after a forward",
        );
    }

    #[tokio::test]
    async fn forwarded_overflow_uses_remote_capacity_not_commitment_fee_residual() {
        let mut mgr = make_manager();
        let uid = 189476124653200987495269098788434301048u128;
        // Exact production regression: the 151,958-sat funding output has 659 sats reserved for
        // the funder's commitment fee. After the forward, the remote user owns 67,595 sats and
        // the LSP owns 83,704; channel_value - LSP would incorrectly report 68,254 for the user.
        let mut channel = make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            151_958,
            83_704_000,
            true,
        );
        channel.inbound_capacity_msat = 67_595_000;
        let fake = FakeLdkServer::new(vec![channel]);
        seed_channel(
            &mut mgr,
            uid,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            46.4,
            70_433,
            4_740,
            75_173,
            65_877.7,
        );

        mgr.handle_payment_forwarded(
            USER_CHANNEL_ID_DECIMAL.to_string(),
            Some("next-ucid-production-regression".to_string()),
            CHANNEL_ID_HEX.to_string(),
            "next-channel".to_string(),
            COUNTERPARTY_HEX.to_string(),
            "next-node".to_string(),
            7_578_000,
            0,
            &fake as &dyn LdkServerCalls,
            66_000.96,
        )
        .await;

        let expected = 46.4 - (2_838.0 / 100_000_000.0 * 66_000.96);
        let sc = &mgr.stable_channels[0];
        assert!((sc.expected_usd.0 - expected).abs() < 1e-9);
        assert_eq!(sc.stable_receiver_btc.sats, 67_595);
    }

    #[tokio::test]
    async fn handle_payment_forwarded_covered_by_native_keeps_expected_usd() {
        let (mut mgr, fake) = seed_forwarded_fixture().await;
        // Forward 20k out, fully covered by the 40k native buffer. Post-forward user side = 30_000 (LSP 70_000).
        *fake.channels.lock().unwrap() = vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX,
            100_000, 70_000_000, true,
        )];

        mgr.handle_payment_forwarded(
            USER_CHANNEL_ID_DECIMAL.to_string(),
            Some("next-ucid-2".to_string()),
            "prev-chan-2".to_string(),
            "next-chan-2".to_string(),
            "prev-node-2".to_string(),
            "next-node-2".to_string(),
            20_000_000,
            0,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;

        let exp = mgr.stable_channels[0].expected_usd.0;
        assert!((exp - 10.0).abs() < 0.01, "expected_usd must stay ~10.0, got {}", exp);
        // Native buffer shrank by the spend: 40_000 - 20_000 = 20_000.
        assert_eq!(mgr.stable_channels[0].native_sats, 20_000);
        // native_sats and native_channel_btc must agree after reconcile.
        assert_eq!(
            mgr.stable_channels[0].native_channel_btc.sats,
            mgr.stable_channels[0].native_sats,
            "native_channel_btc must match native_sats after a forward",
        );
    }

    #[tokio::test]
    async fn handle_payment_forwarded_untracked_channel_is_noop() {
        let mut mgr = make_manager();
        // Untracked channel: a forward on an unknown channel must not panic or invent a record.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.handle_payment_forwarded(
            USER_CHANNEL_ID_DECIMAL.to_string(),
            None,
            "prev-chan-3".to_string(),
            "next-chan-3".to_string(),
            "prev-node-3".to_string(),
            "next-node-3".to_string(),
            45_000_000,
            0,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;
        assert!(mgr.stable_channels.is_empty());
    }

    #[tokio::test]
    async fn forwarded_deduction_sends_sync() {
        let mut mgr = make_manager();
        // Post-forward channel snapshot: their = 5,000 sats (our 95k via outbound 95M msat).
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 95_000_000, true,
        )]);
        // expected $10 -> backing 10,000; native 40,000; receiver 50,000 at $100k.
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 10.0, 10_000, 40_000, 50_000, 100_000.0);

        // Forward 45,000 sats out: pre = 5,000 + 45,000 = 50,000, native 40,000, overflow 5,000 = $5.
        mgr.handle_payment_forwarded(
            USER_CHANNEL_ID_DECIMAL.to_string(),
            Some("next-ucid-4".to_string()),
            "prev-chan-4".to_string(),
            "next-chan-4".to_string(),
            "prev-node-4".to_string(),
            "next-node-4".to_string(),
            45_000_000, // outbound_amount_forwarded_msat
            0,          // fee_msat
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "a SYNC should be sent after a stable deduction");
        assert_eq!(sends[0].amount_msat, 1);
        assert_eq!(
            sends[0].custom_tlvs[0].type_num,
            stable_channels::constants::STABLE_CHANNEL_TLV_TYPE
        );
    }

    #[tokio::test]
    async fn payment_forwarded_audit_records_both_legs() {
        let _g = AUDIT_TEST_GUARD.lock().unwrap();
        let (mut mgr, fake) = seed_forwarded_fixture().await;
        *fake.channels.lock().unwrap() = vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX,
            100_000, 95_000_000, true,
        )];
        stable_channels::audit::enable_test_capture();
        mgr.handle_payment_forwarded(
            USER_CHANNEL_ID_DECIMAL.to_string(),
            Some("outbound-ucid".to_string()),
            "prev-chan-hex".to_string(),
            "next-chan-hex".to_string(),
            "prev-node-pubkey".to_string(),
            "next-node-pubkey".to_string(),
            45_000_000,
            0,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let (_, data) = events.iter().find(|(e, _)| e == "PAYMENT_FORWARDED")
            .expect("PAYMENT_FORWARDED must be emitted");
        assert_eq!(data["prev_user_channel_id"], USER_CHANNEL_ID_DECIMAL, "inbound leg must be recorded");
        assert_eq!(data["next_user_channel_id"], "outbound-ucid", "outbound leg must be recorded");
        assert_eq!(data["prev_node_id"], "prev-node-pubkey");
        assert_eq!(data["next_node_id"], "next-node-pubkey");
    }

    #[tokio::test]
    async fn run_tick_skips_zero_target() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        // expected_usd defaulted to 0; tick must not attempt any send.
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;

        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(
                &crate::config::PushConfig::default(),
                mgr.data_dir(),
            ),
        ));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0).await;
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn run_tick_skips_cooldown_active() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(10.0), None,
            &fake as &dyn LdkServerCalls, 100_000.0,
        ).await;
        // Pretend we just paid: bump last_stability_payment to "now".
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        mgr.stable_channels[0].last_stability_payment = now;

        // Force a large drift by swapping in a channel with no outbound capacity.
        let fake2 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 0, true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(
                &crate::config::PushConfig::default(),
                mgr.data_dir(),
            ),
        ));
        mgr.run_tick(&fake2 as &dyn LdkServerCalls, &push, 100_000.0).await;
        assert!(
            fake2.sends.lock().unwrap().is_empty(),
            "cooldown should suppress send"
        );
    }

    #[tokio::test]
    async fn run_tick_sends_when_connected_and_drift_exceeds_threshold() {
        let mut mgr = make_manager();
        // Channel exists, set expected_usd = 50.
        let fake_initial = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(50.0), None,
            &fake_initial as &dyn LdkServerCalls, 100_000.0,
        ).await;

        // Price drops 20% to 80_000 (receiver USD below 50), peer connected.
        let fake_drift = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(
                &crate::config::PushConfig::default(),
                mgr.data_dir(),
            ),
        ));

        mgr.run_tick(&fake_drift as &dyn LdkServerCalls, &push, 80_000.0).await;

        let sends = fake_drift.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "expected one stability payment");
        assert_eq!(sends[0].node_id, COUNTERPARTY_HEX);
        assert!(sends[0].amount_msat > 0);
        assert!(mgr.stable_channels[0].last_stability_payment > 0,
            "cooldown timestamp should be set");
    }

    #[tokio::test]
    async fn run_tick_send_failure_keeps_cooldown_unset() {
        let mut mgr = make_manager();
        let fake_initial = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(50.0), None,
            &fake_initial as &dyn LdkServerCalls, 100_000.0,
        ).await;
        let fake_drift = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )])
        .with_send_failure();
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(
                &crate::config::PushConfig::default(),
                mgr.data_dir(),
            ),
        ));

        mgr.run_tick(&fake_drift as &dyn LdkServerCalls, &push, 80_000.0).await;

        assert_eq!(
            mgr.stable_channels[0].last_stability_payment, 0,
            "failed send must not start cooldown"
        );
    }

    #[tokio::test]
    async fn run_tick_pushes_when_offline_and_drift_exceeds_threshold() {
        let mut mgr = make_manager();
        let fake_initial = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(50.0), None,
            &fake_initial as &dyn LdkServerCalls, 100_000.0,
        ).await;

        // Peer disconnected: is_usable=false.
        let fake_offline = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, false,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(
                &crate::config::PushConfig::default(),
                mgr.data_dir(),
            ),
        ));

        mgr.run_tick(&fake_offline as &dyn LdkServerCalls, &push, 80_000.0).await;

        let sends = fake_offline.sends.lock().unwrap();
        assert!(sends.is_empty(), "must not send when peer offline");
        assert_eq!(
            mgr.stable_channels[0].last_stability_payment, 0,
            "must not bump cooldown when only pushing"
        );
    }

    #[tokio::test]
    async fn run_tick_check_only_when_connected_and_user_above_par() {
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        // expected_usd=50 at price 100k -> backing_sats = 50_000
        mgr.edit_stable_channel(CHANNEL_ID_HEX, Some(50.0), None, &fake0 as &dyn LdkServerCalls, 100_000.0).await;

        // Price RISES to 120k: stable_usd_value = 50_000/1e8*120k = $60 > $50 target -> user_to_lsp.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 120_000.0).await;
        assert!(fake.sends.lock().unwrap().is_empty(), "LSP must NOT send when user is above par (CHECK_ONLY)");
    }

    #[tokio::test]
    async fn run_tick_resets_backing_to_equilibrium_after_send() {
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        // expected_usd=50 at price 100k -> backing_sats = 50_000
        mgr.edit_stable_channel(CHANNEL_ID_HEX, Some(50.0), None, &fake0 as &dyn LdkServerCalls, 100_000.0).await;

        // Price DROPS to 80k: stable_usd_value = 50_000/1e8*80k = $40 < $50 -> lsp_to_user -> send.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 80_000.0).await;

        assert_eq!(fake.sends.lock().unwrap().len(), 1, "should send in lsp_to_user direction");
        // backing reset to target/price = 50/80000*1e8 = 62_500 (NOT left at stale 50_000).
        assert_eq!(mgr.stable_channels[0].backing_sats, 62_500, "backing must reset to equilibrium, preventing oscillation");
    }

    #[tokio::test]
    async fn run_tick_skips_high_risk_channel() {
        let mut mgr = make_manager();
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 50.0, 50_000, 0, 50_000, 100_000.0);
        mgr.stable_channels[0].risk_level = stable_channels::constants::MAX_RISK_LEVEL + 1;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        // Price drops 20% -> would normally pay lsp_to_user; high risk must skip.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 80_000.0).await;
        assert!(
            fake.sends.lock().unwrap().is_empty(),
            "a channel above MAX_RISK_LEVEL must not trigger a stability send"
        );
    }

    #[tokio::test]
    async fn backstop_deducts_and_syncs_after_two_low_ticks() {
        let mut mgr = make_manager();
        // expected $10 -> backing 10_000; receiver 50_000 (native 40_000) at $100k.
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 10.0, 10_000, 40_000, 50_000, 100_000.0);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        // Live balance dropped to 5_000 (< backing 10_000): a spend the forwarded event missed.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 95_000_000, true,
        )]);

        // Tick 1: debounce only.
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0).await;
        assert!((mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6, "tick 1 must not deduct");
        assert!(fake.sends.lock().unwrap().is_empty(), "tick 1 must not SYNC");

        // Tick 2: act.
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0).await;
        let exp = mgr.stable_channels[0].expected_usd.0;
        assert!((exp - 5.0).abs() < 0.01, "tick 2 must deduct ~$5 (10_000-5_000 sats), got {}", exp);
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "tick 2 must send exactly one SYNC");
        assert_eq!(sends[0].custom_tlvs.len(), 1, "SYNC must carry exactly one stable TLV");
        assert_eq!(
            sends[0].custom_tlvs[0].type_num,
            stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
            "SYNC TLV must be the stable-channel type",
        );
    }

    #[tokio::test]
    async fn backstop_single_tick_dip_does_not_deduct() {
        let mut mgr = make_manager();
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 10.0, 10_000, 40_000, 50_000, 100_000.0);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        // Tick 1: transient dip to 5_000 (in-flight outbound HTLC; outbound_capacity excludes it).
        let dip = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 95_000_000, true,
        )]);
        mgr.run_tick(&dip as &dyn LdkServerCalls, &push, 100_000.0).await;
        // Tick 2: balance restored to 50_000 (HTLC resolved without spending stable).
        let restored = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.run_tick(&restored as &dyn LdkServerCalls, &push, 100_000.0).await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6, "a transient dip must not deduct");
        assert!(restored.sends.lock().unwrap().is_empty(), "no SYNC for a transient dip");
    }

    #[tokio::test]
    async fn backstop_noop_when_balance_healthy() {
        let mut mgr = make_manager();
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 10.0, 10_000, 40_000, 50_000, 100_000.0);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        // Healthy: their 50_000 >= backing 10_000.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0).await;
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0).await;
        assert!((mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6);
        assert!(fake.sends.lock().unwrap().is_empty(), "no backstop action when healthy");
    }

    #[tokio::test]
    async fn reconcile_hydrates_channel_with_decimal_user_channel_id() {
        let mut mgr = make_manager();
        // Persist a row whose user_channel_id is the realistic decimal form.
        mgr.db.save_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 12.0, 30_000, 5_000, Some("dec")).unwrap();

        // The live channel reports the SAME decimal user_channel_id (as real gRPC does).
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.reconcile_from_grpc(&fake as &dyn LdkServerCalls, 100_000.0).await;

        assert_eq!(mgr.stable_channels.len(), 1, "decimal-id channel MUST hydrate (not be dropped)");
        assert_eq!(mgr.stable_channels[0].expected_usd.0, 12.0);
        // The in-memory u128 must equal the decimal parse, not a hex misparse.
        assert_eq!(mgr.stable_channels[0].user_channel_id, 189476124653200987495269098788434301048u128);
    }

    #[test]
    fn parse_user_channel_id_prefers_decimal() {
        assert_eq!(parse_user_channel_id("189476124653200987495269098788434301048"),
                   Some(189476124653200987495269098788434301048u128));
        // hex fallback still works for 0x-prefixed values
        assert_eq!(parse_user_channel_id("0x01"), Some(1));
    }

    #[tokio::test]
    async fn send_sync_message_keysends_signed_tlv() {
        let mgr = make_manager();
        let fake = FakeLdkServer::new(vec![]);
        mgr.db
            .save_channel("sync-channel", "7", 25.0, 31_250, 0, None)
            .unwrap();
        assert!(
            mgr.send_sync_message(
                &fake as &dyn LdkServerCalls,
                7u128,
                CHANNEL_ID_HEX,
                25.0,
                31_250,
                COUNTERPARTY_HEX,
            )
            .await
        );

        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].amount_msat, 1);
        assert_eq!(sends[0].node_id, COUNTERPARTY_HEX);
        assert_eq!(sends[0].custom_tlvs.len(), 1);
        assert_eq!(
            sends[0].custom_tlvs[0].type_num,
            stable_channels::constants::STABLE_CHANNEL_TLV_TYPE
        );
        assert_eq!(fake.sign_calls.lock().unwrap().len(), 1);

        let raw = std::str::from_utf8(sends[0].custom_tlvs[0].value.as_ref()).unwrap();
        let env = crate::messages::parse_envelope(raw).unwrap();
        assert_eq!(env.signature, "fake-sig");
        let v: serde_json::Value = serde_json::from_str(&env.payload).unwrap();
        assert_eq!(v["type"], "SYNC_V1");
        assert_eq!(v["channel_id"], CHANNEL_ID_HEX);
        assert_eq!(v["user_channel_id"], "7");
        assert_eq!(v["expected_usd"], 25.0);
        assert_eq!(v["backing_sats"], 31_250);
        assert_eq!(v["sync_version"], 1);
        assert_eq!(mgr.db.get_sync_version("7").unwrap(), Some(1));
    }

    #[tokio::test]
    async fn fake_sign_and_verify_behaviour() {
        let fake = FakeLdkServer::new(vec![]);
        let sig = fake
            .sign_message(SignMessageRequest { message: b"hello".to_vec().into() })
            .await
            .unwrap();
        assert_eq!(sig.signature, "fake-sig");
        assert_eq!(fake.sign_calls.lock().unwrap().len(), 1);

        let ok = fake
            .verify_signature(VerifySignatureRequest {
                message: b"hello".to_vec().into(),
                signature: "fake-sig".to_string(),
                public_key: COUNTERPARTY_HEX.to_string(),
            })
            .await
            .unwrap();
        assert!(ok.valid);

        let bad = FakeLdkServer::new(vec![]).with_verify_failure();
        let res = bad
            .verify_signature(VerifySignatureRequest {
                message: b"x".to_vec().into(),
                signature: "s".to_string(),
                public_key: COUNTERPARTY_HEX.to_string(),
            })
            .await
            .unwrap();
        assert!(!res.valid);
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_channel(
        mgr: &mut StableChannelManager,
        user_channel_id: u128,
        counterparty: &str,
        channel_id: &str,
        expected_usd: f64,
        backing_sats: u64,
        native_sats: u64,
        receiver_sats: u64,
        price: f64,
    ) {
        mgr.stable_channels.push(StableChannel {
            channel_id: ldk_node::lightning::ln::types::ChannelId::from_bytes(
                parse_channel_id_hex(channel_id),
            ),
            user_channel_id,
            counterparty: parse_pubkey_hex(counterparty),
            is_stable_receiver: false,
            expected_usd: USD::from_f64(expected_usd),
            expected_btc: Bitcoin::from_sats(0),
            stable_receiver_btc: Bitcoin::from_sats(receiver_sats),
            stable_receiver_usd: USD::from_bitcoin(Bitcoin::from_sats(receiver_sats), price),
            stable_provider_btc: Bitcoin::from_sats(0),
            stable_provider_usd: USD(0.0),
            latest_price: price,
            risk_level: 0,
            payment_made: false,
            timestamp: 0,
            formatted_datetime: String::new(),
            sc_dir: String::new(),
            prices: String::new(),
            onchain_btc: Bitcoin::from_sats(0),
            onchain_usd: USD(0.0),
            note: None,
            native_channel_btc: Bitcoin::from_sats(0),
            backing_sats,
            native_sats,
            last_stability_payment: 0,
        });
    }

    fn trade_envelope(channel_id: &str, user_channel_id: &str, expected_usd: f64) -> String {
        let payload = serde_json::json!({
            "type": "TRADE_V1",
            "channel_id": channel_id,
            "user_channel_id": user_channel_id,
            "expected_usd": expected_usd,
        })
        .to_string();
        serde_json::json!({ "payload": payload, "signature": "wallet-sig" }).to_string()
    }

    #[tokio::test]
    async fn run_tick_cooldown_emits_audit_with_uid() {
        let _g = AUDIT_TEST_GUARD.lock().unwrap();
        stable_channels::audit::enable_test_capture();
        let mut mgr = make_manager();
        // Seed channel with backing_sats=0 so stable_usd_value = stable_receiver_usd (live balance).
        // receiver_sats=50_000 at 100k = $50; expected=50. Price drops to 80k -> $40 < $50 (20% drift).
        seed_channel(&mut mgr, 1u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 50.0, 0, 0, 50_000, 100_000.0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        // Set last_stability_payment in the future so (now - future) < 0 <= cooldown, activating the gate even when cooldown_secs=0.
        mgr.stable_channels[0].last_stability_payment = now + 100;
        // Channel with 50k their side; price 80k -> drift 20% -> exceeds threshold -> hits cooldown gate.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(
                &crate::config::PushConfig::default(),
                mgr.data_dir(),
            ),
        ));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 80_000.0).await;
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let cd = events.iter().find(|(e, _)| e == "STABILITY_COOLDOWN")
            .expect("STABILITY_COOLDOWN should be emitted on a cooldown-blocked tick");
        assert!(cd.1.get("user_channel_id").is_some(), "must carry user_channel_id");
        assert!(cd.1.get("channel_id").is_some(), "must carry channel_id");
    }

    fn trade_envelope_with_ts(
        channel_id: &str,
        user_channel_id: &str,
        expected_usd: f64,
        ts: u64,
    ) -> String {
        let payload = serde_json::json!({
            "type": "TRADE_V1",
            "channel_id": channel_id,
            "user_channel_id": user_channel_id,
            "expected_usd": expected_usd,
            "ts": ts,
        })
        .to_string();
        serde_json::json!({ "payload": payload, "signature": "wallet-sig" }).to_string()
    }

    fn trade_envelope_with_allocation(
        channel_id: &str,
        user_channel_id: &str,
        expected_usd: f64,
        quote_price: f64,
        backing_sats: u64,
    ) -> String {
        let payload = serde_json::json!({
            "type": "TRADE_V1",
            "channel_id": channel_id,
            "user_channel_id": user_channel_id,
            "expected_usd": expected_usd,
            "quote_price": quote_price,
            "backing_sats": backing_sats,
            "ts": test_unix_now(),
        })
        .to_string();
        serde_json::json!({ "payload": payload, "signature": "wallet-sig" }).to_string()
    }

    fn test_unix_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    async fn handle_trade_with_valid_fee(
        mgr: &mut StableChannelManager,
        envelope: &str,
        ldk: &dyn LdkServerCalls,
        lsp_price: f64,
    ) {
        let signed = crate::messages::parse_envelope(envelope).unwrap();
        let payload = crate::messages::parse_trade_payload(&signed.payload).unwrap();
        let current_expected = mgr
            .stable_channels
            .iter()
            .find(|sc| {
                payload
                    .user_channel_id
                    .as_deref()
                    .and_then(parse_user_channel_id)
                    == Some(sc.user_channel_id)
            })
            .map(|sc| sc.expected_usd.0)
            .unwrap_or(0.0);
        let fee_msat = expected_trade_fee_msat(
            current_expected,
            payload.expected_usd,
            payload.quote_price.unwrap_or(lsp_price),
        )
        .unwrap();
        mgr.handle_trade_message(envelope, Some(fee_msat), ldk, lsp_price)
            .await;
    }

    #[test]
    fn trade_fee_matches_wallet_buy_and_sell_rounding() {
        assert_eq!(
            expected_trade_fee_msat(100.0, 50.0, 100_000.0),
            Some(500_000)
        );
        assert_eq!(
            expected_trade_fee_msat(50.0, 99.5, 100_000.0),
            Some(500_000)
        );
        assert_eq!(
            expected_trade_fee_msat(50.0, 50.0, 100_000.0),
            Some(1)
        );
    }

    #[tokio::test]
    async fn trade_applies_valid_target() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn trade_rejects_underpaid_signed_fee() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            0.0,
            0,
            50_000,
            50_000,
            100_000.0,
        );
        let env = trade_envelope_with_allocation(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            49.95,
            100_000.0,
            49_950,
        );

        mgr.handle_trade_message(
            &env,
            Some(1),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 0);
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn trade_applies_signed_allocation_without_lsp_repricing() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            0.0,
            0,
            50_000,
            50_000,
            100_500.0,
        );

        // At the wallet quote this is exactly 49,950 backing sats. Re-deriving at the LSP's
        // slightly newer price would produce a different allocation.
        let env = trade_envelope_with_allocation(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            49.95,
            100_000.0,
            49_950,
        );
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_500.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].backing_sats, 49_950);
        assert_eq!(mgr.stable_channels[0].native_sats, 50);
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "accepted allocation must be synced back");
        let raw = std::str::from_utf8(sends[0].custom_tlvs[0].value.as_ref()).unwrap();
        let sync = crate::messages::parse_envelope(raw).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&sync.payload).unwrap();
        assert_eq!(payload["backing_sats"], 49_950);
    }

    #[tokio::test]
    async fn trade_accepts_economically_consistent_full_allocation_with_balance_skew() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            0.0,
            0,
            50_000,
            50_000,
            100_000.0,
        );

        // The wallet signed against a receiver balance five sats below the LSP's post-settlement
        // observation. The pair is still fully collateralized and differs by only half a cent.
        let env = trade_envelope_with_allocation(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            50.0,
            100_000.0,
            49_995,
        );
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 50.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 49_995);
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn trade_rejects_allocation_not_derived_from_signed_quote() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            5.0,
            5_000,
            45_000,
            50_000,
            100_000.0,
        );

        let env = trade_envelope_with_allocation(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            49.95,
            100_000.0,
            49_000,
        );
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 5.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 5_000);
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn trade_rejects_invalid_signature() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]).with_verify_failure();
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 3.0, 3_000, 47_000, 50_000, 100_000.0);

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 3.0).abs() < 1e-6); // unchanged
    }

    #[tokio::test]
    async fn trade_rejects_over_balance() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 999.0);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 0.0).abs() < 1e-6); // unchanged
    }

    #[tokio::test]
    async fn trade_admits_at_balance_boundary_within_epsilon() {
        let mut mgr = make_manager();
        // Live receiver side = 50_000 sats at $100k -> receiver_usd = $50.00 exactly.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        // A wallet-push lands at receiver_usd plus a sub-epsilon overshoot (independent f64 paths).
        let target = 50.0 + stable_channels::constants::STABILITY_THRESHOLD_USD / 2.0;
        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, target);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!(
            (mgr.stable_channels[0].expected_usd.0 - target).abs() < 1e-6,
            "a target within epsilon of the balance must be admitted, got {}",
            mgr.stable_channels[0].expected_usd.0
        );
    }

    #[tokio::test]
    async fn trade_channel_not_found_is_noop() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 5.0, 5_000, 45_000, 50_000, 100_000.0);

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 5.0).abs() < 1e-6); // unchanged
    }

    #[tokio::test]
    async fn trade_rejects_stale_ts() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        // A captured signed trade replayed a day later must be rejected (replay protection).
        let stale = test_unix_now() - 86_400;
        let env = trade_envelope_with_ts(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0, stale);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!(
            (mgr.stable_channels[0].expected_usd.0 - 0.0).abs() < 1e-6,
            "a stale signed trade must be rejected, got {}",
            mgr.stable_channels[0].expected_usd.0
        );
    }

    #[tokio::test]
    async fn trade_stale_audit_carries_user_channel_id() {
        let _g = AUDIT_TEST_GUARD.lock().unwrap();
        stable_channels::audit::enable_test_capture();
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);
        let stale = test_unix_now() - 86_400;
        let env = trade_envelope_with_ts(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0, stale);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let stale_ev = events.iter().find(|(e, _)| e == "TRADE_STALE")
            .expect("TRADE_STALE must be emitted for a stale signed trade");
        assert!(stale_ev.1.get("user_channel_id").is_some(), "TRADE_STALE must carry user_channel_id");
    }

    #[tokio::test]
    async fn trade_accepts_fresh_ts() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        // A trade signed just now is within the window and applies normally.
        let env = trade_envelope_with_ts(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0, test_unix_now());
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6, "a fresh signed trade must apply");
    }

    #[tokio::test]
    async fn splice_out_deducts_and_syncs() {
        let mut mgr = make_manager();
        // Post-splice snapshot: their = 5,000 (our 95k via outbound 95M msat).
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 95_000_000, true,
        )]);
        // expected $10 -> backing 10,000; receiver was 50,000.
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 10.0, 10_000, 40_000, 50_000, 100_000.0);

        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_DECIMAL.to_string(),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        // backing 10,000 vs new receiver 5,000 -> overflow 5,000 = $5 -> expected $5.
        assert!((mgr.stable_channels[0].expected_usd.0 - 5.0).abs() < 1e-6);
        assert_eq!(fake.sends.lock().unwrap().len(), 1, "splice-out should SYNC");
    }

    #[tokio::test]
    async fn splice_in_does_not_sync() {
        let mut mgr = make_manager();
        // Post-splice snapshot: their grew to 80,000 (our 20k via outbound 20M msat).
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 20_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 10.0, 10_000, 40_000, 50_000, 100_000.0);

        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_DECIMAL.to_string(),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6); // unchanged
        assert_eq!(fake.sends.lock().unwrap().len(), 0, "splice-in must not SYNC");
    }

    #[tokio::test]
    async fn splice_replay_does_not_double_deduct() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 95_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 10.0, 10_000, 40_000, 50_000, 100_000.0);

        for _ in 0..2 {
            mgr.handle_channel_ready(
                CHANNEL_ID_HEX.to_string(),
                USER_CHANNEL_ID_DECIMAL.to_string(),
                &fake as &dyn LdkServerCalls,
                100_000.0,
            )
            .await;
        }

        assert!((mgr.stable_channels[0].expected_usd.0 - 5.0).abs() < 1e-6); // deducted once, not twice
        assert_eq!(fake.sends.lock().unwrap().len(), 1, "second pass deducts nothing, no second SYNC");
    }

    #[tokio::test]
    async fn edit_stable_channel_emits_audit_event() {
        // Editing a USD target must leave a STABLE_EDITED entry in the audit log.
        use stable_channels::audit::{get_audit_log_path, set_audit_log_path};
        let dir = tempdir().unwrap();
        let audit_path = dir.path().join("audit_log.txt");
        // OnceLock: this wins if unset, otherwise we read whichever path is live.
        set_audit_log_path(audit_path.to_str().unwrap());
        let path = get_audit_log_path()
            .expect("an audit log path is set")
            .to_string();

        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX,
            100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX, Some(7.5), None,
            &fake as &dyn LdkServerCalls, 100_000.0,
        ).await;

        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            contents.contains("STABLE_EDITED"),
            "audit log should record STABLE_EDITED, got: {}",
            contents
        );
        assert!(
            contents.contains(USER_CHANNEL_ID_DECIMAL),
            "STABLE_EDITED audit entry should include the user_channel_id"
        );
    }

    fn fwd(prev: &str, next: &str, amt: u64) -> GrpcForwardedPayment {
        GrpcForwardedPayment {
            prev_htlcs: vec![HtlcLocator {
                channel_id: prev.into(),
                user_channel_id: Some("10".into()),
                node_id: Some("02aa".into()),
            }],
            next_htlcs: vec![HtlcLocator {
                channel_id: next.into(),
                user_channel_id: Some("20".into()),
                node_id: Some("02bb".into()),
            }],
            total_fee_earned_msat: Some(7),
            skimmed_fee_msat: None,
            claim_from_onchain_tx: false,
            outbound_amount_forwarded_msat: Some(amt),
        }
    }

    #[tokio::test]
    async fn backfill_emits_unseen_then_dedups() {
        // open_in_memory() is #[cfg(test)]-gated in the shared crate, unreachable across this crate boundary; use the tempdir pattern from make_manager() instead.
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let fake = FakeLdkServer::new(vec![]).with_forwarded(vec![fwd("aa", "bb", 1000), fwd("cc", "dd", 2000)]);
        assert_eq!(crate::backfill::backfill_forwards(&fake, &db).await, 2); // both unseen
        assert_eq!(crate::backfill::backfill_forwards(&fake, &db).await, 0); // both now seen
    }

    #[test]
    fn should_log_on_outcome_change() {
        assert!(stability_should_log("", "check_only", 0.0, 90.0, 90.0, 0.25, 1.0, true));
        assert!(stability_should_log("cooldown", "check_only", 90.0, 90.0, 90.0, 0.25, 1.0, true));
    }

    #[test]
    fn should_log_on_significant_value_move_when_tracking() {
        // same outcome, move > $0.25 and > 1% -> true
        assert!(stability_should_log("check_only", "check_only", 90.0, 92.0, 90.0, 0.25, 1.0, true));
        // same outcome, sub-threshold move -> false
        assert!(!stability_should_log("check_only", "check_only", 90.0, 90.10, 90.0, 0.25, 1.0, true));
    }

    #[test]
    fn no_value_trigger_when_not_tracking() {
        // same outcome, huge move, but track_value=false -> false
        assert!(!stability_should_log("high_risk", "high_risk", 90.0, 200.0, 90.0, 0.25, 1.0, false));
    }

    #[tokio::test]
    async fn run_tick_throttles_repeated_check_only() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(CHANNEL_ID_HEX, Some(50.0), None, &fake0 as &dyn LdkServerCalls, 100_000.0).await;
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        stable_channels::audit::enable_test_capture();
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 120_000.0).await; // above par -> CHECK_ONLY (emit)
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 120_000.0).await; // identical -> throttled
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let n = events.iter().filter(|(e, _)| e == "STABILITY_CHECK_ONLY").count();
        assert_eq!(n, 1, "identical repeated ticks must emit CHECK_ONLY once");
    }

    /// TLV marker record that is NOT a signed envelope: the stability-payment carrier.
    fn stability_marker() -> CustomTlvRecord {
        CustomTlvRecord {
            type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
            value: vec![1u8].into(),
        }
    }

    #[tokio::test]
    async fn incoming_stability_payment_resets_backing_and_preserves_native() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        // $10 target at $100k: equilibrium backing = 10_000 sats; user side 50_000 -> native 40_000.
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(CHANNEL_ID_HEX, Some(10.0), None, &fake0 as &dyn LdkServerCalls, 100_000.0).await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        // Snapshot balances at par (no drift at $100k, so nothing fires).
        mgr.run_tick(&fake0 as &dyn LdkServerCalls, &push, 100_000.0).await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);
        assert_eq!(mgr.stable_channels[0].stable_receiver_btc.sats, 50_000);

        // Price rises to $110k: user is $1 above par and settles 909 sats to the LSP.
        // Live user side drops 50_000 -> 49_091 (our side gains).
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_909_000, true,
        )]);
        mgr.handle_payment_received(
            vec![stability_marker()],
            Some("pay-1".to_string()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;

        let sc = &mgr.stable_channels[0];
        // Equilibrium at $110k: 10/110_000 BTC = 9_090 sats.
        assert_eq!(sc.backing_sats, 9_090, "backing must reset to equilibrium at the new price");
        // Native must absorb only rounding, never the settlement: 49_091 - 9_090 = 40_001.
        assert_eq!(sc.native_sats, 40_001, "native sats must be preserved across the settlement");
        assert!(sc.backing_sats <= sc.stable_receiver_btc.sats, "backing may never exceed live balance");
    }

    #[tokio::test]
    async fn incoming_stability_payment_prevents_backstop_misfire() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(CHANNEL_ID_HEX, Some(10.0), None, &fake0 as &dyn LdkServerCalls, 100_000.0).await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        mgr.run_tick(&fake0 as &dyn LdkServerCalls, &push, 100_000.0).await;

        // User settles 909 sats; books reconciled at receive.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_909_000, true,
        )]);
        mgr.handle_payment_received(
            vec![stability_marker()], Some("pay-2".to_string()), Some(909_000),
            &fake as &dyn LdkServerCalls, 110_000.0,
        ).await;
        let expected_before = mgr.stable_channels[0].expected_usd.0;

        // Two ticks at the new price: without receive-time reconcile the backstop
        // would read the settled sats as an unreconciled spend and deduct expected_usd.
        stable_channels::audit::enable_test_capture();
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 110_000.0).await;
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 110_000.0).await;
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();

        assert!(
            !events.iter().any(|(e, _)| e == "BACKSTOP_STABLE_DEDUCTED"),
            "settled stability payment must not trigger the backstop"
        );
        assert_eq!(
            mgr.stable_channels[0].expected_usd.0, expected_before,
            "expected_usd must survive a settled stability payment"
        );
    }

    #[tokio::test]
    async fn ambiguous_incoming_stability_payment_mutates_nothing() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        const CHAN2_ID: &str = "aa634c603646c60b0df9f07c3011708652125915c80300a9bb8fb37c9c0de05b";
        const UID2_HEX: &str = "00000000000000000000000000000002";
        // Two identical channels: an identical balance drop on both is unattributable.
        let mk = |outbound_msat: u64| {
            vec![
                make_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, outbound_msat, true),
                make_channel(CHAN2_ID, UID2_HEX, COUNTERPARTY_HEX, 100_000, outbound_msat, true),
            ]
        };
        let fake0 = FakeLdkServer::new(mk(50_000_000));
        mgr.edit_stable_channel(CHANNEL_ID_HEX, Some(10.0), None, &fake0 as &dyn LdkServerCalls, 100_000.0).await;
        mgr.edit_stable_channel(CHAN2_ID, Some(10.0), None, &fake0 as &dyn LdkServerCalls, 100_000.0).await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        mgr.run_tick(&fake0 as &dyn LdkServerCalls, &push, 100_000.0).await;
        let backing_before: Vec<u64> = mgr.stable_channels.iter().map(|s| s.backing_sats).collect();

        let fake = FakeLdkServer::new(mk(50_909_000));
        stable_channels::audit::enable_test_capture();
        mgr.handle_payment_received(
            vec![stability_marker()], Some("pay-3".to_string()), Some(909_000),
            &fake as &dyn LdkServerCalls, 110_000.0,
        ).await;
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();

        let backing_after: Vec<u64> = mgr.stable_channels.iter().map(|s| s.backing_sats).collect();
        assert_eq!(backing_before, backing_after, "ambiguous attribution must not touch the books");
        assert!(
            events.iter().any(|(e, d)| e == "STABILITY_RECEIVE_UNATTRIBUTED"
                && d.get("candidates").and_then(|v| v.as_u64()) == Some(2)),
            "the miss must be audited with the candidate count"
        );
    }
}
