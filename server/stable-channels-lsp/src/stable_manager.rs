//! In-memory stable-channel manager, backed by the shared sqlite channels table.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use ldk_server_client::client::LdkServerClient;
use ldk_server_client::error::LdkServerError;
use ldk_server_client::ldk_server_grpc::api::{
    GetBalancesRequest, GetBalancesResponse, GetForwardedPaymentTrackingModeRequest,
    GetForwardedPaymentTrackingModeResponse, GetPaymentDetailsRequest, GetPaymentDetailsResponse,
    ListChannelsRequest, ListChannelsResponse, ListForwardedPaymentsRequest,
    ListForwardedPaymentsResponse, ListPeersRequest, ListPeersResponse, ListPaymentsRequest,
    ListPaymentsResponse, SignMessageRequest, SignMessageResponse, SpontaneousSendRequest,
    SpontaneousSendResponse, VerifySignatureRequest, VerifySignatureResponse,
};
use ldk_server_client::ldk_server_grpc::events::ChannelStateChangeReason;
use ldk_server_client::ldk_server_grpc::types::{Channel, CustomTlvRecord, PaymentStatus};
use stable_channels::constants::{
    MAX_TRADE_QUOTE_DEVIATION_PERCENT, SIGNED_STABILITY_TLV_TYPE,
    STABILITY_PAYMENT_AUTH_TTL_SECS,
};
use stable_channels::db::{Database, InboundStabilityRegistration, PendingTradeResponse};
use stable_channels::stable::StabilityPaymentDirection;
use stable_channels::trade::TradeRejectionReason;
use stable_channels::types::{Bitcoin, StableChannel, USD};
use tracing::{error, info};

/// Ordinary SYNC retries back off per accepted-then-failed attempt since the last delivery.
pub(crate) const SYNC_RETRY_BACKOFF_BASE_SECS: u64 = 60;
pub(crate) const SYNC_RETRY_BACKOFF_MAX_SECS: u64 = 3600;
/// Consecutive undelivered attempts after which retries stop until a SYNC is delivered.
pub(crate) const SYNC_RETRY_MAX_ATTEMPTS: u64 = 10;
/// An accepted SYNC with no terminal outcome after this long is treated as failed.
pub(crate) const SYNC_PENDING_TIMEOUT_SECS: u64 = 3600;

/// The first retry is immediate; later ones double from the base until the cap.
fn sync_retry_delay_secs(attempts: u64) -> u64 {
    if attempts < 2 {
        return 0;
    }
    let doublings = attempts - 2;
    if doublings >= 32 {
        return SYNC_RETRY_BACKOFF_MAX_SECS;
    }
    (SYNC_RETRY_BACKOFF_BASE_SECS << doublings).min(SYNC_RETRY_BACKOFF_MAX_SECS)
}

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

/// Trade-entry backstop only. Inbound capacity is the user's post-payment spendable balance;
/// neither the LSP's reserve nor the fee may be subtracted from it again.
fn max_stabilization_rejected(
    enforced: bool,
    channel: &Channel,
    old_expected: f64,
    new_expected: f64,
    new_backing_sats: u64,
    source: &str,
) -> bool {
    if new_expected <= old_expected {
        return false;
    }
    let spendable = channel.inbound_capacity_msat / 1000;
    let cap = stable_channels::stabilization::backing_cap(spendable);
    let exceeds = cap.is_none_or(|cap| new_backing_sats > cap);
    if exceeds {
        stable_channels::audit::audit_event(
            "MAX_STABILIZATION_REJECTED",
            serde_json::json!({
                "enforced": enforced, "source": source, "channel_id": channel.channel_id,
                "user_channel_id": channel.user_channel_id, "post_fee_spendable_sats": spendable,
                "cap_sats": cap, "new_backing_sats": new_backing_sats,
                "current_expected_usd": old_expected, "new_expected_usd": new_expected,
            }),
        );
    }
    enforced && exceeds
}

fn splice_balance_change(before_sats: u64, after_sats: u64) -> (&'static str, u64) {
    if after_sats > before_sats {
        ("in", after_sats - before_sats)
    } else if after_sats < before_sats {
        ("out", before_sats - after_sats)
    } else {
        ("unchanged", 0)
    }
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
        // The wallet floors its USD fee to whole sats before sending. Reconstructing a sell's
        // gross amount from the signed net target can land on the adjacent sat due to that lost
        // fraction, so admit exactly one sat while still rejecting material underpayment.
        return 1000;
    }

    // Transitional legacy wallets did not sign their quote. Admit the same maximum price skew as
    // signed trades, plus one sat for whole-sat rounding, while still rejecting material underpay.
    ((expected_msat as f64 * MAX_TRADE_QUOTE_DEVIATION_PERCENT / 100.0).ceil() as u64)
        .max(1000)
}

fn trade_reduction_exhausts_backing(
    current_backing_sats: u64,
    current_expected_usd: f64,
    new_expected_usd: f64,
    price: f64,
) -> bool {
    if new_expected_usd >= current_expected_usd || new_expected_usd == 0.0 || price <= 0.0 {
        return false;
    }
    let old_target = current_expected_usd / price * 100_000_000.0;
    let new_target = new_expected_usd / price * 100_000_000.0;
    if !old_target.is_finite()
        || !new_target.is_finite()
        || old_target < 0.0
        || new_target < 0.0
        || old_target >= u64::MAX as f64
        || new_target >= u64::MAX as f64
    {
        return false;
    }
    (old_target.floor() as u64).saturating_sub(new_target.floor() as u64)
        >= current_backing_sats
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
    async fn list_payments(
        &self,
        _req: ListPaymentsRequest,
    ) -> Result<ListPaymentsResponse, LdkServerError> {
        Ok(ListPaymentsResponse::default())
    }
    async fn get_payment_details(
        &self,
        _req: GetPaymentDetailsRequest,
    ) -> Result<GetPaymentDetailsResponse, LdkServerError> {
        Ok(GetPaymentDetailsResponse::default())
    }
    async fn get_forwarded_payment_tracking_mode(
        &self,
        _req: GetForwardedPaymentTrackingModeRequest,
    ) -> Result<GetForwardedPaymentTrackingModeResponse, LdkServerError> {
        Ok(GetForwardedPaymentTrackingModeResponse::default())
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
    async fn list_payments(
        &self,
        req: ListPaymentsRequest,
    ) -> Result<ListPaymentsResponse, LdkServerError> {
        LdkServerClient::list_payments(self, req).await
    }
    async fn get_payment_details(
        &self,
        req: GetPaymentDetailsRequest,
    ) -> Result<GetPaymentDetailsResponse, LdkServerError> {
        LdkServerClient::get_payment_details(self, req).await
    }
    async fn get_forwarded_payment_tracking_mode(
        &self,
        req: GetForwardedPaymentTrackingModeRequest,
    ) -> Result<GetForwardedPaymentTrackingModeResponse, LdkServerError> {
        LdkServerClient::get_forwarded_payment_tracking_mode(self, req).await
    }
}

/// A correction already calculated from an observed balance. Preserve it across save failures:
/// recalculating after another payment can erase the original deduction.
#[derive(Clone)]
struct PendingBookUpdate {
    channel_id: String,
    proposed: StableChannel,
    context: &'static str,
    audits: Vec<(&'static str, serde_json::Value)>,
    needs_sync: bool,
}

/// In-memory list of stable channels plus a handle to the shared sqlite channels table.
pub struct StableChannelManager {
    pub stable_channels: Vec<StableChannel>,
    /// Shadow mode by default; this flag never changes settlement/reconciliation behavior.
    pub enforce_max_stabilization: bool,
    db: Arc<Database>,
    data_dir: PathBuf,
    /// Per-channel consecutive low-balance tick count for the balance-truth backstop debounce (ignores transient in-flight HTLCs).
    spend_debounce: std::collections::HashMap<u128, u8>,
    /// Splice events still awaiting a usable snapshot or a committed correction.
    pending_splices: std::collections::HashMap<u128, Option<String>>,
    pending_book_updates: std::collections::HashMap<u128, PendingBookUpdate>,
    /// Per-channel last logged stability outcome + value, so run_tick only audits on state-change.
    stability_throttle: std::collections::HashMap<u128, (String, f64)>,
    /// Channels awaiting a startup SYNC or retry. Accepted attempts and terminal outcomes are
    /// persisted in settlement_payments; this set also covers failures before an ID is recorded.
    startup_sync_pending: std::collections::HashSet<u128>,
    startup_sync_initialized: bool,
    /// Channels whose failure-driven SYNC retries stopped at the cap, audited once each.
    sync_retry_exhausted: std::collections::HashSet<u128>,
}

/// Outcome of an `edit_stable_channel` call.
#[derive(Debug, PartialEq)]
pub struct EditOutcome {
    pub ok: bool,
    pub status: String,
}

impl StableChannelManager {
    /// Keep the next event with its caller until earlier corrections commit. Return the lock
    /// itself so a tick or settings edit cannot overtake the event after the check.
    pub(crate) async fn lock_for_event<'a>(
        manager: &'a tokio::sync::Mutex<Self>,
        ldk: &dyn LdkServerCalls,
    ) -> tokio::sync::MutexGuard<'a, Self> {
        loop {
            let mut mgr = manager.lock().await;
            let price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
            if mgr.retry_pending_reconciliations(ldk, price).await {
                return mgr;
            }
            drop(mgr);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    fn persist_pending_book_update(&mut self, uid: u128) -> bool {
        let Some(pending) = self.pending_book_updates.get(&uid).cloned() else {
            return true;
        };
        let Some(idx) = self
            .stable_channels
            .iter()
            .position(|sc| sc.user_channel_id == uid)
        else {
            self.pending_book_updates.remove(&uid);
            self.pending_splices.remove(&uid);
            return true;
        };
        let sc = &pending.proposed;
        if let Err(error) = self.db.save_channel(
            &pending.channel_id,
            &uid.to_string(),
            sc.expected_usd.0,
            sc.backing_sats,
            sc.native_sats,
            sc.note.as_deref(),
        ) {
            tracing::error!(
                "[stable] pending {} save failed: {}",
                pending.context,
                error
            );
            stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({
                    "op": "save_channel", "context": pending.context,
                    "channel_id": pending.channel_id, "user_channel_id": uid.to_string(),
                    "error": error.to_string(),
                }),
            );
            return false;
        }
        self.stable_channels[idx] = pending.proposed;
        self.pending_book_updates.remove(&uid);
        self.pending_splices.remove(&uid);
        self.spend_debounce.remove(&uid);
        for (event, data) in pending.audits {
            stable_channels::audit::audit_event(event, data);
        }
        if pending.needs_sync {
            self.startup_sync_pending.insert(uid);
        }
        true
    }

    async fn retry_pending_reconciliations(
        &mut self,
        ldk: &dyn LdkServerCalls,
        price: f64,
    ) -> bool {
        if self.pending_book_updates.is_empty() && self.pending_splices.is_empty() {
            return true;
        }
        let updates: Vec<_> = self.pending_book_updates.keys().copied().collect();
        for uid in updates {
            self.persist_pending_book_update(uid);
        }
        let splices = self.pending_splices.clone();
        for (uid, funding_txo) in splices {
            if !self.pending_book_updates.contains_key(&uid) {
                self.handle_channel_ready_splice(uid, funding_txo.as_deref(), ldk, price)
                    .await;
            }
        }
        self.retry_startup_sync(ldk).await;
        // Only a calculated correction is an ordering barrier. A splice with no snapshot
        // must not hold the event stream: a later ChannelClosed may be what removes it.
        self.pending_book_updates.is_empty()
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub(crate) fn unix_time_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64) as i64
    }

    async fn send_pending_trade_response(
        db: &Database,
        ldk: &dyn LdkServerCalls,
        response: &PendingTradeResponse,
    ) {
        let now = Self::unix_time_secs();
        match db.reserve_trade_response_attempt(
            &response.inbound_payment_id,
            response.attempts,
            now,
        ) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "DB_WRITE_FAILED",
                    serde_json::json!({
                        "op": "reserve_trade_response_attempt",
                        "trade_payment_id": response.inbound_payment_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        }
        let send = ldk
            .spontaneous_send(SpontaneousSendRequest {
                amount_msat: 1,
                node_id: response.counterparty.clone(),
                route_parameters: None,
                preimage: None,
                custom_tlvs: vec![CustomTlvRecord {
                    type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
                    value: response.response_envelope.clone().into_bytes().into(),
                }],
            })
            .await;
        match send {
            Ok(sent) if !sent.payment_id.is_empty() => {
                match db.mark_trade_response_in_flight(
                    &response.inbound_payment_id,
                    &sent.payment_id,
                ) {
                    Ok(true) => stable_channels::audit::audit_event(
                        "TRADE_RESPONSE_SENT",
                        serde_json::json!({
                            "trade_payment_id": response.inbound_payment_id,
                            "response_payment_id": sent.payment_id,
                            "attempt": response.attempts.saturating_add(1),
                        }),
                    ),
                    Ok(false) | Err(_) => stable_channels::audit::audit_event(
                        "TRADE_RESPONSE_PAYMENT_ID_PERSIST_FAILED",
                        serde_json::json!({
                            "trade_payment_id": response.inbound_payment_id,
                            "response_payment_id": sent.payment_id,
                        }),
                    ),
                }
            }
            result => stable_channels::audit::audit_event(
                "TRADE_RESPONSE_SEND_FAILED",
                serde_json::json!({
                    "trade_payment_id": response.inbound_payment_id,
                    "attempt": response.attempts.saturating_add(1),
                    "error": result.err().map(|error| error.to_string()),
                }),
            ),
        }
    }

    /// Reconcile uncertain sends, expire 14-day obligations, prune 30-day response bytes, and
    /// deliver all currently due decisions. Every send is reserved durably first.
    pub async fn retry_pending_trade_responses(db: &Database, ldk: &dyn LdkServerCalls) {
        let in_flight = db
            .in_flight_trade_response_payment_ids()
            .unwrap_or_default();
        for payment_id in in_flight {
            match ldk
                .get_payment_details(GetPaymentDetailsRequest {
                    payment_id: payment_id.clone(),
                })
                .await
                .ok()
                .and_then(|response| response.payment)
                .map(|payment| payment.status)
            {
                Some(status) if status == PaymentStatus::Succeeded as i32 => {
                    let _ = db.mark_trade_response_delivered(
                        &payment_id,
                        Self::unix_time_secs(),
                    );
                }
                Some(status) if status == PaymentStatus::Failed as i32 => {
                    let _ = db.mark_trade_response_failed(&payment_id, Self::unix_time_secs());
                }
                _ => {}
            }
        }
        let now = Self::unix_time_secs();
        let _ = db.abandon_expired_trade_responses(now);
        let _ = db.prune_trade_response_details(now);
        let responses = match db.due_trade_responses(now, 32) {
            Ok(responses) => responses,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "DB_READ_FAILED",
                    serde_json::json!({
                        "op": "due_trade_responses",
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        for response in responses {
            Self::send_pending_trade_response(db, ldk, &response).await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn reject_correlated_trade(
        &self,
        ldk: &dyn LdkServerCalls,
        inbound_payment_id: &str,
        trade_id: &str,
        request_hash: &str,
        channel_id: &str,
        user_channel_id: &str,
        counterparty: &str,
        reason: TradeRejectionReason,
    ) {
        let decided_at = Self::unix_time_secs();
        let payload = crate::messages::build_trade_rejected_payload(
            channel_id,
            trade_id,
            inbound_payment_id,
            request_hash,
            reason,
            decided_at as u64,
        );
        let signature = match ldk
            .sign_message(SignMessageRequest {
                message: payload.as_bytes().to_vec().into(),
            })
            .await
        {
            Ok(response) => response.signature,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "TRADE_REJECTION_SIGN_FAILED",
                    serde_json::json!({
                        "trade_id": trade_id,
                        "reason_code": reason.as_str(),
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        let envelope = crate::messages::build_envelope(payload, signature);
        match self.db.persist_trade_rejection(
            inbound_payment_id,
            trade_id,
            request_hash,
            channel_id,
            user_channel_id,
            counterparty,
            reason.as_str(),
            decided_at,
            &envelope,
        ) {
            Ok(true) => stable_channels::audit::audit_event(
                "TRADE_REJECTION_QUEUED",
                serde_json::json!({
                    "protocol_path": "hardened",
                    "trade_id": trade_id,
                    "trade_payment_id": inbound_payment_id,
                    "request_hash": request_hash,
                    "reason_code": reason.as_str(),
                }),
            ),
            Ok(false) => {}
            Err(error) => stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({
                    "op": "persist_trade_rejection",
                    "trade_id": trade_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }

    /// Consume an asynchronous failure for an outbound stability payment. The database performs
    /// the authoritative compare-and-swap; the in-memory allocation is restored only when it is
    /// still the exact optimistic state written by that payment.
    pub fn handle_failed_stability_payment(
        &mut self,
        payment_id: &str,
    ) -> Option<stable_channels::db::StabilityRollback> {
        let rollback = match self.db.rollback_failed_stability_settlement(payment_id) {
            Ok(value) => value?,
            Err(error) => {
                tracing::error!(
                    "[stable] failed-payment rollback lookup failed for {}: {}",
                    payment_id,
                    error
                );
                stable_channels::audit::audit_event(
                    "DB_WRITE_FAILED",
                    serde_json::json!({
                        "op": "rollback_failed_stability_settlement",
                        "payment_id": payment_id,
                        "error": error.to_string(),
                    }),
                );
                return None;
            }
        };

        if rollback.applied {
            let rollback_user_channel_id = parse_user_channel_id(&rollback.user_channel_id);
            if let Some(sc) = self
                .stable_channels
                .iter_mut()
                .find(|sc| rollback_user_channel_id == Some(sc.user_channel_id))
            {
                if sc.backing_sats == rollback.backing_sats_after
                    && sc.native_sats == rollback.native_sats_before
                    && sc.expected_usd.0 == rollback.expected_usd
                {
                    sc.backing_sats = rollback.backing_sats_before;
                    sc.native_sats = rollback.native_sats_before;
                    sc.native_channel_btc = Bitcoin::from_sats(sc.native_sats);
                    sc.last_stability_payment = rollback.last_stability_payment_before;
                }
            }
            self.stability_throttle
                .remove(&rollback_user_channel_id.unwrap_or_default());
        }

        Some(rollback)
    }

    pub fn new(db: Arc<Database>, data_dir: PathBuf) -> Self {
        Self {
            stable_channels: Vec::new(),
            enforce_max_stabilization: false,
            db,
            data_dir,
            spend_debounce: std::collections::HashMap::new(),
            pending_splices: std::collections::HashMap::new(),
            pending_book_updates: std::collections::HashMap::new(),
            stability_throttle: std::collections::HashMap::new(),
            startup_sync_pending: std::collections::HashSet::new(),
            startup_sync_initialized: false,
            sync_retry_exhausted: std::collections::HashSet::new(),
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
        if !self.retry_pending_reconciliations(ldk_server, btc_price).await {
            return EditOutcome {
                ok: false,
                status: "Balance correction is pending; retry the edit after it is saved".to_owned(),
            };
        }
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

        // Manual target increases are trade-entry too. Notes/reductions and settlement
        // reconciliation must remain possible for a drift-inflated position.
        if expected_usd_in.is_some()
            && max_stabilization_rejected(
                self.enforce_max_stabilization,
                &channel,
                prior_target.unwrap_or(0.0),
                expected_usd_f,
                backing_sats,
                "edit",
            )
        {
            return EditOutcome {
                ok: false,
                status: TradeRejectionReason::InsufficientCapacity
                    .user_message()
                    .to_string(),
            };
        }

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
            self.pending_splices.remove(&t);
            self.pending_book_updates.remove(&t);
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
        if !self.retry_pending_reconciliations(ldk, btc_price).await {
            return;
        }
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
        // Poll accepted sends as well as handling live events: a terminal event can be missed
        // during an event-stream gap. Only a known failure permits a replacement attempt.
        if let Err(error) = self.reconcile_sync_outcomes(ldk).await {
            tracing::warn!("[stable] SYNC outcome reconciliation failed: {}", error);
        }
        match self.db.list_failed_sync_channels() {
            Ok(channels) => self.queue_due_sync_retries(&channels),
            Err(error) => {
                tracing::error!("[stable] failed to load SYNC retries: {}", error);
                return;
            }
        }
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
                    && !self.pending_splices.contains_key(&sc.user_channel_id)
                    && !self.pending_book_updates.contains_key(&sc.user_channel_id)
            })
            .map(|sc| {
                (
                    sc.user_channel_id,
                    sc.channel_id.to_string(),
                    sc.stable_receiver_btc.sats,
                    sc.counterparty.to_string(),
                )
            })
            .collect();
        if syncs.is_empty() {
            return;
        }
        // An offline peer cannot take a keysend: keep the obligation queued and consume no version.
        let usable: std::collections::HashSet<u128> =
            match ldk.list_channels(ListChannelsRequest {}).await {
                Ok(response) => response
                    .channels
                    .iter()
                    .filter(|c| c.is_usable)
                    .filter_map(|c| parse_user_channel_id(&c.user_channel_id))
                    .collect(),
                Err(error) => {
                    tracing::warn!("[stable] SYNC retry skipped, list_channels failed: {}", error);
                    return;
                }
            };
        for (uid, channel_id, live_sats, counterparty) in syncs {
            if !usable.contains(&uid) {
                continue;
            }
            // Rebuild the correction from committed books, never from the failed payload or
            // an in-memory allocation whose database save may have failed.
            let record = match self.db.load_channel(&uid.to_string()) {
                Ok(Some(record))
                    if record.channel_id == channel_id && live_sats >= record.backing_sats => record,
                Ok(_) => continue,
                Err(error) => {
                    tracing::error!("[stable] failed to load SYNC books for {}: {}", uid, error);
                    continue;
                }
            };
            // send_sync_message clears the queued retry only after recording its replacement.
            self.send_sync_message(
                ldk,
                uid,
                &channel_id,
                record.expected_usd,
                record.backing_sats,
                &counterparty,
            )
            .await;
        }
    }

    /// Queue a failed channel's retry once its backoff has elapsed. Past the attempt cap the
    /// channel waits for a delivered SYNC from any path, audited once rather than every tick.
    fn queue_due_sync_retries(&mut self, failed: &[String]) {
        let now = Self::unix_time_secs();
        let mut exhausted = std::collections::HashSet::new();
        for user_channel_id in failed {
            let Some(uid) = parse_user_channel_id(user_channel_id) else { continue };
            let (attempts, last_attempt_at) = match self.db.sync_retry_attempts(user_channel_id) {
                Ok(state) => state,
                Err(error) => {
                    tracing::error!("[stable] failed to load SYNC attempts for {}: {}", uid, error);
                    continue;
                }
            };
            if attempts >= SYNC_RETRY_MAX_ATTEMPTS {
                if !self.sync_retry_exhausted.contains(&uid) {
                    stable_channels::audit::audit_event(
                        "SYNC_RETRY_EXHAUSTED",
                        serde_json::json!({ "user_channel_id": user_channel_id, "attempts": attempts }),
                    );
                }
                exhausted.insert(uid);
                continue;
            }
            let due_at = last_attempt_at.saturating_add(sync_retry_delay_secs(attempts) as i64);
            if now >= due_at {
                self.startup_sync_pending.insert(uid);
            }
        }
        self.sync_retry_exhausted = exhausted;
    }

    async fn reconcile_sync_outcomes(&self, ldk: &dyn LdkServerCalls) -> anyhow::Result<()> {
        let now = Self::unix_time_secs();
        for (payment_id, recorded_at) in self.db.list_pending_sync_payments()? {
            let response = match ldk
                .get_payment_details(GetPaymentDetailsRequest {
                    payment_id: payment_id.clone(),
                })
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!("[stable] SYNC payment lookup failed for {}: {}", payment_id, error);
                    continue;
                }
            };
            let status = response.payment.as_ref().map(|payment| payment.status);
            if status == Some(PaymentStatus::Failed as i32) {
                self.db.mark_sync_payment_failed(&payment_id)?;
            } else if status == Some(PaymentStatus::Succeeded as i32) {
                let payment = response.payment.unwrap_or_default();
                self.db.mark_settlement_succeeded(
                    &payment_id,
                    payment.amount_msat,
                    payment.fee_paid_msat,
                    Some("outbound"),
                )?;
            } else if now.saturating_sub(recorded_at) > SYNC_PENDING_TIMEOUT_SECS as i64 {
                // No outcome after the timeout: LDK lost the payment or its HTLC is stuck. Replace it.
                self.db.mark_sync_payment_failed(&payment_id)?;
                stable_channels::audit::audit_event(
                    "SYNC_PENDING_ABANDONED",
                    serde_json::json!({
                        "payment_id": payment_id,
                        "age_secs": now.saturating_sub(recorded_at),
                        "ldk_status": status,
                    }),
                );
            }
        }
        Ok(())
    }

    /// Self-heal: if the in-memory list is empty (startup/reconnect reconcile skipped on a cold price cache), rebuild it from truth; a populated list is left untouched so a transient empty snapshot can't wipe it.
    pub async fn reconcile_if_empty(&mut self, ldk: &dyn LdkServerCalls, btc_price: f64) {
        // run_tick owns retries here and skips settlement on a tick that commits a correction.
        if !self.pending_book_updates.is_empty() || !self.pending_splices.is_empty() {
            return;
        }
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
        funding_txo: Option<String>,
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
            self.handle_channel_ready_splice(
                target_uid,
                funding_txo.as_deref(),
                ldk,
                btc_price,
            )
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
        let mut ready_detail = serde_json::json!({
            "channel_id": channel_id,
            "user_channel_id": user_channel_id,
            "funding_txo": funding_txo,
        });
        if let Some(detail) = ready_detail.as_object_mut() {
            detail.extend(crate::channel_audit::channel_snapshot_fields(&c));
        }
        stable_channels::audit::audit_event("CHANNEL_READY_TRACKED", ready_detail);
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
        if let Some(record) = custom_records
            .iter()
            .find(|record| record.type_num == SIGNED_STABILITY_TLV_TYPE)
        {
            self.handle_signed_stability_payment(
                record,
                payment_id.as_deref(),
                amount_msat,
                ldk,
                btc_price,
            )
            .await;
            // A malformed signed record must never downgrade to the unsigned marker included for
            // older mobile clients.
            return;
        }

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
            if let Some(_envelope) = crate::messages::parse_envelope(&raw) {
                // An envelope is a control message, even when its inner type is unknown or
                // malformed. Let the trade handler audit/drop it; never reinterpret it as a
                // stability payment. The trade settlement is recorded INSIDE the handler, only
                // after the signature verifies — a forged or unsigned envelope from any peer no
                // longer writes a settlement row before it is authenticated.
                self.handle_trade_payment(
                    &raw,
                    payment_id.as_deref(),
                    amount_msat,
                    ldk,
                    btc_price,
                )
                    .await;
            } else if rec.value.as_ref() == [1u8] {
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
            } else {
                stable_channels::audit::audit_event(
                    "LEGACY_STABILITY_MARKER_INVALID",
                    serde_json::json!({
                        "payment_id": payment_id,
                        "amount_msat": amount_msat,
                        "payload_len": rec.value.len(),
                    }),
                );
            }
            return;
        }
        // No stable TLV: plain receipt — emit audit so it's visible in the log.
        stable_channels::audit::audit_event(
            "PAYMENT_RECEIVED",
            serde_json::json!({ "payment_id": payment_id, "amount_msat": amount_msat }),
        );
    }

    async fn handle_signed_stability_payment(
        &mut self,
        record: &CustomTlvRecord,
        payment_id: Option<&str>,
        amount_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        if record.value.len()
            > stable_channels::constants::MAX_SIGNED_STABILITY_TLV_VALUE_BYTES
        {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_PAYLOAD_INVALID",
                serde_json::json!({
                    "payment_id": payment_id,
                    "reason": "oversize",
                    "payload_len": record.value.len(),
                }),
            );
            return;
        }
        let Ok(raw) = std::str::from_utf8(record.value.as_ref()) else {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_PAYLOAD_INVALID",
                serde_json::json!({ "payment_id": payment_id, "reason": "utf8" }),
            );
            return;
        };
        let Some(envelope) = stable_channels::stable::parse_stability_signed_envelope(raw) else {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_PAYLOAD_INVALID",
                serde_json::json!({ "payment_id": payment_id, "reason": "envelope" }),
            );
            return;
        };
        let Some(payload) =
            stable_channels::stable::parse_stability_payment_payload(&envelope.payload)
        else {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_PAYLOAD_INVALID",
                serde_json::json!({ "payment_id": payment_id, "reason": "fields" }),
            );
            return;
        };
        let (Some(payment_id), Some(received_msat)) = (payment_id, amount_msat) else {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_BINDING_INVALID",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                    "amount_msat": amount_msat,
                    "reason": "missing_payment_details",
                }),
            );
            return;
        };
        if payload.direction != StabilityPaymentDirection::UserToLsp {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_BINDING_INVALID",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                    "reason": "direction",
                }),
            );
            return;
        }
        if payload.amount_msat != received_msat {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_AMOUNT_MISMATCH",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                    "signed_amount_msat": payload.amount_msat,
                    "received_amount_msat": received_msat,
                }),
            );
            return;
        }
        let registration = match self.db.register_inbound_stability_settlement(
            &payload.settlement_id,
            payment_id,
            &payload.channel_id,
            payload.amount_msat,
            "user_to_lsp",
            raw,
        ) {
            Ok(registration) => registration,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "STABILITY_PAYMENT_REPLAY_CONFLICT",
                    serde_json::json!({
                        "settlement_id": payload.settlement_id,
                        "payment_id": payment_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if registration == InboundStabilityRegistration::Applied {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_REPLAY_IGNORED",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                }),
            );
            return;
        }
        if registration == InboundStabilityRegistration::Invalid {
            return;
        }

        let invalidate = |reason: &str| {
            let _ = self.db.finish_inbound_stability_settlement(
                &payload.settlement_id,
                "invalid",
                Some(reason),
            );
        };
        let received_at = match self
            .db
            .inbound_stability_settlement_received_at(&payload.settlement_id)
        {
            Ok(Some(received_at)) => received_at,
            Ok(None) => return,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "DB_READ_FAILED",
                    serde_json::json!({
                        "op": "inbound_stability_settlement_received_at",
                        "settlement_id": payload.settlement_id,
                        "payment_id": payment_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        // Evaluate expiry at the durable first-receipt time. A transient failure may be retried
        // later without turning an on-time, already-settled Lightning payment into an invalid one.
        if !stable_channels::stable::stability_payment_is_fresh(&payload, received_at) {
            invalidate("expired");
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_EXPIRED",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                    "created_at": payload.created_at,
                    "expires_at": payload.expires_at,
                    "received_at": received_at,
                }),
            );
            return;
        }

        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(response) => response.channels,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "STABILITY_PAYMENT_CHANNEL_LOOKUP_FAILED",
                    serde_json::json!({
                        "settlement_id": payload.settlement_id,
                        "payment_id": payment_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        let Some(channel) = channels
            .iter()
            .find(|channel| channel.channel_id.eq_ignore_ascii_case(&payload.channel_id))
            .cloned()
        else {
            invalidate("channel");
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_CHANNEL_MISMATCH",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                    "channel_id": payload.channel_id,
                }),
            );
            return;
        };
        let Some(user_channel_id) = parse_user_channel_id(&channel.user_channel_id) else {
            invalidate("user_channel_id");
            return;
        };
        let canonical_user_channel_id = format!("{}", user_channel_id);
        let Some(idx) = self
            .stable_channels
            .iter()
            .position(|stable| stable.user_channel_id == user_channel_id)
        else {
            let known_stable_channel = match self
                .db
                .get_active_user_channel_id_by_channel_id(&payload.channel_id)
            {
                Ok(channel_id) => channel_id.is_some(),
                Err(error) => {
                    stable_channels::audit::audit_event(
                        "DB_READ_FAILED",
                        serde_json::json!({
                            "op": "get_active_user_channel_id_by_channel_id",
                            "settlement_id": payload.settlement_id,
                            "payment_id": payment_id,
                            "error": error.to_string(),
                        }),
                    );
                    return;
                }
            };
            if !known_stable_channel {
                invalidate("channel_not_stable");
            }
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_CHANNEL_UNAVAILABLE",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                    "channel_id": payload.channel_id,
                    "will_retry": known_stable_channel,
                }),
            );
            return;
        };
        let signature_valid = match ldk
            .verify_signature(VerifySignatureRequest {
                message: envelope.payload.as_bytes().to_vec().into(),
                signature: envelope.signature,
                public_key: channel.counterparty_node_id.clone(),
            })
            .await
        {
            Ok(response) => response.valid,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "STABILITY_PAYMENT_SIGNATURE_CHECK_FAILED",
                    serde_json::json!({
                        "settlement_id": payload.settlement_id,
                        "payment_id": payment_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if !signature_valid {
            invalidate("signature");
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_SIGNATURE_INVALID",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                    "channel_id": payload.channel_id,
                }),
            );
            return;
        }
        if payload.expected_usd.to_bits()
            != self.stable_channels[idx].expected_usd.0.to_bits()
        {
            // The signed amount and local equilibrium bound the economic transition. A target
            // difference between independent peers is useful telemetry, but is not a reason to
            // discard an already-settled authenticated payment.
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_STATE_DIVERGENCE",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                    "signed_expected_usd": payload.expected_usd,
                    "local_expected_usd": self.stable_channels[idx].expected_usd.0,
                }),
            );
        }
        if btc_price <= 0.0 {
            stable_channels::audit::audit_event(
                "STABILITY_PAYMENT_PRICE_UNAVAILABLE",
                serde_json::json!({
                    "settlement_id": payload.settlement_id,
                    "payment_id": payment_id,
                }),
            );
            return;
        }

        let (_, their_sats) = channel_peer_balances(&channel);
        let amount_sats = received_msat / 1000;
        let mut allocation_expected_usd = self.stable_channels[idx].expected_usd.0;
        let mut backing_before = self.stable_channels[idx].backing_sats;
        let Some(mut backing_after) =
            stable_channels::stable::backing_after_user_to_lsp_stability(
                backing_before,
                allocation_expected_usd,
                btc_price,
                amount_sats,
                their_sats,
            )
        else {
            invalidate("allocation");
            return;
        };
        let mut native_after = their_sats.saturating_sub(backing_after);
        let amount_usd = amount_sats as f64
            / stable_channels::constants::SATS_IN_BTC as f64
            * btc_price;
        let persist = |backing_sats_before, backing_sats_after, native_sats_after| {
            self.db.record_signed_stability_payment_and_update_allocation(
                payment_id,
                &payload.settlement_id,
                received_msat,
                Some(amount_usd),
                Some(btc_price),
                &canonical_user_channel_id,
                backing_sats_before,
                backing_sats_after,
                native_sats_after,
            )
        };
        let persisted = match persist(backing_before, backing_after, native_after) {
            Err(ref error)
                if stable_channels::db::is_stale_inbound_stability_allocation(error) =>
            {
                let durable = match self.db.load_channel(&canonical_user_channel_id) {
                    Ok(Some(channel)) => channel,
                    Ok(None) => return,
                    Err(error) => {
                        stable_channels::audit::audit_event(
                            "STABILITY_PAYMENT_PERSIST_FAILED",
                            serde_json::json!({
                                "settlement_id": payload.settlement_id,
                                "payment_id": payment_id,
                                "stage": "reload_after_stale_allocation",
                                "error": error.to_string(),
                            }),
                        );
                        return;
                    }
                };
                let Some(reloaded_backing_after) =
                    stable_channels::stable::backing_after_user_to_lsp_stability(
                        durable.backing_sats,
                        durable.expected_usd,
                        btc_price,
                        amount_sats,
                        their_sats,
                    )
                else {
                    return;
                };
                allocation_expected_usd = durable.expected_usd;
                backing_before = durable.backing_sats;
                backing_after = reloaded_backing_after;
                native_after = their_sats.saturating_sub(backing_after);
                match persist(backing_before, backing_after, native_after) {
                    Ok(persisted) => persisted,
                    Err(error) => {
                        stable_channels::audit::audit_event(
                            "STABILITY_PAYMENT_PERSIST_FAILED",
                            serde_json::json!({
                                "settlement_id": payload.settlement_id,
                                "payment_id": payment_id,
                                "stage": "retry_after_stale_allocation",
                                "error": error.to_string(),
                            }),
                        );
                        return;
                    }
                }
            }
            Err(ref error) if stable_channels::db::is_missing_channel_row(error) => {
                if let Err(error) = self.db.save_channel(
                    &payload.channel_id,
                    &canonical_user_channel_id,
                    allocation_expected_usd,
                    backing_before,
                    their_sats.saturating_sub(backing_before),
                    self.stable_channels[idx].note.as_deref(),
                ) {
                    stable_channels::audit::audit_event(
                        "STABILITY_PAYMENT_PERSIST_FAILED",
                        serde_json::json!({
                            "settlement_id": payload.settlement_id,
                            "payment_id": payment_id,
                            "stage": "canonicalize_channel_row",
                            "error": error.to_string(),
                        }),
                    );
                    return;
                }
                match persist(backing_before, backing_after, native_after) {
                    Ok(persisted) => persisted,
                    Err(error) => {
                        stable_channels::audit::audit_event(
                            "STABILITY_PAYMENT_PERSIST_FAILED",
                            serde_json::json!({
                                "settlement_id": payload.settlement_id,
                                "payment_id": payment_id,
                                "stage": "retry_after_channel_canonicalization",
                                "error": error.to_string(),
                            }),
                        );
                        return;
                    }
                }
            }
            Ok(persisted) => persisted,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "STABILITY_PAYMENT_PERSIST_FAILED",
                    serde_json::json!({
                        "settlement_id": payload.settlement_id,
                        "payment_id": payment_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if persisted.is_new {
            let stable = &mut self.stable_channels[idx];
            stable.expected_usd = USD::from_f64(allocation_expected_usd);
            stable.latest_price = btc_price;
            stable.stable_receiver_btc = Bitcoin::from_sats(their_sats);
            stable.stable_receiver_usd = USD::from_bitcoin(stable.stable_receiver_btc, btc_price);
            stable.backing_sats = backing_after;
            stable.native_sats = native_after;
            stable_channels::stable::recompute_native(stable);
            self.spend_debounce.remove(&stable.user_channel_id);
        }
        stable_channels::audit::audit_event(
            "STABILITY_PAYMENT_V1_APPLIED",
            serde_json::json!({
                "settlement_id": payload.settlement_id,
                "payment_id": payment_id,
                "channel_id": payload.channel_id,
                "amount_msat": received_msat,
                "backing_sats_before": backing_before,
                "backing_sats_after": backing_after,
                "native_sats_after": native_after,
                "is_new": persisted.is_new,
            }),
        );
    }

    async fn retry_pending_signed_stability(
        &mut self,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        if !btc_price.is_finite() || btc_price <= 0.0 {
            return;
        }
        let pending = match self.db.pending_inbound_stability_settlements(32) {
            Ok(pending) => pending,
            Err(error) => {
                stable_channels::audit::audit_event(
                    "DB_READ_FAILED",
                    serde_json::json!({
                        "op": "pending_inbound_stability_settlements",
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        for settlement in pending {
            let record = CustomTlvRecord {
                type_num: SIGNED_STABILITY_TLV_TYPE,
                value: settlement.envelope.into_bytes().into(),
            };
            self.handle_signed_stability_payment(
                &record,
                Some(&settlement.payment_id),
                Some(settlement.amount_msat),
                ldk,
                btc_price,
            )
            .await;
        }
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
            if sc.expected_usd.0 < 0.01 && sc.backing_sats == 0 {
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
        // Amount-proportional settlement. Reduce the stable backing by exactly the sats
        // received — never blindly to equilibrium. The stability marker is unsigned and
        // carries no proof of the amount owed, so a token 1-sat payment must settle only
        // 1 sat of drift, not erase the entire above-par surplus and reclassify it as the
        // user's own native BTC. Floor at equilibrium so a (rounding) overpayment cannot
        // drive backing below the peg; clamp to the live balance so backing never exceeds it.
        let Some(settled_backing) = stable_channels::stable::backing_after_user_to_lsp_stability(
            sc.backing_sats,
            sc.expected_usd.0,
            btc_price,
            amount_sats,
            their_sats,
        ) else {
            stable_channels::audit::audit_event(
                "STABILITY_RECEIVE_UNATTRIBUTED",
                serde_json::json!({
                    "payment_id": payment_id,
                    "amount_msat": amount_msat,
                    "reason": "invalid_allocation_inputs",
                }),
            );
            return;
        };
        sc.backing_sats = settled_backing;
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
        let had_pending = !self.pending_book_updates.is_empty() || !self.pending_splices.is_empty();
        if !self.retry_pending_reconciliations(ldk, btc_price).await || had_pending {
            // Give the event loop a chance to process the events held behind this correction
            // before considering another balance-based deduction or stability payment.
            return;
        }
        // LDK Server's event stream is not replayable. Finish any receive that was durably
        // registered before a transient channel/signature/DB failure.
        self.retry_pending_signed_stability(ldk, btc_price).await;
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
            if self.pending_splices.contains_key(&sc.user_channel_id) {
                // A failed splice save must finish before a tick can change these books.
                continue;
            }
            if sc.expected_usd.0 < 0.01 && sc.backing_sats == 0 {
                continue;
            }
            let Some(c) = by_user_channel_id.get(&sc.user_channel_id) else { continue; };

            let (our_sats, their_sats) = channel_peer_balances(c);
            let mut proposed = sc.clone();
            proposed.stable_provider_btc = Bitcoin::from_sats(our_sats);
            proposed.stable_receiver_btc = Bitcoin::from_sats(their_sats);
            proposed.stable_provider_usd = USD::from_bitcoin(proposed.stable_provider_btc, btc_price);
            proposed.stable_receiver_usd = USD::from_bitcoin(proposed.stable_receiver_btc, btc_price);
            proposed.latest_price = btc_price;

            // Balance-truth backstop: live balance below backing means a spend went unreconciled (no PaymentForwarded) — deduct + SYNC. Debounced since outbound_capacity excludes in-flight HTLCs.
            let uid = sc.user_channel_id;
            if their_sats < sc.backing_sats {
                let count = {
                    let cnt = self.spend_debounce.entry(uid).or_insert(0);
                    *cnt = cnt.saturating_add(1);
                    *cnt
                };
                if count >= BACKSTOP_DEBOUNCE_TICKS {
                    if let Some(usd_deducted) =
                        stable_channels::stable::reconcile_outgoing(&mut proposed, btc_price)
                    {
                        if let Err(e) = self.db.save_channel(
                            &c.channel_id,
                            &format!("{}", uid),
                            proposed.expected_usd.0,
                            proposed.backing_sats,
                            proposed.native_sats,
                            proposed.note.as_deref(),
                        ) {
                            tracing::error!("[stable] backstop save_channel failed: {}", e);
                            stable_channels::audit::audit_event(
                                "DB_WRITE_FAILED",
                                serde_json::json!({ "op": "save_channel", "context": "backstop", "user_channel_id": format!("{}", uid), "channel_id": c.channel_id, "error": e.to_string() }),
                            );
                            let data = serde_json::json!({
                                "channel_id": c.channel_id, "user_channel_id": uid.to_string(),
                                "their_sats": their_sats, "usd_deducted": usd_deducted,
                                "new_expected_usd": proposed.expected_usd.0,
                                "new_backing_sats": proposed.backing_sats,
                            });
                            self.pending_book_updates.insert(uid, PendingBookUpdate {
                                channel_id: c.channel_id.clone(), proposed,
                                context: "backstop",
                                audits: vec![("BACKSTOP_STABLE_DEDUCTED", data)],
                                needs_sync: true,
                            });
                            // Preserve the exact failed correction and leave published books intact.
                            continue;
                        }
                        stable_channels::audit::audit_event(
                            "BACKSTOP_STABLE_DEDUCTED",
                            serde_json::json!({
                                "channel_id": c.channel_id,
                                "user_channel_id": format!("{}", uid),
                                "their_sats": their_sats,
                                "usd_deducted": usd_deducted,
                                "new_expected_usd": proposed.expected_usd.0,
                                "new_backing_sats": proposed.backing_sats,
                            }),
                        );
                        backstop_syncs.push((
                            uid,
                            c.channel_id.clone(),
                            proposed.expected_usd.0,
                            proposed.backing_sats,
                            proposed.counterparty.to_string(),
                        ));
                    }
                    self.spend_debounce.remove(&uid);
                }
            } else {
                self.spend_debounce.remove(&uid);
            }
            *sc = proposed;

            let stable_usd_value = if sc.backing_sats > 0 {
                (sc.backing_sats as f64 / 100_000_000.0) * btc_price
            } else {
                sc.stable_receiver_usd.0
            };
            let target = sc.expected_usd.0;
            let percent_from_par =
                (((stable_usd_value - target) / target.max(0.01)) * 100.0).abs();
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
                    let settlement_id = stable_channels::stable::new_stability_settlement_id();
                    let created_at = now.max(0) as u64;
                    let expires_at = created_at.saturating_add(STABILITY_PAYMENT_AUTH_TTL_SECS);
                    let payload = match stable_channels::stable::build_stability_payment_payload(
                        &settlement_id,
                        &c.channel_id,
                        amount_msat,
                        StabilityPaymentDirection::LspToUser,
                        sc.expected_usd.0,
                        created_at,
                        expires_at,
                    ) {
                        Ok(payload) => payload,
                        Err(error) => {
                            stable_channels::audit::audit_event(
                                "STABILITY_PAYMENT_SERIALIZE_FAILED",
                                serde_json::json!({
                                    "channel_id": c.channel_id,
                                    "user_channel_id": format!("{}", sc.user_channel_id),
                                    "settlement_id": settlement_id,
                                    "amount_msat": amount_msat,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                    };
                    let signature = match ldk
                        .sign_message(SignMessageRequest {
                            message: payload.as_bytes().to_vec().into(),
                        })
                        .await
                    {
                        Ok(response) if !response.signature.is_empty() => response.signature,
                        Ok(_) => {
                            stable_channels::audit::audit_event(
                                "STABILITY_PAYMENT_SIGN_FAILED",
                                serde_json::json!({
                                    "channel_id": c.channel_id,
                                    "user_channel_id": format!("{}", sc.user_channel_id),
                                    "settlement_id": settlement_id,
                                    "reason": "empty_signature",
                                }),
                            );
                            continue;
                        }
                        Err(error) => {
                            stable_channels::audit::audit_event(
                                "STABILITY_PAYMENT_SIGN_FAILED",
                                serde_json::json!({
                                    "channel_id": c.channel_id,
                                    "user_channel_id": format!("{}", sc.user_channel_id),
                                    "settlement_id": settlement_id,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                    };
                    let envelope = match stable_channels::stable::build_stability_signed_envelope(
                        payload,
                        signature,
                    ) {
                        Ok(envelope) => envelope,
                        Err(error) => {
                            stable_channels::audit::audit_event(
                                "STABILITY_PAYMENT_SERIALIZE_FAILED",
                                serde_json::json!({
                                    "channel_id": c.channel_id,
                                    "user_channel_id": format!("{}", sc.user_channel_id),
                                    "settlement_id": settlement_id,
                                    "stage": "envelope",
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                    };
                    let send_req = SpontaneousSendRequest {
                        amount_msat,
                        node_id: sc.counterparty.to_string(),
                        route_parameters: None,
                        preimage: None,
                        // Keep the marker during the mobile rollout. Upgraded receivers must
                        // prefer and validate the signed record whenever both are present.
                        custom_tlvs: vec![
                            CustomTlvRecord {
                                type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
                                value: vec![1u8].into(),
                            },
                            CustomTlvRecord {
                                type_num: SIGNED_STABILITY_TLV_TYPE,
                                value: envelope.into_bytes().into(),
                            },
                        ],
                    };
                    let channel_id_clone = c.channel_id.clone();
                    let user_channel_id_clone = c.user_channel_id.clone();
                    let expected_usd_for_db = sc.expected_usd.0;
                    let note_for_db = sc.note.clone();
                    let backing_before = sc.backing_sats;
                    let backing_after =
                        ((sc.expected_usd.0 / btc_price) * 100_000_000.0) as u64;
                    let native_before = sc.native_sats;
                    let last_stability_payment_before = sc.last_stability_payment;
                    let counterparty_for_db = sc.counterparty.to_string();
                    match ldk.spontaneous_send(send_req).await {
                        Ok(resp) => {
                            stable_channels::audit::audit_event(
                                "STABILITY_PAYMENT_V1_SENT",
                                serde_json::json!({
                                    "payment_id": resp.payment_id,
                                    "settlement_id": settlement_id,
                                    "channel_id": channel_id_clone,
                                    "user_channel_id": user_channel_id_clone,
                                    "amount_msat": amount_msat,
                                    "direction": "lsp_to_user",
                                }),
                            );
                            let persisted = if resp.payment_id.is_empty() {
                                false
                            } else {
                                match self.db.record_stability_settlement_with_rollback(
                                    &resp.payment_id,
                                    &user_channel_id_clone,
                                    &channel_id_clone,
                                    backing_before,
                                    backing_after,
                                    native_before,
                                    expected_usd_for_db,
                                    last_stability_payment_before,
                                    amount_msat,
                                    direction,
                                    &counterparty_for_db,
                                    note_for_db.as_deref(),
                                ) {
                                    Ok(true) => true,
                                    Ok(false) => {
                                        stable_channels::audit::audit_event(
                                            "DB_WRITE_FAILED",
                                            serde_json::json!({ "op": "record_stability_settlement_with_rollback", "kind": "stability", "payment_id": resp.payment_id.clone(), "user_channel_id": user_channel_id_clone.clone(), "channel_id": channel_id_clone.clone(), "error": "duplicate payment id or invalid rollback metadata" }),
                                        );
                                        false
                                    },
                                    Err(e) => {
                                        tracing::error!(
                                            "[stable] record_settlement (stability) failed: {}",
                                            e
                                        );
                                        stable_channels::audit::audit_event(
                                            "DB_WRITE_FAILED",
                                            serde_json::json!({ "op": "record_stability_settlement_with_rollback", "kind": "stability", "payment_id": resp.payment_id.clone(), "user_channel_id": user_channel_id_clone.clone(), "channel_id": channel_id_clone.clone(), "error": e.to_string() }),
                                        );
                                        false
                                    },
                                }
                            };
                            sc.last_stability_payment = now;
                            if persisted {
                                // The database and ledger own this optimistic transition. Only
                                // update the cache after that transaction commits.
                                sc.backing_sats = backing_after;
                            }
                            self.stability_throttle.insert(
                                sc.user_channel_id,
                                (if persisted { "payment_sent" } else { "payment_persist_failed" }.to_string(), stable_usd_value),
                            );
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
    /// Returns true only after the accepted payment ID and version are saved for outcome tracking.
    /// Acceptance is not delivery: later failures are retried from the latest committed books.
    /// Allocation state is unchanged, while the version is durably reserved before signing.
    pub async fn send_sync_message(
        &mut self,
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
            preimage: None,
            custom_tlvs: vec![CustomTlvRecord {
                type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
                value: envelope.into_bytes().into(),
            }],
        };
        match ldk.spontaneous_send(req).await {
            Ok(resp) => {
                if let Err(e) = self.db.record_sync_payment(
                    &resp.payment_id,
                    &user_channel_id.to_string(),
                    sync_version,
                ) {
                    stable_channels::audit::audit_event(
                        "SYNC_MESSAGE_FAILED",
                        serde_json::json!({ "stage": "record_payment", "payment_id": resp.payment_id, "user_channel_id": user_channel_id.to_string(), "error": e.to_string() }),
                    );
                    return false;
                }
                self.startup_sync_pending.remove(&user_channel_id);
                stable_channels::audit::audit_event(
                    "SYNC_MESSAGE_SENT",
                    serde_json::json!({
                        "user_channel_id": format!("{}", user_channel_id),
                        "channel_id": channel_id,
                        "expected_usd": expected_usd,
                        "backing_sats": backing_sats,
                        "sync_version": sync_version,
                        "payment_id": resp.payment_id,
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
        skimmed_fee_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let total_sats = outbound_amount_forwarded_msat.saturating_add(fee_msat) / 1000;
        let forward_detail = serde_json::json!({
            "prev_user_channel_id": prev_user_channel_id,
            "next_user_channel_id": next_user_channel_id,
            "prev_channel_id": prev_channel_id,
            "next_channel_id": next_channel_id,
            "prev_node_id": prev_node_id,
            "next_node_id": next_node_id,
            "forwarded_msat": outbound_amount_forwarded_msat,
            "fee_msat": fee_msat,
            "skimmed_fee_msat": skimmed_fee_msat,
            "total_sats": total_sats,
        });
        let fingerprint = stable_channels::db::forward_fingerprint(
            &prev_channel_id,
            &next_channel_id,
            Some(outbound_amount_forwarded_msat),
            Some(fee_msat),
        );
        let draft = stable_channels::ledger::LedgerEventDraft::from_audit_event(
            "PAYMENT_FORWARDED",
            forward_detail,
        );
        if let Err(error) = self
            .db
            .append_forwarded_event_if_unseen(&fingerprint, &draft)
        {
            stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({
                    "op": "append_forwarded_event_if_unseen",
                    "fingerprint": fingerprint,
                    "error": error.to_string(),
                }),
            );
        }

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
            if (sc.expected_usd.0 <= 0.0 && sc.backing_sats == 0) || btc_price <= 0.0 {
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
        funding_txo: Option<&str>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        if self.pending_book_updates.contains_key(&uid) {
            if self.persist_pending_book_update(uid) {
                self.retry_startup_sync(ldk).await;
            }
            return;
        }
        let Some(idx) = self
            .stable_channels
            .iter()
            .position(|sc| sc.user_channel_id == uid)
        else {
            self.pending_splices.remove(&uid);
            return;
        };
        self.pending_splices
            .insert(uid, funding_txo.map(str::to_owned));
        if btc_price <= 0.0 {
            return;
        }
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

        // Reconcile a copy so a failed save also preserves the old channel id and cooldown.
        let mut proposed = self.stable_channels[idx].clone();
        let before_receiver_sats = proposed.stable_receiver_btc.sats;
        let (splice_direction, splice_amount_sats) =
            splice_balance_change(before_receiver_sats, their_sats);
        proposed.channel_id =
            ldk_node::lightning::ln::types::ChannelId::from_bytes(new_channel_id_bytes);
        proposed.stable_provider_btc = Bitcoin::from_sats(our_sats);
        proposed.stable_receiver_btc = Bitcoin::from_sats(their_sats);
        proposed.stable_provider_usd = USD::from_bitcoin(proposed.stable_provider_btc, btc_price);
        proposed.stable_receiver_usd = USD::from_bitcoin(proposed.stable_receiver_btc, btc_price);
        proposed.latest_price = btc_price;
        stable_channels::stable::recompute_native(&mut proposed);
        let usd_deducted = stable_channels::stable::reconcile_outgoing(&mut proposed, btc_price);
        let ucid_str = uid.to_string();
        let mut audits = Vec::new();
        if let Some(d) = usd_deducted {
            audits.push((
                "SPLICE_OUT_STABLE_DEDUCTED",
                serde_json::json!({
                    "channel_id": channel_id_hex,
                    "user_channel_id": ucid_str,
                    "usd_deducted": d,
                    "new_expected_usd": proposed.expected_usd.0,
                }),
            ));
        }
        audits.push((
            "CHANNEL_READY_SPLICE",
            serde_json::json!({
                "channel_id": channel_id_hex,
                "user_channel_id": ucid_str,
                "funding_txo": funding_txo,
                "dedup_key": funding_txo.map(|outpoint| format!("lsp:channel-ready-splice:{ucid_str}:{outpoint}")),
                "direction": splice_direction,
                "amount_sats": splice_amount_sats,
                "before_live_receiver_sats": before_receiver_sats,
                "after_live_receiver_sats": their_sats,
                "before_btc_price": btc_price,
                "btc_price": btc_price,
                "deducted": usd_deducted.is_some(),
            }),
        ));
        self.pending_book_updates.insert(
            uid,
            PendingBookUpdate {
                channel_id: channel_id_hex,
                proposed,
                context: "handle_channel_ready_splice",
                audits,
                needs_sync: usd_deducted.is_some(),
            },
        );
        if self.persist_pending_book_update(uid) {
            self.retry_startup_sync(ldk).await;
        }
    }

    /// Parse a TRADE_V1 envelope, verify it against the channel counterparty, validate against
    /// balance, and apply the new USD target. Drops (with an audit line) on any failure.
    pub async fn handle_trade_payment(
        &mut self,
        raw: &str,
        inbound_payment_id: Option<&str>,
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
        if payload.trade_id.is_none()
            && (payload.expected_usd < 0.0 || !payload.expected_usd.is_finite())
        {
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
        // channel_id is authoritative when present; only requests that omit it may use the
        // legacy node-local user_channel_id fallback.
        let chan = channels.into_iter().find(|c| match payload.channel_id.as_deref() {
            Some(channel_id) => c.channel_id == channel_id,
            None => payload.user_channel_id.as_deref().is_some_and(|user_channel_id| {
                let wanted = parse_user_channel_id(user_channel_id);
                wanted.is_some() && wanted == parse_user_channel_id(&c.user_channel_id)
            }),
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
        stable_channels::audit::audit_event(
            "TRADE_PROTOCOL_PATH",
            serde_json::json!({
                "path": if payload.trade_id.is_some() { "hardened" } else { "legacy" },
                "channel_id": chan.channel_id,
                "user_channel_id": chan.user_channel_id.clone(),
            }),
        );

        // Verify-then-write: the settlement row is recorded only now that the envelope's
        // signature is verified against the channel counterparty. An unauthenticated peer's
        // forged TLV is dropped above without ever touching the settlements table.
        if let Some(pid) = inbound_payment_id {
            if let Err(e) = self.db.record_settlement(pid, "trade") {
                tracing::error!("[stable] record_settlement (inbound trade) failed: {}", e);
                stable_channels::audit::audit_event(
                    "DB_WRITE_FAILED",
                    serde_json::json!({ "op": "record_settlement", "kind": "trade", "payment_id": pid, "error": e.to_string() }),
                );
            }
        }

        // A trade id opts into durable correlated results. Legacy mobile-shaped requests continue
        // through the original silent-rejection / ordinary-SYNC path below.
        if let Some(trade_id) = payload.trade_id.as_deref() {
            if !stable_channels::trade::is_trade_id(trade_id)
                || !payload
                    .channel_id
                    .as_deref()
                    .is_some_and(stable_channels::trade::is_channel_id)
            {
                stable_channels::audit::audit_event(
                    "TRADE_CORRELATION_INVALID",
                    serde_json::json!({ "channel_id": chan.channel_id }),
                );
                return;
            }
            let Some(inbound_payment_id) = inbound_payment_id
                .filter(|payment_id| stable_channels::trade::is_payment_id(payment_id))
            else {
                stable_channels::audit::audit_event(
                    "TRADE_PAYMENT_UNATTRIBUTABLE",
                    serde_json::json!({ "channel_id": chan.channel_id }),
                );
                return;
            };
            let Some(received_msat) = amount_msat else {
                stable_channels::audit::audit_event(
                    "TRADE_PAYMENT_UNATTRIBUTABLE",
                    serde_json::json!({ "channel_id": chan.channel_id }),
                );
                return;
            };
            let request_hash = stable_channels::trade::request_hash(envelope.payload.as_bytes());
            let now = Self::unix_time_secs();

            match self.db.trade_decision_by_payment(inbound_payment_id) {
                Ok(Some(decision)) => {
                    if decision.trade_id == trade_id && decision.request_hash == request_hash {
                        let _ = self.db.requeue_exact_trade_response(
                            inbound_payment_id,
                            trade_id,
                            &request_hash,
                            now,
                        );
                    }
                    return;
                }
                Ok(None) => {}
                Err(_) => {
                    self.reject_correlated_trade(
                        ldk,
                        inbound_payment_id,
                        trade_id,
                        &request_hash,
                        &chan.channel_id,
                        &chan.user_channel_id,
                        &chan.counterparty_node_id,
                        TradeRejectionReason::InternalFailure,
                    )
                    .await;
                    return;
                }
            }
            match self.db.trade_decision_by_trade_id(trade_id) {
                Ok(Some(_)) => {
                    stable_channels::audit::audit_event(
                        "TRADE_ID_REUSED",
                        serde_json::json!({ "trade_id": trade_id }),
                    );
                    return;
                }
                Ok(None) => {}
                Err(_) => {
                    self.reject_correlated_trade(
                        ldk,
                        inbound_payment_id,
                        trade_id,
                        &request_hash,
                        &chan.channel_id,
                        &chan.user_channel_id,
                        &chan.counterparty_node_id,
                        TradeRejectionReason::InternalFailure,
                    )
                    .await;
                    return;
                }
            }

            macro_rules! reject_correlated {
                ($reason:expr) => {{
                    self.reject_correlated_trade(
                        ldk,
                        inbound_payment_id,
                        trade_id,
                        &request_hash,
                        &chan.channel_id,
                        &chan.user_channel_id,
                        &chan.counterparty_node_id,
                        $reason,
                    )
                    .await;
                    return;
                }};
            }

            if !payload.expected_usd.is_finite() || payload.expected_usd < 0.0 {
                reject_correlated!(TradeRejectionReason::InvalidAmount);
            }
            let timestamp_valid = payload.ts != 0
                && now >= 0
                && (now as u64).abs_diff(payload.ts)
                    <= stable_channels::constants::TRADE_RESULT_TIMEOUT_SECS;
            if !timestamp_valid {
                reject_correlated!(TradeRejectionReason::StaleRequest);
            }
            let Some(target_uid) = parse_user_channel_id(&chan.user_channel_id) else {
                reject_correlated!(TradeRejectionReason::InternalFailure);
            };
            let Some(current) = self
                .stable_channels
                .iter()
                .find(|channel| channel.user_channel_id == target_uid)
                .cloned()
            else {
                reject_correlated!(TradeRejectionReason::InternalFailure);
            };
            let new_expected =
                stable_channels::stable::normalize_trade_expected_usd(payload.expected_usd);
            if stable_channels::trade::target_matches(current.expected_usd.0, new_expected) {
                reject_correlated!(TradeRejectionReason::InvalidAmount);
            }
            let Some(quote_price) = payload.quote_price else {
                reject_correlated!(TradeRejectionReason::InvalidQuote);
            };
            if !quote_price.is_finite()
                || quote_price <= 0.0
                || !btc_price.is_finite()
                || btc_price <= 0.0
            {
                reject_correlated!(TradeRejectionReason::InvalidQuote);
            }
            let Some(expected_fee_msat) = expected_trade_fee_msat(
                current.expected_usd.0,
                new_expected,
                quote_price,
            ) else {
                reject_correlated!(TradeRejectionReason::InvalidFee);
            };
            let tolerance_msat = trade_fee_tolerance_msat(expected_fee_msat, true);
            if received_msat.abs_diff(expected_fee_msat) > tolerance_msat {
                reject_correlated!(TradeRejectionReason::InvalidFee);
            }
            let quote_deviation_percent =
                ((quote_price - btc_price) / btc_price * 100.0).abs();
            if quote_deviation_percent > MAX_TRADE_QUOTE_DEVIATION_PERCENT {
                reject_correlated!(TradeRejectionReason::QuoteDeviation);
            }
            let (our_sats, their_sats) = channel_peer_balances(&chan);
            let receiver_usd = USD::from_bitcoin(Bitcoin::from_sats(their_sats), btc_price).0;
            if new_expected > receiver_usd {
                reject_correlated!(TradeRejectionReason::InsufficientCapacity);
            }

            let mut updated = current.clone();
            updated.stable_provider_btc = Bitcoin::from_sats(our_sats);
            updated.stable_receiver_btc = Bitcoin::from_sats(their_sats);
            updated.stable_provider_usd =
                USD::from_bitcoin(updated.stable_provider_btc, btc_price);
            updated.stable_receiver_usd =
                USD::from_bitcoin(updated.stable_receiver_btc, btc_price);
            updated.latest_price = btc_price;
            if !stable_channels::stable::apply_trade(&mut updated, new_expected, btc_price) {
                let reason = if new_expected < current.expected_usd.0
                    && (new_expected == 0.0
                        || trade_reduction_exhausts_backing(
                            current.backing_sats,
                            current.expected_usd.0,
                            new_expected,
                            btc_price,
                        ))
                {
                    TradeRejectionReason::SettlementRequired
                } else {
                    TradeRejectionReason::UnsafeAllocation
                };
                reject_correlated!(reason);
            }
            // Trade-entry only: never apply the cap in settlement/reconciliation.
            if max_stabilization_rejected(self.enforce_max_stabilization, &chan,
                current.expected_usd.0, new_expected, updated.backing_sats, "correlated")
            {
                reject_correlated!(TradeRejectionReason::InsufficientCapacity);
            }
            let sync_version = match self
                .db
                .candidate_sync_version(&format!("{}", target_uid))
            {
                Ok(version) => version,
                Err(_) => reject_correlated!(TradeRejectionReason::InternalFailure),
            };
            let response_user_channel_id = payload
                .user_channel_id
                .as_deref()
                .unwrap_or(&chan.user_channel_id);
            let acceptance_payload = crate::messages::build_trade_sync_payload(
                &chan.channel_id,
                response_user_channel_id,
                updated.expected_usd.0,
                updated.backing_sats,
                sync_version,
                trade_id,
                inbound_payment_id,
                &request_hash,
            );
            let signature = match ldk
                .sign_message(SignMessageRequest {
                    message: acceptance_payload.as_bytes().to_vec().into(),
                })
                .await
            {
                Ok(response) => response.signature,
                Err(_) => reject_correlated!(TradeRejectionReason::InternalFailure),
            };
            let response_envelope =
                crate::messages::build_envelope(acceptance_payload, signature);
            let native_sats = their_sats.saturating_sub(updated.backing_sats);
            match self.db.persist_trade_acceptance(
                inbound_payment_id,
                trade_id,
                &request_hash,
                &chan.channel_id,
                &format!("{}", target_uid),
                &chan.counterparty_node_id,
                updated.expected_usd.0,
                updated.backing_sats,
                native_sats,
                sync_version,
                now,
                &response_envelope,
            ) {
                Ok(true) => {
                    updated.native_sats = native_sats;
                    updated.native_channel_btc = Bitcoin::from_sats(native_sats);
                    if let Some(in_memory) = self
                        .stable_channels
                        .iter_mut()
                        .find(|channel| channel.user_channel_id == target_uid)
                    {
                        *in_memory = updated.clone();
                    }
                    stable_channels::audit::audit_event(
                        "TRADE_ACCEPTED",
                        serde_json::json!({
                            "protocol_path": "hardened",
                            "trade_id": trade_id,
                            "trade_payment_id": inbound_payment_id,
                            "request_hash": request_hash,
                            "expected_usd": updated.expected_usd.0,
                            "backing_sats": updated.backing_sats,
                            "sync_version": sync_version,
                        }),
                    );
                }
                Ok(false) | Err(_) => {
                    reject_correlated!(TradeRejectionReason::InternalFailure);
                }
            }
            return;
        }

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
        let new_expected =
            stable_channels::stable::normalize_trade_expected_usd(payload.expected_usd);

        let fee_price = payload.quote_price.unwrap_or(btc_price);
        let Some(expected_fee_msat) = expected_trade_fee_msat(
            current_expected_usd,
            new_expected,
            fee_price,
        ) else {
            stable_channels::audit::audit_event(
                "TRADE_FEE_INVALID",
                serde_json::json!({
                    "reason": "fee inputs are invalid",
                    "old_expected_usd": current_expected_usd,
                    "new_expected_usd": new_expected,
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
                    "new_expected_usd": new_expected,
                    "fee_price": fee_price,
                    "channel_id": chan.channel_id.clone(),
                    "user_channel_id": chan.user_channel_id.clone(),
                }),
            );
            return;
        }
        let (our_sats, their_sats) = channel_peer_balances(&chan);
        let quoted_trade = match payload.quote_price {
            Some(quote_price) => {
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
                Some((quote_price, quote_deviation_percent))
            }
            None => None,
        };

        // The quote is a consent bound only. Capacity and allocation always use the LSP's price.
        // Never admit a target above the locally valued balance: the stability threshold is a
        // payment deadband, not extra trade capacity.
        let receiver_usd = USD::from_bitcoin(Bitcoin::from_sats(their_sats), btc_price).0;
        if new_expected > receiver_usd {
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
            let mut updated = sc.clone();
            updated.stable_provider_btc = Bitcoin::from_sats(our_sats);
            updated.stable_receiver_btc = Bitcoin::from_sats(their_sats);
            updated.stable_provider_usd = USD::from_bitcoin(updated.stable_provider_btc, btc_price);
            updated.stable_receiver_usd = USD::from_bitcoin(updated.stable_receiver_btc, btc_price);
            updated.latest_price = btc_price;
            if !stable_channels::stable::apply_trade(&mut updated, new_expected, btc_price) {
                stable_channels::audit::audit_event(
                    "TRADE_ALLOCATION_REJECTED",
                    serde_json::json!({
                        "channel_id": channel_id_hex,
                        "user_channel_id": format!("{}", target_uid),
                        "current_expected_usd": sc.expected_usd.0,
                        "new_expected_usd": new_expected,
                        "current_backing_sats": sc.backing_sats,
                        "live_receiver_sats": their_sats,
                        "lsp_price": btc_price,
                        "reason": "target delta cannot preserve the current stability drift",
                    }),
                );
                return;
            }
            // Trade-entry only; reductions may remain above the cap after price drift.
            if max_stabilization_rejected(
                self.enforce_max_stabilization,
                &chan,
                sc.expected_usd.0,
                new_expected,
                updated.backing_sats,
                "legacy",
            ) {
                return;
            }
            *sc = updated;
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
                "protocol_path": "legacy",
                "channel_id": channel_id_hex,
                "user_channel_id": ucid_str,
                "new_expected_usd": expected_usd_f,
                "backing_sats": backing,
                "native_sats": native,
                "quote_price": quoted_trade.map(|(price, _)| price),
                "lsp_price": btc_price,
                "quote_deviation_percent": quoted_trade.map(|(_, deviation)| deviation),
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

    /// Compatibility/test entry point. Production passes the actual inbound LDK payment id above.
    pub async fn handle_trade_message(
        &mut self,
        raw: &str,
        inbound_payment_id: Option<&str>,
        amount_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let synthetic_payment_id = stable_channels::trade::request_hash(raw.as_bytes());
        self.handle_trade_payment(
            raw,
            inbound_payment_id.or(Some(&synthetic_payment_id)),
            amount_msat,
            ldk,
            btc_price,
        )
        .await;
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
        GetBalancesRequest, GetBalancesResponse, GetPaymentDetailsRequest,
        GetPaymentDetailsResponse, ListForwardedPaymentsRequest, ListForwardedPaymentsResponse,
        ListPaymentsRequest, ListPaymentsResponse, ListPeersRequest, ListPeersResponse,
    };
    use ldk_server_client::ldk_server_grpc::types::{
        Channel as GrpcChannel, ForwardedPayment as GrpcForwardedPayment, HtlcLocator,
        Payment as GrpcPayment, PaymentStatus,
        PendingSweepBalance as GrpcPendingSweepBalance, Peer as GrpcPeer,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
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
        pub verify_calls: StdMutex<Vec<VerifySignatureRequest>>,
        pub forwarded: StdMutex<Vec<GrpcForwardedPayment>>,
        pub forward_next_page_token: StdMutex<Option<String>>,
        pub forward_calls: AtomicUsize,
        pub sweeps: StdMutex<Vec<GrpcPendingSweepBalance>>,
        pub peers: StdMutex<Vec<GrpcPeer>>,
        pub payments: StdMutex<Vec<GrpcPayment>>,
        pub tracking_mode: StdMutex<i32>,
        pub tracking_mode_fails: bool,
        pub payments_page_size: StdMutex<Option<usize>>,
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
                verify_calls: StdMutex::new(Vec::new()),
                forwarded: StdMutex::new(Vec::new()),
                forward_next_page_token: StdMutex::new(None),
                forward_calls: AtomicUsize::new(0),
                sweeps: StdMutex::new(Vec::new()),
                peers: StdMutex::new(Vec::new()),
                payments: StdMutex::new(Vec::new()),
                tracking_mode: StdMutex::new(0),
                tracking_mode_fails: false,
                payments_page_size: StdMutex::new(None),
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
        pub fn with_forward_cursor(self, token: String) -> Self {
            *self.forward_next_page_token.lock().unwrap() = Some(token);
            self
        }
        pub fn with_sweeps(self, s: Vec<GrpcPendingSweepBalance>) -> Self { *self.sweeps.lock().unwrap() = s; self }
        pub fn with_peers(self, p: Vec<GrpcPeer>) -> Self { *self.peers.lock().unwrap() = p; self }
        pub fn with_payments(self, p: Vec<GrpcPayment>) -> Self { *self.payments.lock().unwrap() = p; self }
        pub fn with_tracking_mode(self, mode: ldk_server_client::ldk_server_grpc::types::ForwardedPaymentTrackingMode) -> Self {
            *self.tracking_mode.lock().unwrap() = mode as i32;
            self
        }
        pub fn with_payments_page_size(self, size: usize) -> Self { *self.payments_page_size.lock().unwrap() = Some(size); self }
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
            let mut sends = self.sends.lock().unwrap();
            sends.push(req);
            Ok(SpontaneousSendResponse {
                payment_id: if sends.len() == 1 {
                    "fake-payment-id".to_string()
                } else {
                    format!("fake-payment-id-{}", sends.len())
                },
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
            req: VerifySignatureRequest,
        ) -> Result<VerifySignatureResponse, LdkServerError> {
            self.verify_calls.lock().unwrap().push(req);
            Ok(VerifySignatureResponse {
                valid: self.verify_should_pass,
            })
        }
        async fn list_forwarded_payments(&self, _req: ListForwardedPaymentsRequest)
            -> Result<ListForwardedPaymentsResponse, LdkServerError> {
            self.forward_calls.fetch_add(1, Ordering::SeqCst);
            Ok(ListForwardedPaymentsResponse {
                forwarded_payments: self.forwarded.lock().unwrap().clone(),
                next_page_token: self.forward_next_page_token.lock().unwrap().clone(),
            })
        }
        async fn get_balances(&self, _req: GetBalancesRequest)
            -> Result<GetBalancesResponse, LdkServerError> {
            Ok(GetBalancesResponse { pending_balances_from_channel_closures: self.sweeps.lock().unwrap().clone(), ..Default::default() })
        }
        async fn list_peers(&self, _req: ListPeersRequest)
            -> Result<ListPeersResponse, LdkServerError> {
            Ok(ListPeersResponse { peers: self.peers.lock().unwrap().clone() })
        }
        async fn list_payments(&self, _req: ListPaymentsRequest)
            -> Result<ListPaymentsResponse, LdkServerError> {
            // Newest first, like LDK Node; a page size cuts the list and signals that more pages exist.
            let payments = self.payments.lock().unwrap().clone();
            match *self.payments_page_size.lock().unwrap() {
                Some(size) if payments.len() > size => Ok(ListPaymentsResponse {
                    payments: payments.into_iter().take(size).collect(),
                    next_page_token: Some("next-page".into()),
                }),
                _ => Ok(ListPaymentsResponse { payments, next_page_token: None }),
            }
        }
        async fn get_payment_details(&self, req: GetPaymentDetailsRequest)
            -> Result<GetPaymentDetailsResponse, LdkServerError> {
            Ok(GetPaymentDetailsResponse {
                payment: self
                    .payments
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|payment| payment.payment_id == req.payment_id)
                    .cloned(),
            })
        }
        async fn get_forwarded_payment_tracking_mode(&self, _req: GetForwardedPaymentTrackingModeRequest)
            -> Result<GetForwardedPaymentTrackingModeResponse, LdkServerError> {
            if self.tracking_mode_fails {
                return Err(LdkServerError::new(LdkServerErrorCode::InternalServerError, "tracking mode unavailable"));
            }
            Ok(GetForwardedPaymentTrackingModeResponse { mode: *self.tracking_mode.lock().unwrap() })
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
            None,
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
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            None,
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
                ("pay_test_1".to_string(), "trade".to_string()),
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
            None,       // skimmed_fee_msat
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
            None,
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
            None,
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
            None,
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
            None,       // skimmed_fee_msat
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
        let (mut mgr, fake) = seed_forwarded_fixture().await;
        *fake.channels.lock().unwrap() = vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX,
            100_000, 95_000_000, true,
        )];
        mgr.handle_payment_forwarded(
            USER_CHANNEL_ID_DECIMAL.to_string(),
            Some("outbound-ucid".to_string()),
            "prev-chan-hex".to_string(),
            "next-chan-hex".to_string(),
            "prev-node-pubkey".to_string(),
            "next-node-pubkey".to_string(),
            45_000_000,
            0,
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        ).await;
        let page = mgr
            .db
            .list_ledger_events(&stable_channels::ledger::LedgerQuery {
                identifier: Some("prev-chan-hex".into()),
                limit: 20,
                ..Default::default()
            })
            .unwrap();
        let data = page
            .events
            .iter()
            .find(|event| event.event_type == "PAYMENT_FORWARDED")
            .expect("PAYMENT_FORWARDED must be emitted");
        assert_eq!(data.detail["prev_user_channel_id"], USER_CHANNEL_ID_DECIMAL, "inbound leg must be recorded");
        assert_eq!(data.detail["next_user_channel_id"], "outbound-ucid", "outbound leg must be recorded");
        assert_eq!(data.detail["prev_node_id"], "prev-node-pubkey");
        assert_eq!(data.detail["next_node_id"], "next-node-pubkey");
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
            None,
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
        assert_eq!(sends[0].custom_tlvs.len(), 2);
        assert_eq!(
            sends[0].custom_tlvs[0].type_num,
            stable_channels::constants::STABLE_CHANNEL_TLV_TYPE
        );
        assert_eq!(sends[0].custom_tlvs[0].value.as_ref(), [1u8]);
        assert_eq!(
            sends[0].custom_tlvs[1].type_num,
            stable_channels::constants::SIGNED_STABILITY_TLV_TYPE
        );
        let raw = std::str::from_utf8(sends[0].custom_tlvs[1].value.as_ref()).unwrap();
        let envelope = stable_channels::stable::parse_stability_signed_envelope(raw).unwrap();
        let payload =
            stable_channels::stable::parse_stability_payment_payload(&envelope.payload).unwrap();
        assert_eq!(payload.channel_id, CHANNEL_ID_HEX);
        assert_eq!(payload.amount_msat, sends[0].amount_msat);
        assert_eq!(
            payload.direction,
            stable_channels::stable::StabilityPaymentDirection::LspToUser
        );
        assert_eq!(payload.expected_usd, 50.0);
        assert_eq!(envelope.signature, "fake-sig");
        assert_eq!(
            fake_drift.sign_calls.lock().unwrap().as_slice(),
            [envelope.payload.as_bytes()]
        );
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
    async fn failed_outbound_stability_payment_restores_backing_and_cooldown() {
        let mut mgr = make_manager();
        let initial = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(50.0),
            None,
            &initial as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let drift = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));

        mgr.run_tick(&drift as &dyn LdkServerCalls, &push, 80_000.0)
            .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 62_500);
        assert!(mgr.stable_channels[0].last_stability_payment > 0);

        let rollback = mgr
            .handle_failed_stability_payment("fake-payment-id")
            .expect("failure should find reversible stability metadata");
        assert!(rollback.applied);
        assert_eq!(mgr.stable_channels[0].backing_sats, 50_000);
        assert_eq!(mgr.stable_channels[0].native_sats, 0);
        assert_eq!(mgr.stable_channels[0].last_stability_payment, 0);
        assert_eq!(
            mgr.db
                .load_channel(USER_CHANNEL_ID_HEX)
                .unwrap()
                .unwrap()
                .backing_sats,
            50_000
        );
        assert!(mgr
            .handle_failed_stability_payment("fake-payment-id")
            .is_none());
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

    fn seed_persisted_save_test_channel(mgr: &mut StableChannelManager, expected: f64) {
        let backing = if expected > 0.0 { 10_000 } else { 0 };
        seed_channel(
            mgr,
            USER_CHANNEL_ID_DECIMAL.parse().unwrap(),
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            expected,
            backing,
            50_000 - backing,
            50_000,
            100_000.0,
        );
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                expected,
                backing,
                50_000 - backing,
                None,
            )
            .unwrap();
    }

    fn save_test_connection(mgr: &StableChannelManager) -> rusqlite::Connection {
        rusqlite::Connection::open(mgr.data_dir().join(stable_channels::db::DB_FILENAME)).unwrap()
    }

    fn assert_saved_books(mgr: &StableChannelManager, expected: f64, backing: u64, native: u64) {
        let sc = &mgr.stable_channels[0];
        // Read through a new connection, as a restarted process would.
        let reopened = Database::open(mgr.data_dir()).unwrap();
        let saved = reopened
            .load_channel(USER_CHANNEL_ID_DECIMAL)
            .unwrap()
            .unwrap();
        assert_eq!(sc.expected_usd.0, expected);
        assert_eq!(sc.backing_sats, backing);
        assert_eq!(sc.native_sats, native);
        assert_eq!(saved.expected_usd, expected);
        assert_eq!(saved.backing_sats, backing);
        assert_eq!(saved.native_sats, native);
        assert_eq!(saved.channel_id, sc.channel_id.to_string());
    }

    fn committed_book_count(conn: &rusqlite::Connection) -> u64 {
        conn.query_row(
            "SELECT COUNT(*) FROM ledger_events
             WHERE event_type = 'CHANNEL_ACCOUNTING_STATE_COMMITTED'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn assert_book_sync(fake: &FakeLdkServer, expected: f64, backing: u64) {
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        let envelope: serde_json::Value =
            serde_json::from_slice(&sends[0].custom_tlvs[0].value).unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(envelope["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["type"], "SYNC_V1");
        assert_eq!(payload["expected_usd"], expected);
        assert_eq!(payload["backing_sats"], backing);
    }

    #[tokio::test]
    async fn backstop_save_failure_preserves_books_and_retries() {
        let mut mgr = make_manager();
        seed_persisted_save_test_channel(&mut mgr, 10.0);
        let conn = save_test_connection(&mgr);
        // Fail after UPDATE channels, to exercise rollback of the full save transaction.
        conn.execute_batch(
            "CREATE TRIGGER reject_book_save BEFORE INSERT ON ledger_events
             WHEN NEW.event_type = 'CHANNEL_ACCOUNTING_STATE_COMMITTED'
             BEGIN SELECT RAISE(ABORT, 'forced ledger failure'); END;",
        )
        .unwrap();
        let before_commits = committed_book_count(&conn);
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )]);
        let push = Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake, &push, 100_000.0).await;
        let before = serde_json::to_string(&mgr.stable_channels[0]).unwrap();
        // Price drift also exercises the requirement to skip settlement after a failed save.
        for _ in 0..3 {
            mgr.run_tick(&fake, &push, 80_000.0).await;
            assert_eq!(
                serde_json::to_string(&mgr.stable_channels[0]).unwrap(),
                before
            );
            assert_saved_books(&mgr, 10.0, 10_000, 40_000);
            assert!(fake.sends.lock().unwrap().is_empty());
            assert!(fake.sign_calls.lock().unwrap().is_empty());
            assert_eq!(committed_book_count(&conn), before_commits);
        }

        conn.execute_batch("DROP TRIGGER reject_book_save").unwrap();
        // Save the original $4 correction calculated at $80k; a later price must not reprice it.
        mgr.run_tick(&fake, &push, 100_000.0).await;
        assert_saved_books(&mgr, 6.0, 5_000, 0);
        assert_eq!(committed_book_count(&conn), before_commits + 1);
        assert_book_sync(&fake, 6.0, 5_000);
        mgr.run_tick(&fake, &push, 100_000.0).await;
        assert_saved_books(&mgr, 6.0, 5_000, 0);
        assert_eq!(committed_book_count(&conn), before_commits + 1);
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn splice_save_failure_preserves_books_and_tick_retries() {
        for failure in [
            "CREATE TRIGGER reject_book_save BEFORE UPDATE ON channels
             BEGIN SELECT RAISE(ABORT, 'forced channel failure'); END;",
            "CREATE TRIGGER reject_book_save BEFORE INSERT ON ledger_events
             WHEN NEW.event_type = 'CHANNEL_ACCOUNTING_STATE_COMMITTED'
             BEGIN SELECT RAISE(ABORT, 'forced ledger failure'); END;",
        ] {
            // Cover stable-spending splice-out, splice-in, and a channel with no USD target.
            for (target, outbound_msat, expected, backing, native) in [
                (10.0, 95_000_000, 5.0, 5_000, 0),
                // Preserve the existing persisted allocation policy on splice-in;
                // recompute_native updates the live native projection separately.
                (10.0, 20_000_000, 10.0, 10_000, 40_000),
                (0.0, 20_000_000, 0.0, 0, 50_000),
            ] {
                let mut mgr = make_manager();
                seed_persisted_save_test_channel(&mut mgr, target);
                let before = serde_json::to_string(&mgr.stable_channels[0]).unwrap();
                let conn = save_test_connection(&mgr);
                conn.execute_batch(failure).unwrap();
                let before_commits = committed_book_count(&conn);
                let new_channel_id = "ab".repeat(32);
                let fake = FakeLdkServer::new(vec![make_channel(
                    &new_channel_id,
                    USER_CHANNEL_ID_DECIMAL,
                    COUNTERPARTY_HEX,
                    100_000,
                    outbound_msat,
                    true,
                )]);
                let push = Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
                    &crate::config::PushConfig::default(),
                    mgr.data_dir(),
                )));
                mgr.handle_channel_ready(
                    new_channel_id.clone(),
                    USER_CHANNEL_ID_DECIMAL.to_owned(),
                    Some("save-failure-splice:0".to_owned()),
                    &fake,
                    100_000.0,
                )
                .await;
                assert_eq!(
                    serde_json::to_string(&mgr.stable_channels[0]).unwrap(),
                    before
                );
                let uid = USER_CHANNEL_ID_DECIMAL.parse().unwrap();
                mgr.startup_sync_pending.insert(uid);
                // The normal tick runner retries startup SYNCs before run_tick. An unresolved
                // splice must block that send too, even though the cached balance is still high.
                mgr.reconcile_if_empty(&fake, 100_000.0).await;
                assert!(fake.sends.lock().unwrap().is_empty());
                mgr.startup_sync_pending.remove(&uid);
                for _ in 0..3 {
                    mgr.run_tick(&fake, &push, 100_000.0).await;
                    assert_eq!(
                        serde_json::to_string(&mgr.stable_channels[0]).unwrap(),
                        before
                    );
                    let old_backing = if target > 0.0 { 10_000 } else { 0 };
                    assert_saved_books(&mgr, target, old_backing, 50_000 - old_backing);
                    assert!(fake.sends.lock().unwrap().is_empty());
                    assert!(fake.sign_calls.lock().unwrap().is_empty());
                    assert_eq!(committed_book_count(&conn), before_commits);
                }

                conn.execute_batch("DROP TRIGGER reject_book_save").unwrap();
                // No second ChannelReady event: the periodic tick must finish the save.
                mgr.run_tick(&fake, &push, 100_000.0).await;
                assert_saved_books(&mgr, expected, backing, native);
                assert_eq!(
                    mgr.stable_channels[0].channel_id.to_string(),
                    new_channel_id
                );
                assert_eq!(
                    mgr.stable_channels[0].native_channel_btc.sats,
                    (100_000 - outbound_msat / 1000) - backing,
                );
                assert_eq!(committed_book_count(&conn), before_commits + 1);
                let sync_count = usize::from(expected < target);
                assert_eq!(fake.sends.lock().unwrap().len(), sync_count);
                if sync_count > 0 {
                    assert_book_sync(&fake, expected, backing);
                }

                mgr.handle_channel_ready(
                    new_channel_id.clone(),
                    USER_CHANNEL_ID_DECIMAL.to_owned(),
                    Some("save-failure-splice:0".to_owned()),
                    &fake,
                    100_000.0,
                )
                .await;
                mgr.run_tick(&fake, &push, 100_000.0).await;
                assert_saved_books(&mgr, expected, backing, native);
                assert_eq!(committed_book_count(&conn), before_commits + 1);
                assert_eq!(fake.sends.lock().unwrap().len(), sync_count);
            }
        }
    }

    #[tokio::test]
    async fn splice_retry_waits_for_price_and_snapshot() {
        let mut mgr = make_manager();
        seed_persisted_save_test_channel(&mut mgr, 10.0);
        let before = serde_json::to_string(&mgr.stable_channels[0]).unwrap();
        let fake = FakeLdkServer::new(vec![]);
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_owned(),
            USER_CHANNEL_ID_DECIMAL.to_owned(),
            Some("delayed-snapshot:0".to_owned()),
            &fake,
            0.0,
        )
        .await;
        let push = Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake, &push, 100_000.0).await;
        assert_eq!(
            serde_json::to_string(&mgr.stable_channels[0]).unwrap(),
            before
        );
        assert!(fake.sends.lock().unwrap().is_empty());

        *fake.channels.lock().unwrap() = vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )];
        mgr.run_tick(&fake, &push, 100_000.0).await;
        assert_saved_books(&mgr, 5.0, 5_000, 0);
        assert_book_sync(&fake, 5.0, 5_000);
        assert!(mgr.pending_splices.is_empty());
    }

    struct BufferedTestSource(tokio::sync::mpsc::Receiver<crate::event_loop::EventItem>);

    #[async_trait]
    impl crate::event_loop::EventSource for BufferedTestSource {
        async fn next_event(&mut self) -> Option<crate::event_loop::EventItem> {
            self.0.recv().await
        }
    }

    async fn failed_correction_before_forward(splice: bool, hold_database_failure: bool) {
        let mut mgr = make_manager();
        seed_persisted_save_test_channel(&mut mgr, 10.0);
        let conn = save_test_connection(&mgr);
        let commits_before = committed_book_count(&conn);
        conn.execute_batch(
            "CREATE TRIGGER reject_book_save BEFORE INSERT ON ledger_events
             WHEN NEW.event_type = 'CHANNEL_ACCOUNTING_STATE_COMMITTED'
             BEGIN SELECT RAISE(ABORT, 'forced ledger failure'); END;",
        )
        .unwrap();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )]);
        let push = Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        if splice {
            mgr.handle_channel_ready(
                CHANNEL_ID_HEX.to_owned(),
                USER_CHANNEL_ID_DECIMAL.to_owned(),
                Some("splice-before-forward:0".to_owned()),
                &fake,
                100_000.0,
            )
            .await;
        } else {
            mgr.run_tick(&fake, &push, 100_000.0).await;
            mgr.run_tick(&fake, &push, 100_000.0).await;
        }
        assert_saved_books(&mgr, 10.0, 10_000, 40_000);
        assert!(fake.sends.lock().unwrap().is_empty());
        if !hold_database_failure {
            conn.execute_batch("DROP TRIGGER reject_book_save").unwrap();
        }

        // A new successful payment is already reflected in live capacity. Keep its handler
        // behind the original correction, using the same lock as live event dispatch.
        *fake.channels.lock().unwrap() = vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            97_000_000,
            true,
        )];
        let shared = tokio::sync::Mutex::new(mgr);
        let next_event = async {
            let mut mgr = StableChannelManager::lock_for_event(&shared, &fake).await;
            mgr.handle_payment_forwarded(
                USER_CHANNEL_ID_DECIMAL.to_owned(),
                Some("next-ucid".to_owned()),
                CHANNEL_ID_HEX.to_owned(),
                "next-channel".to_owned(),
                COUNTERPARTY_HEX.to_owned(),
                "next-node".to_owned(),
                2_000_000,
                0,
                None,
                &fake,
                100_000.0,
            )
            .await;
        };
        tokio::pin!(next_event);
        let mut buffered_burst = None;
        if hold_database_failure {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(50), &mut next_event,)
                    .await
                    .is_err(),
                "the event must wait, not complete or be discarded"
            );
            {
                let mut mgr = shared.lock().await;
                let before = serde_json::to_string(&mgr.stable_channels[0]).unwrap();
                // Reconnect hydration and a settings edit must not replace the saved proposal.
                mgr.reconcile_from_grpc(&fake, 80_000.0).await;
                let edit = mgr
                    .edit_stable_channel(CHANNEL_ID_HEX, Some(9.0), None, &fake, 100_000.0)
                    .await;
                assert!(!edit.ok);
                assert_eq!(
                    serde_json::to_string(&mgr.stable_channels[0]).unwrap(),
                    before
                );
                assert_saved_books(&mgr, 10.0, 10_000, 40_000);
                assert!(fake.sends.lock().unwrap().is_empty());
                assert_eq!(committed_book_count(&conn), commits_before);
            }
            // Model the server's bounded subscriber queue. More than its 1,024-event
            // broadcast capacity must drain locally even while the database is still failing.
            let (sender, source) = tokio::sync::mpsc::channel(64);
            let (mut reader, receiver) =
                crate::event_loop::buffer_events(BufferedTestSource(source));
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                use ldk_server_client::ldk_server_grpc::events::{
                    event_envelope::Event, ChannelStateChanged, EventEnvelope,
                };
                for n in 0..2048 {
                    sender
                        .send(Ok(EventEnvelope {
                            event: Some(Event::ChannelStateChanged(ChannelStateChanged {
                                user_channel_id: n.to_string(),
                                ..Default::default()
                            })),
                        }))
                        .await
                        .unwrap();
                }
                drop(sender);
                reader.join_next().await.unwrap().unwrap();
            })
            .await
            .expect("the subscription reader must keep draining while accounting waits");
            assert_eq!(receiver.len(), 2048);
            buffered_burst = Some(receiver);
            conn.execute_batch("DROP TRIGGER reject_book_save").unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), &mut next_event)
            .await
            .expect("the retained event should resume after the database recovers");
        if let Some(mut receiver) = buffered_burst {
            for n in 0..2048 {
                let event = receiver.recv().await.unwrap().unwrap();
                let Some(ldk_server_client::ldk_server_grpc::events::event_envelope::Event::ChannelStateChanged(channel)) = event.event else {
                    panic!("unexpected buffered event");
                };
                assert_eq!(
                    channel.user_channel_id,
                    n.to_string(),
                    "events must retain their order"
                );
            }
            assert!(receiver.recv().await.is_none());
        }
        let mut mgr = shared.lock().await;
        mgr.run_tick(&fake, &push, 100_000.0).await;
        assert_saved_books(&mgr, 3.0, 3_000, 0);
        assert_eq!(committed_book_count(&conn), commits_before + 2);
        assert!(mgr.pending_book_updates.is_empty());
        assert!(mgr.pending_splices.is_empty());
        let sends = fake.sends.lock().unwrap();
        let payloads: Vec<serde_json::Value> = sends
            .iter()
            .map(|send| {
                let envelope: serde_json::Value =
                    serde_json::from_slice(&send.custom_tlvs[0].value).unwrap();
                serde_json::from_str(envelope["payload"].as_str().unwrap()).unwrap()
            })
            .collect();
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0]["expected_usd"], 5.0);
        assert_eq!(payloads[0]["backing_sats"], 5_000);
        assert_eq!(payloads[1]["expected_usd"], 3.0);
        assert_eq!(payloads[1]["backing_sats"], 3_000);
    }

    #[tokio::test]
    async fn failed_splice_is_committed_before_a_later_forward() {
        failed_correction_before_forward(true, false).await;
    }

    #[tokio::test]
    async fn splice_retry_preserves_the_correction_when_live_balance_grows() {
        let mut mgr = make_manager();
        seed_persisted_save_test_channel(&mut mgr, 10.0);
        let conn = save_test_connection(&mgr);
        conn.execute_batch(
            "CREATE TRIGGER reject_book_save BEFORE INSERT ON ledger_events
             WHEN NEW.event_type = 'CHANNEL_ACCOUNTING_STATE_COMMITTED'
             BEGIN SELECT RAISE(ABORT, 'forced ledger failure'); END;",
        )
        .unwrap();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )]);
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_owned(),
            USER_CHANNEL_ID_DECIMAL.to_owned(),
            Some("splice-before-deposit:0".to_owned()),
            &fake,
            100_000.0,
        )
        .await;
        conn.execute_batch("DROP TRIGGER reject_book_save").unwrap();
        // Later BTC arriving and a new price must not erase or reprice the failed deduction.
        *fake.channels.lock().unwrap() = vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            85_000_000,
            true,
        )];
        let push = Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake, &push, 80_000.0).await;
        assert_saved_books(&mgr, 5.0, 5_000, 0);
        assert_book_sync(&fake, 5.0, 5_000);
        assert!(mgr.pending_book_updates.is_empty());
        assert!(mgr.pending_splices.is_empty());
    }

    #[tokio::test]
    async fn failed_backstop_is_committed_before_a_later_forward() {
        failed_correction_before_forward(false, false).await;
    }

    #[tokio::test]
    async fn failed_correction_holds_the_event_until_database_recovery() {
        failed_correction_before_forward(true, true).await;
        failed_correction_before_forward(false, true).await;
    }

    #[tokio::test]
    async fn splice_retry_is_removed_when_channel_closes() {
        let mut mgr = make_manager();
        seed_persisted_save_test_channel(&mut mgr, 10.0);
        let fake = FakeLdkServer::new(vec![]);
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_owned(),
            USER_CHANNEL_ID_DECIMAL.to_owned(),
            Some("closed-splice:0".to_owned()),
            &fake,
            100_000.0,
        )
        .await;
        assert_eq!(mgr.pending_splices.len(), 1);
        let shared = tokio::sync::Mutex::new(mgr);
        let mut mgr = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            StableChannelManager::lock_for_event(&shared, &fake),
        )
        .await
        .expect("a missing snapshot must not block the channel's close event");
        mgr.handle_channel_closed(
            CHANNEL_ID_HEX.to_owned(),
            USER_CHANNEL_ID_DECIMAL.to_owned(),
            None,
            None,
            0,
            None,
        );
        assert!(mgr.pending_splices.is_empty());
        assert!(mgr.stable_channels.is_empty());
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
        let mut mgr = make_manager();
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

    async fn sync_delivery_fixture() -> (tempfile::TempDir, StableChannelManager, FakeLdkServer) {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        db.save_channel(CHANNEL_ID_HEX, "7", 25.0, 31_250, 18_750, None).unwrap();
        let mut mgr = StableChannelManager::new(db, dir.path().to_path_buf());
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, "7", COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.reconcile_from_grpc(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
        (dir, mgr, fake)
    }

    async fn dispatch_sync_outcome(
        mgr: &mut StableChannelManager,
        fake: &FakeLdkServer,
        payment_id: &str,
        succeeded: bool,
    ) -> crate::event_loop::DispatchOutcome {
        use ldk_server_client::ldk_server_grpc::events::{
            event_envelope::Event, PaymentFailed, PaymentSuccessful,
        };
        let payment = Some(GrpcPayment {
            payment_id: payment_id.into(),
            amount_msat: Some(1),
            direction: 1,
            status: if succeeded { PaymentStatus::Succeeded } else { PaymentStatus::Failed } as i32,
            ..Default::default()
        });
        let event = if succeeded {
            Event::PaymentSuccessful(PaymentSuccessful {
                payment_id: payment_id.into(),
                payment,
                ..Default::default()
            })
        } else {
            Event::PaymentFailed(PaymentFailed { payment_id: payment_id.into(), payment, reason: None })
        };
        let db = mgr.db.clone();
        crate::event_loop::dispatch_event(Some(event), mgr, &db, fake, 80_000.0).await
    }

    fn sent_sync_payload(fake: &FakeLdkServer, index: usize) -> serde_json::Value {
        let sends = fake.sends.lock().unwrap();
        let raw = std::str::from_utf8(sends[index].custom_tlvs[0].value.as_ref()).unwrap();
        let envelope = crate::messages::parse_envelope(raw).unwrap();
        assert_eq!(envelope.signature, "fake-sig");
        serde_json::from_str(&envelope.payload).unwrap()
    }

    #[tokio::test]
    async fn sync_delivery_failure_retries_committed_books_until_delivered() {
        let (_dir, mut mgr, fake) = sync_delivery_fixture().await;
        assert_eq!(mgr.db.list_pending_settlements().unwrap().len(), 1);
        assert_eq!(dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id", false).await,
            crate::event_loop::DispatchOutcome::Continue);
        assert_eq!(mgr.db.list_failed_sync_channels().unwrap(), vec!["7"]);

        mgr.db.save_channel(CHANNEL_ID_HEX, "7", 12.0, 15_000, 35_000, None).unwrap();
        // Simulate a later in-memory correction whose database save failed (#321). Neither
        // this state nor the original failed payload may become the retry's balance.
        mgr.stable_channels[0].expected_usd = USD(99.0);
        mgr.stable_channels[0].backing_sats = 90_000;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        let retry = sent_sync_payload(&fake, 1);
        assert_eq!(retry["type"], "SYNC_V1");
        assert_eq!(retry["expected_usd"], 12.0);
        assert_eq!(retry["backing_sats"], 15_000);
        assert_eq!(retry["sync_version"], 2);
        assert_eq!(mgr.db.list_pending_settlements().unwrap(),
            vec![("fake-payment-id-2".into(), "sync".into())]);

        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2, "pending attempt must not be duplicated");
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-2", true).await;
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id", false).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2, "delivered correction stops retries");
        assert!(mgr.db.list_pending_settlements().unwrap().is_empty());
        assert!(mgr.db.list_failed_sync_channels().unwrap().is_empty());
        let books = mgr.db.load_channel("7").unwrap().unwrap();
        assert_eq!((books.expected_usd, books.backing_sats, books.native_sats), (12.0, 15_000, 35_000));
    }

    #[tokio::test]
    async fn sync_delivery_older_outcomes_cannot_override_newer_attempts() {
        let (_dir, mut mgr, fake) = sync_delivery_fixture().await;
        assert!(mgr.send_sync_message(&fake, 7, CHANNEL_ID_HEX, 25.0, 31_250, COUNTERPARTY_HEX).await);
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id", false).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2, "newer pending SYNC supersedes old failure");

        assert!(mgr.send_sync_message(&fake, 7, CHANNEL_ID_HEX, 25.0, 31_250, COUNTERPARTY_HEX).await);
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-3", false).await;
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-2", true).await;
        assert_eq!(mgr.db.list_failed_sync_channels().unwrap(), vec!["7"],
            "older success must not clear the latest failed correction");
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 4);
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-4", true).await;
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-3", false).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn sync_delivery_poll_recovers_missed_failure_and_success_events() {
        let (_dir, mut mgr, fake) = sync_delivery_fixture().await;
        fake.payments.lock().unwrap().push(GrpcPayment {
            payment_id: "fake-payment-id".into(), status: PaymentStatus::Pending as i32,
            direction: 1, amount_msat: Some(1), ..Default::default()
        });
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
        fake.payments.lock().unwrap()[0].status = PaymentStatus::Failed as i32;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2);
        fake.payments.lock().unwrap().push(GrpcPayment {
            payment_id: "fake-payment-id-2".into(), status: PaymentStatus::Succeeded as i32,
            direction: 1, amount_msat: Some(1), ..Default::default()
        });
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert!(mgr.db.list_pending_settlements().unwrap().is_empty());
        assert!(mgr.db.list_failed_sync_channels().unwrap().is_empty());
        assert_eq!(fake.sends.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn sync_delivery_failure_survives_restart_and_reconnect_backfill() {
        let (dir, mut mgr, fake) = sync_delivery_fixture().await;
        fake.payments.lock().unwrap().push(GrpcPayment {
            payment_id: "fake-payment-id".into(), status: PaymentStatus::Failed as i32,
            direction: 1, amount_msat: Some(1), ..Default::default()
        });
        let counts = crate::backfill::reconcile_event_history(&fake, &mgr.db, None).await;
        assert!(counts.settlement_outcomes_safe);
        assert_eq!(mgr.db.list_failed_sync_channels().unwrap(), vec!["7"]);
        drop(mgr);
        let db = Arc::new(Database::open(dir.path()).unwrap());
        assert_eq!(db.list_failed_sync_channels().unwrap(), vec!["7"]);
        mgr = StableChannelManager::new(db, dir.path().to_path_buf());
        mgr.reconcile_from_grpc(&fake, 80_000.0).await;
        assert_eq!(sent_sync_payload(&fake, 1)["sync_version"], 2);
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-2", true).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn sync_delivery_db_failure_keeps_outcome_retryable() {
        let (dir, mut mgr, fake) = sync_delivery_fixture().await;
        let conn = rusqlite::Connection::open(dir.path().join(stable_channels::db::DB_FILENAME)).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_sync_outcome BEFORE UPDATE OF outcome ON settlement_payments
            BEGIN SELECT RAISE(FAIL, 'injected outcome failure'); END;").unwrap();
        assert_eq!(dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id", false).await,
            crate::event_loop::DispatchOutcome::Reconnect);
        assert_eq!(mgr.db.list_pending_settlements().unwrap().len(), 1);
        assert!(mgr.db.list_failed_sync_channels().unwrap().is_empty());
        conn.execute_batch("DROP TRIGGER fail_sync_outcome;").unwrap();
        fake.payments.lock().unwrap().push(GrpcPayment {
            payment_id: "fake-payment-id".into(), status: PaymentStatus::Failed as i32,
            direction: 1, amount_msat: Some(1), ..Default::default()
        });
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn sync_delivery_failed_attempt_registration_is_retried() {
        let (dir, mut mgr, fake) = sync_delivery_fixture().await;
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id", false).await;
        let conn = rusqlite::Connection::open(dir.path().join(stable_channels::db::DB_FILENAME)).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_sync_record BEFORE INSERT ON settlement_payments
            WHEN NEW.kind = 'sync' BEGIN SELECT RAISE(FAIL, 'injected record failure'); END;").unwrap();
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2);
        assert!(mgr.startup_sync_pending.contains(&7));
        assert_eq!(mgr.db.list_failed_sync_channels().unwrap(), vec!["7"]);
        conn.execute_batch("DROP TRIGGER fail_sync_record;").unwrap();
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(sent_sync_payload(&fake, 2)["sync_version"], 3);
        assert!(!mgr.startup_sync_pending.contains(&7));
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-3", true).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn sync_delivery_does_not_retry_closed_channels_or_other_payment_kinds() {
        let (_dir, mut mgr, fake) = sync_delivery_fixture().await;
        mgr.db.record_settlement_with_channel("trade-response", "trade", "7").unwrap();
        mgr.db.record_settlement_with_channel("stability", "stability", "7").unwrap();
        dispatch_sync_outcome(&mut mgr, &fake, "trade-response", false).await;
        dispatch_sync_outcome(&mut mgr, &fake, "stability", false).await;
        dispatch_sync_outcome(&mut mgr, &fake, "unrelated", false).await;
        assert!(mgr.db.list_failed_sync_channels().unwrap().is_empty());
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id", false).await;
        mgr.handle_channel_closed(CHANNEL_ID_HEX.into(), "7".into(), None, None, 0, None);
        assert!(mgr.db.list_failed_sync_channels().unwrap().is_empty());
        fake.channels.lock().unwrap().clear();
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
    }

    #[test]
    fn sync_retry_delay_is_immediate_once_then_doubles_to_a_cap() {
        assert_eq!(sync_retry_delay_secs(0), 0);
        assert_eq!(sync_retry_delay_secs(1), 0);
        assert_eq!(sync_retry_delay_secs(2), SYNC_RETRY_BACKOFF_BASE_SECS);
        assert_eq!(sync_retry_delay_secs(3), 2 * SYNC_RETRY_BACKOFF_BASE_SECS);
        assert_eq!(sync_retry_delay_secs(7), 32 * SYNC_RETRY_BACKOFF_BASE_SECS);
        assert_eq!(sync_retry_delay_secs(8), SYNC_RETRY_BACKOFF_MAX_SECS);
        assert_eq!(sync_retry_delay_secs(64), SYNC_RETRY_BACKOFF_MAX_SECS);
        assert_eq!(sync_retry_delay_secs(u64::MAX), SYNC_RETRY_BACKOFF_MAX_SECS);
    }

    fn age_sync_attempts(dir: &tempfile::TempDir, secs: i64) {
        let conn = rusqlite::Connection::open(dir.path().join(stable_channels::db::DB_FILENAME)).unwrap();
        conn.execute(
            "UPDATE settlement_payments SET recorded_at = recorded_at - ?1 WHERE kind = 'sync'",
            rusqlite::params![secs],
        ).unwrap();
    }

    #[tokio::test]
    async fn sync_retry_waits_until_the_channel_is_usable() {
        let (_dir, mut mgr, fake) = sync_delivery_fixture().await;
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id", false).await;
        fake.channels.lock().unwrap()[0].is_usable = false;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 1, "no attempt while the peer is offline");
        assert_eq!(mgr.db.get_sync_version("7").unwrap(), Some(1), "no version consumed offline");
        assert!(mgr.startup_sync_pending.contains(&7), "the obligation stays queued");
        fake.channels.lock().unwrap()[0].is_usable = true;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2);
        assert_eq!(sent_sync_payload(&fake, 1)["sync_version"], 2);
    }

    #[tokio::test]
    async fn sync_retry_backs_off_after_repeated_delivery_failures() {
        let (dir, mut mgr, fake) = sync_delivery_fixture().await;
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id", false).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2, "the first retry is immediate");
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-2", false).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2, "the second retry waits one backoff step");
        age_sync_attempts(&dir, SYNC_RETRY_BACKOFF_BASE_SECS as i64);
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 3);
        dispatch_sync_outcome(&mut mgr, &fake, "fake-payment-id-3", false).await;
        age_sync_attempts(&dir, SYNC_RETRY_BACKOFF_BASE_SECS as i64);
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 3, "the third retry waits twice the base");
        age_sync_attempts(&dir, SYNC_RETRY_BACKOFF_BASE_SECS as i64);
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 4);
        assert_eq!(sent_sync_payload(&fake, 3)["sync_version"], 4);
    }

    #[tokio::test]
    async fn sync_retry_stops_at_the_attempt_cap_until_a_sync_is_delivered() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        stable_channels::audit::enable_test_capture();
        let (dir, mut mgr, fake) = sync_delivery_fixture().await;
        let mut newest = "fake-payment-id".to_string();
        for attempt in 1..SYNC_RETRY_MAX_ATTEMPTS {
            dispatch_sync_outcome(&mut mgr, &fake, &newest, false).await;
            age_sync_attempts(&dir, SYNC_RETRY_BACKOFF_MAX_SECS as i64);
            mgr.reconcile_if_empty(&fake, 80_000.0).await;
            assert_eq!(fake.sends.lock().unwrap().len() as u64, attempt + 1, "attempt {attempt} retried");
            newest = format!("fake-payment-id-{}", attempt + 1);
        }
        dispatch_sync_outcome(&mut mgr, &fake, &newest, false).await;
        age_sync_attempts(&dir, SYNC_RETRY_BACKOFF_MAX_SECS as i64);
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len() as u64, SYNC_RETRY_MAX_ATTEMPTS, "no attempt past the cap");
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let exhausted: Vec<_> = events.iter().filter(|(event, _)| event == "SYNC_RETRY_EXHAUSTED").collect();
        assert_eq!(exhausted.len(), 1, "exhaustion is audited once, not every tick");
        assert_eq!(exhausted[0].1["user_channel_id"], "7");

        assert!(mgr.send_sync_message(&fake, 7, CHANNEL_ID_HEX, 25.0, 31_250, COUNTERPARTY_HEX).await);
        let delivered = format!("fake-payment-id-{}", SYNC_RETRY_MAX_ATTEMPTS + 1);
        dispatch_sync_outcome(&mut mgr, &fake, &delivered, true).await;
        assert!(mgr.send_sync_message(&fake, 7, CHANNEL_ID_HEX, 25.0, 31_250, COUNTERPARTY_HEX).await);
        let failed = format!("fake-payment-id-{}", SYNC_RETRY_MAX_ATTEMPTS + 2);
        dispatch_sync_outcome(&mut mgr, &fake, &failed, false).await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len() as u64, SYNC_RETRY_MAX_ATTEMPTS + 3,
            "retries resume once a SYNC is delivered");
    }

    #[tokio::test]
    async fn sync_pending_attempt_is_abandoned_after_the_timeout() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        stable_channels::audit::enable_test_capture();
        let (dir, mut mgr, fake) = sync_delivery_fixture().await;
        fake.payments.lock().unwrap().push(GrpcPayment {
            payment_id: "fake-payment-id".into(), status: PaymentStatus::Pending as i32,
            direction: 1, amount_msat: Some(1), ..Default::default()
        });
        age_sync_attempts(&dir, SYNC_PENDING_TIMEOUT_SECS as i64 - 10);
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 1, "a young pending attempt is still awaited");
        age_sync_attempts(&dir, 20);
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2, "an abandoned attempt is replaced");
        assert_eq!(mgr.db.list_pending_settlements().unwrap(),
            vec![("fake-payment-id-2".into(), "sync".into())]);
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert!(events.iter().any(|(event, data)|
            event == "SYNC_PENDING_ABANDONED" && data["payment_id"] == "fake-payment-id"));
    }

    #[tokio::test]
    async fn sync_pending_attempt_unknown_to_ldk_is_abandoned_after_the_timeout() {
        let (dir, mut mgr, fake) = sync_delivery_fixture().await;
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 1, "an unknown young attempt is still awaited");
        age_sync_attempts(&dir, SYNC_PENDING_TIMEOUT_SECS as i64 + 10);
        mgr.reconcile_if_empty(&fake, 80_000.0).await;
        assert_eq!(fake.sends.lock().unwrap().len(), 2);
        assert!(mgr.db.list_failed_sync_channels().unwrap().is_empty(), "the replacement supersedes it");
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

    fn correlated_trade_envelope(
        channel_id: &str,
        user_channel_id: &str,
        trade_id: &str,
        expected_usd: f64,
        quote_price: f64,
    ) -> String {
        let payload = serde_json::json!({
            "type": "TRADE_V1",
            "channel_id": channel_id,
            "user_channel_id": user_channel_id,
            "trade_id": trade_id,
            "expected_usd": expected_usd,
            "quote_price": quote_price,
            "ts": test_unix_now(),
        })
        .to_string();
        serde_json::json!({ "payload": payload, "signature": "wallet-sig" }).to_string()
    }

    fn correlated_trade_envelope_at(
        trade_id: &str,
        expected_usd: f64,
        quote_price: Option<f64>,
        ts: u64,
    ) -> String {
        let payload = serde_json::json!({
            "type": "TRADE_V1",
            "channel_id": CHANNEL_ID_HEX,
            "user_channel_id": USER_CHANNEL_ID_DECIMAL,
            "trade_id": trade_id,
            "expected_usd": expected_usd,
            "quote_price": quote_price,
            "ts": ts,
        })
        .to_string();
        serde_json::json!({ "payload": payload, "signature": "wallet-sig" }).to_string()
    }

    fn correlated_rejection_context(
        expected_usd: f64,
        backing_sats: u64,
        receiver_sats: u64,
    ) -> (StableChannelManager, FakeLdkServer) {
        let mut manager = make_manager();
        let channel_value_sats = receiver_sats.saturating_add(100_000);
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            channel_value_sats,
            100_000_000,
            true,
        )]);
        let uid = USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap();
        seed_channel(
            &mut manager,
            uid,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            expected_usd,
            backing_sats,
            receiver_sats.saturating_sub(backing_sats),
            receiver_sats,
            100_000.0,
        );
        manager
            .db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                expected_usd,
                backing_sats,
                receiver_sats.saturating_sub(backing_sats),
                None,
            )
            .unwrap();
        (manager, fake)
    }

    #[tokio::test]
    async fn stabilization_cap_correlated_shadow_and_enforcement() {
        let envelope =
            correlated_trade_envelope_at(&"5".repeat(64), 99.5, Some(100_000.0), test_unix_now());
        let fee = expected_trade_fee_msat(50.0, 99.5, 100_000.0).unwrap();
        let (mut shadow, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        shadow
            .handle_trade_payment(
                &envelope,
                Some(&"6".repeat(64)),
                Some(fee),
                &fake,
                100_000.0,
            )
            .await;
        assert_eq!(
            shadow.stable_channels[0].expected_usd.0, 99.5,
            "shadow must not reject old clients"
        );

        let (mut enforcing, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        enforcing.enforce_max_stabilization = true;
        assert_eq!(
            correlated_rejection_reason(
                &mut enforcing,
                &fake,
                &envelope,
                &"6".repeat(64),
                fee,
                100_000.0
            )
            .await,
            TradeRejectionReason::InsufficientCapacity
        );
    }

    #[tokio::test]
    async fn stabilization_cap_legacy_edit_and_reduction_paths() {
        for enforced in [false, true] {
            let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
            manager.enforce_max_stabilization = enforced;
            let envelope = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 99.5);
            handle_trade_with_valid_fee(&mut manager, &envelope, &fake, 100_000.0).await;
            assert_eq!(
                manager.stable_channels[0].expected_usd.0,
                if enforced { 50.0 } else { 99.5 }
            );

            let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
            manager.enforce_max_stabilization = enforced;
            let result = manager
                .edit_stable_channel(CHANNEL_ID_HEX, Some(99.5), None, &fake, 100_000.0)
                .await;
            assert_eq!(result.ok, !enforced);
        }
        let (mut manager, fake) = correlated_rejection_context(100.0, 100_000, 100_000);
        manager.enforce_max_stabilization = true;
        // A reduction can still be above the entry limit and must not be blocked.
        let envelope = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 99.5);
        handle_trade_with_valid_fee(&mut manager, &envelope, &fake, 100_000.0).await;
        assert_eq!(manager.stable_channels[0].expected_usd.0, 99.5);
    }

    #[test]
    fn stabilization_cap_uses_post_fee_user_capacity_and_audits_both_modes() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        stable_channels::audit::enable_test_capture();
        let mut channel = make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            500_000,
            100_000_000,
            true,
        );
        // Deliberately unrelated total capacity, asymmetric reserves and a funder fee.
        channel.inbound_capacity_msat = 99_000_000; // already paid a 1k-sat fee
        channel.unspendable_punishment_reserve = Some(20_000);
        channel.counterparty_unspendable_punishment_reserve = 5_000;
        assert!(!max_stabilization_rejected(
            true,
            &channel,
            50.0,
            98.01,
            98_010,
            "boundary-test"
        ));
        assert!(max_stabilization_rejected(
            true,
            &channel,
            50.0,
            98.011,
            98_011,
            "boundary-test"
        ));
        assert!(!max_stabilization_rejected(
            false,
            &channel,
            50.0,
            99.0,
            99_000,
            "shadow-test"
        ));
        assert!(max_stabilization_rejected(
            true,
            &channel,
            50.0,
            99.0,
            99_000,
            "enforcing-test"
        ));
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert!(events
            .iter()
            .any(|(name, value)| name == "MAX_STABILIZATION_REJECTED"
                && value["source"] == "shadow-test"
                && value["enforced"] == false));
        assert!(events
            .iter()
            .any(|(name, value)| name == "MAX_STABILIZATION_REJECTED"
                && value["source"] == "enforcing-test"
                && value["enforced"] == true));
    }

    async fn correlated_rejection_reason(
        manager: &mut StableChannelManager,
        fake: &FakeLdkServer,
        envelope: &str,
        payment_id: &str,
        amount_msat: u64,
        lsp_price: f64,
    ) -> TradeRejectionReason {
        let before = manager.stable_channels.first().map(|channel| {
            (channel.expected_usd.0, channel.backing_sats, channel.native_sats)
        });
        let durable_before = manager
            .db
            .load_channel(USER_CHANNEL_ID_DECIMAL)
            .unwrap()
            .map(|channel| {
                (
                    channel.expected_usd,
                    channel.backing_sats,
                    channel.native_sats,
                )
            });
        manager
            .handle_trade_payment(
                envelope,
                Some(payment_id),
                Some(amount_msat),
                fake,
                lsp_price,
            )
            .await;
        let after = manager.stable_channels.first().map(|channel| {
            (channel.expected_usd.0, channel.backing_sats, channel.native_sats)
        });
        assert_eq!(after, before, "rejection must not mutate in-memory allocation");
        assert_eq!(
            manager
                .db
                .load_channel(USER_CHANNEL_ID_DECIMAL)
                .unwrap()
                .map(|channel| {
                    (
                        channel.expected_usd,
                        channel.backing_sats,
                        channel.native_sats,
                    )
                }),
            durable_before,
            "rejection must not mutate durable channel allocation",
        );
        StableChannelManager::retry_pending_trade_responses(manager.db.as_ref(), fake).await;
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].amount_msat, 1);
        let raw = std::str::from_utf8(sends[0].custom_tlvs[0].value.as_ref()).unwrap();
        let response = crate::messages::parse_envelope(raw).unwrap();
        serde_json::from_str::<stable_channels::trade::TradeRejectedV1>(&response.payload)
            .unwrap()
            .reason_code
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
            stable_channels::stable::normalize_trade_expected_usd(payload.expected_usd),
            payload.quote_price.unwrap_or(lsp_price),
        )
        .unwrap();
        mgr.handle_trade_message(envelope, None, Some(fee_msat), ldk, lsp_price)
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
        assert_eq!(expected_trade_fee_msat(1.0, 0.0, 1_000_000.0), Some(1_000));
        assert_eq!(expected_trade_fee_msat(1.0, 0.1, 1_000_000.0), Some(1));
        assert_eq!(trade_fee_tolerance_msat(114_000, true), 1_000);

        // At this boundary the wallet's original gross fee floors to 113 sats, while recovering
        // the gross sell amount from its signed net target produces 114 sats.
        let gross_sell_usd = 7.41;
        let fee_usd = gross_sell_usd * stable_channels::constants::STABLE_CHANNEL_TRADE_FEE_RATE;
        let signed_net_target = gross_sell_usd - fee_usd;
        let wallet_fee_msat = ((fee_usd / 65_000.0 * 100_000_000.0) as u64) * 1000;
        let reconstructed = expected_trade_fee_msat(0.0, signed_net_target, 65_000.0).unwrap();
        assert_eq!(wallet_fee_msat, 113_000);
        assert_eq!(reconstructed, 114_000);
        assert!(wallet_fee_msat.abs_diff(reconstructed) <= trade_fee_tolerance_msat(reconstructed, true));
    }

    #[tokio::test]
    async fn correlated_acceptance_is_atomic_idempotent_and_contains_request_hash() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            300_000,
            100_000,
            true,
        )]);
        let uid = USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap();
        seed_channel(
            &mut mgr,
            uid,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            50.0,
            50_000,
            50_000,
            100_000,
            100_000.0,
        );
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                50.0,
                50_000,
                50_000,
                None,
            )
            .unwrap();
        let trade_id = "b".repeat(64);
        let payment_id = "c".repeat(64);
        let envelope = correlated_trade_envelope(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            &trade_id,
            60.0,
            100_000.0,
        );
        let signed = crate::messages::parse_envelope(&envelope).unwrap();
        let request_hash = stable_channels::trade::request_hash(signed.payload.as_bytes());
        let fee = expected_trade_fee_msat(50.0, 60.0, 100_000.0).unwrap();

        mgr.handle_trade_payment(
            &envelope,
            Some(&payment_id),
            Some(fee),
            &fake,
            100_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels[0].expected_usd.0, 60.0);
        assert_eq!(mgr.db.get_sync_version(USER_CHANNEL_ID_DECIMAL).unwrap(), Some(1));
        assert!(mgr
            .db
            .trade_decision_by_payment(&payment_id)
            .unwrap()
            .is_some());

        mgr.handle_trade_payment(
            &envelope,
            Some(&payment_id),
            Some(fee),
            &fake,
            100_000.0,
        )
        .await;
        assert_eq!(mgr.db.get_sync_version(USER_CHANNEL_ID_DECIMAL).unwrap(), Some(1));
        StableChannelManager::retry_pending_trade_responses(mgr.db.as_ref(), &fake).await;
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].amount_msat, 1);
        let raw = std::str::from_utf8(sends[0].custom_tlvs[0].value.as_ref()).unwrap();
        let response = crate::messages::parse_envelope(raw).unwrap();
        let value: serde_json::Value = serde_json::from_str(&response.payload).unwrap();
        assert_eq!(value["type"], "SYNC_V1");
        assert_eq!(value["trade_id"], trade_id);
        assert_eq!(value["trade_payment_id"], payment_id);
        assert_eq!(value["request_hash"], request_hash);
        assert_eq!(value["expected_usd"], 60.0);
    }

    #[tokio::test]
    async fn trade_response_retry_reconciles_uncertain_send_from_ldk_state() {
        let manager = make_manager();
        let fake = FakeLdkServer::new(vec![]);
        let now = StableChannelManager::unix_time_secs();
        manager
            .db
            .persist_trade_rejection(
                &"7".repeat(64),
                &"8".repeat(64),
                &"9".repeat(64),
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                COUNTERPARTY_HEX,
                TradeRejectionReason::InternalFailure.as_str(),
                now,
                "signed-rejection",
            )
            .unwrap();

        StableChannelManager::retry_pending_trade_responses(manager.db.as_ref(), &fake).await;
        assert_eq!(
            manager
                .db
                .in_flight_trade_response_payment_ids()
                .unwrap(),
            vec!["fake-payment-id".to_string()]
        );
        fake.payments.lock().unwrap().push(GrpcPayment {
            payment_id: "fake-payment-id".to_string(),
            status: PaymentStatus::Succeeded as i32,
            ..Default::default()
        });
        StableChannelManager::retry_pending_trade_responses(manager.db.as_ref(), &fake).await;
        assert!(manager
            .db
            .in_flight_trade_response_payment_ids()
            .unwrap()
            .is_empty());
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn authenticated_correlated_rejection_leaves_allocation_unchanged() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            300_000,
            100_000,
            true,
        )]);
        let uid = USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap();
        seed_channel(
            &mut mgr,
            uid,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            50.0,
            50_000,
            50_000,
            100_000,
            100_000.0,
        );
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                50.0,
                50_000,
                50_000,
                None,
            )
            .unwrap();
        let trade_id = "d".repeat(64);
        let payment_id = "e".repeat(64);
        let envelope = correlated_trade_envelope(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            &trade_id,
            60.0,
            100_000.0,
        );
        mgr.handle_trade_payment(&envelope, Some(&payment_id), Some(1), &fake, 100_000.0)
            .await;
        assert_eq!(mgr.stable_channels[0].expected_usd.0, 50.0);
        assert_eq!(mgr.db.load_channel(USER_CHANNEL_ID_DECIMAL).unwrap().unwrap().expected_usd, 50.0);
        StableChannelManager::retry_pending_trade_responses(mgr.db.as_ref(), &fake).await;
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        let raw = std::str::from_utf8(sends[0].custom_tlvs[0].value.as_ref()).unwrap();
        let response = crate::messages::parse_envelope(raw).unwrap();
        let rejection: stable_channels::trade::TradeRejectedV1 =
            serde_json::from_str(&response.payload).unwrap();
        assert_eq!(rejection.reason_code, TradeRejectionReason::InvalidFee);
        assert_eq!(rejection.trade_payment_id, payment_id);
    }

    #[tokio::test]
    async fn every_authenticated_correlated_rejection_keeps_allocation_unchanged() {
        let trade_id = "5".repeat(64);
        let payment_id = "6".repeat(64);

        let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            50.0,
            Some(100_000.0),
            test_unix_now(),
        );
        assert_eq!(
            correlated_rejection_reason(&mut manager, &fake, &envelope, &payment_id, 1, 100_000.0)
                .await,
            TradeRejectionReason::InvalidAmount
        );

        let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            60.0,
            Some(100_000.0),
            test_unix_now() - stable_channels::constants::TRADE_RESULT_TIMEOUT_SECS - 1,
        );
        assert_eq!(
            correlated_rejection_reason(
                &mut manager,
                &fake,
                &envelope,
                &payment_id,
                expected_trade_fee_msat(50.0, 60.0, 100_000.0).unwrap(),
                100_000.0,
            )
            .await,
            TradeRejectionReason::StaleRequest
        );

        let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            60.0,
            Some(100_000.0),
            test_unix_now(),
        );
        assert_eq!(
            correlated_rejection_reason(&mut manager, &fake, &envelope, &payment_id, 1, 100_000.0)
                .await,
            TradeRejectionReason::InvalidFee
        );

        let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            60.0,
            Some(-1.0),
            test_unix_now(),
        );
        assert_eq!(
            correlated_rejection_reason(&mut manager, &fake, &envelope, &payment_id, 1, 100_000.0)
                .await,
            TradeRejectionReason::InvalidQuote
        );

        let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            60.0,
            Some(90_000.0),
            test_unix_now(),
        );
        assert_eq!(
            correlated_rejection_reason(
                &mut manager,
                &fake,
                &envelope,
                &payment_id,
                expected_trade_fee_msat(50.0, 60.0, 90_000.0).unwrap(),
                100_000.0,
            )
            .await,
            TradeRejectionReason::QuoteDeviation
        );

        let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            110.0,
            Some(100_000.0),
            test_unix_now(),
        );
        assert_eq!(
            correlated_rejection_reason(
                &mut manager,
                &fake,
                &envelope,
                &payment_id,
                expected_trade_fee_msat(50.0, 110.0, 100_000.0).unwrap(),
                100_000.0,
            )
            .await,
            TradeRejectionReason::InsufficientCapacity
        );

        let (mut manager, fake) = correlated_rejection_context(100.0, 100_000, 200_000);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            0.0,
            Some(90_000.0),
            test_unix_now(),
        );
        assert_eq!(
            correlated_rejection_reason(
                &mut manager,
                &fake,
                &envelope,
                &payment_id,
                expected_trade_fee_msat(100.0, 0.0, 90_000.0).unwrap(),
                90_000.0,
            )
            .await,
            TradeRejectionReason::SettlementRequired
        );

        let (mut manager, fake) = correlated_rejection_context(10.0, 49_000, 50_000);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            20.0,
            Some(100_000.0),
            test_unix_now(),
        );
        assert_eq!(
            correlated_rejection_reason(
                &mut manager,
                &fake,
                &envelope,
                &payment_id,
                expected_trade_fee_msat(10.0, 20.0, 100_000.0).unwrap(),
                100_000.0,
            )
            .await,
            TradeRejectionReason::UnsafeAllocation
        );

        let mut manager = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            200_000,
            100_000_000,
            true,
        )]);
        let envelope = correlated_trade_envelope_at(
            &trade_id,
            20.0,
            Some(100_000.0),
            test_unix_now(),
        );
        assert_eq!(
            correlated_rejection_reason(&mut manager, &fake, &envelope, &payment_id, 1, 100_000.0)
                .await,
            TradeRejectionReason::InternalFailure
        );
    }

    #[tokio::test]
    async fn correlated_invalid_signature_receives_no_decision_or_response() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            300_000,
            100_000,
            true,
        )])
        .with_verify_failure();
        let trade_id = "1".repeat(64);
        let payment_id = "2".repeat(64);
        let envelope = correlated_trade_envelope(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            &trade_id,
            60.0,
            100_000.0,
        );
        mgr.handle_trade_payment(
            &envelope,
            Some(&payment_id),
            Some(100_000),
            &fake,
            100_000.0,
        )
        .await;
        assert!(mgr
            .db
            .trade_decision_by_payment(&payment_id)
            .unwrap()
            .is_none());
        StableChannelManager::retry_pending_trade_responses(mgr.db.as_ref(), &fake).await;
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn correlated_unknown_explicit_channel_does_not_fall_back_or_respond() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            300_000,
            100_000,
            true,
        )]);
        let trade_id = "3".repeat(64);
        let payment_id = "4".repeat(64);
        let envelope = correlated_trade_envelope(
            &"f".repeat(64),
            USER_CHANNEL_ID_DECIMAL,
            &trade_id,
            60.0,
            100_000.0,
        );
        mgr.handle_trade_payment(
            &envelope,
            Some(&payment_id),
            Some(100_000),
            &fake,
            100_000.0,
        )
        .await;
        assert!(fake.verify_calls.lock().unwrap().is_empty());
        assert!(mgr
            .db
            .trade_decision_by_payment(&payment_id)
            .unwrap()
            .is_none());
        StableChannelManager::retry_pending_trade_responses(mgr.db.as_ref(), &fake).await;
        assert!(fake.sends.lock().unwrap().is_empty());
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
    async fn tiny_and_noop_trades_preserve_lsp_stability_drift() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            300_000,
            100_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            100.0,
            100_000,
            100_000,
            200_000,
            100_000.0,
        );

        let tiny = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 100.01);
        handle_trade_with_valid_fee(
            &mut mgr,
            &tiny,
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 100_009);

        let noop = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 100.01);
        handle_trade_with_valid_fee(
            &mut mgr,
            &noop,
            &fake as &dyn LdkServerCalls,
            90_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 100_009);
    }

    #[tokio::test]
    async fn trade_arithmetic_underflow_leaves_lsp_state_unchanged() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            300_000,
            100_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            100.0,
            10,
            199_990,
            200_000,
            100_000.0,
        );
        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 99.0);

        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 100.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 10);
        assert_eq!(mgr.stable_channels[0].native_sats, 199_990);
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn full_exit_is_gated_by_lsp_stability_drift() {
        for (price, should_apply) in [(90_000.0, false), (100_001.0, true)] {
            let mut mgr = make_manager();
            let fake = FakeLdkServer::new(vec![make_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                COUNTERPARTY_HEX,
                300_000,
                100_000_000,
                true,
            )]);
            seed_channel(
                &mut mgr,
                189476124653200987495269098788434301048u128,
                COUNTERPARTY_HEX,
                CHANNEL_ID_HEX,
                100.0,
                100_000,
                100_000,
                200_000,
                100_000.0,
            );
            let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 0.0);

            handle_trade_with_valid_fee(
                &mut mgr,
                &env,
                &fake as &dyn LdkServerCalls,
                price,
            )
            .await;

            if should_apply {
                assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
                assert_eq!(mgr.stable_channels[0].backing_sats, 0);
                assert_eq!(fake.sends.lock().unwrap().len(), 1);
            } else {
                assert_eq!(mgr.stable_channels[0].expected_usd.0, 100.0);
                assert_eq!(mgr.stable_channels[0].backing_sats, 100_000);
                assert!(fake.sends.lock().unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn sub_cent_trade_target_is_persisted_as_a_full_exit() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            300_000,
            100_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            100.0,
            100_000,
            100_000,
            200_000,
            100_000.0,
        );
        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 0.009);

        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_001.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 0);
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
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
            None,
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
    async fn trade_uses_lsp_price_instead_of_client_allocation() {
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

        // The client reports a 0.4% higher price and therefore fewer backing sats. The quote is
        // inside the slippage bound, but neither it nor the legacy backing field controls the LSP.
        let env = trade_envelope_with_allocation(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            49.75,
            100_400.0,
            49_551,
        );
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].backing_sats, 49_750);
        assert_eq!(mgr.stable_channels[0].native_sats, 250);
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "the LSP must sync its own allocation");
        let raw = std::str::from_utf8(sends[0].custom_tlvs[0].value.as_ref()).unwrap();
        let sync = crate::messages::parse_envelope(raw).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&sync.payload).unwrap();
        assert_eq!(payload["backing_sats"], 49_750);
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
        assert_eq!(mgr.stable_channels[0].backing_sats, 50_000);
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn trade_ignores_legacy_allocation_not_derived_from_quote() {
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

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 49.95);
        assert_eq!(mgr.stable_channels[0].backing_sats, 49_950);
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
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
    async fn trade_rejects_even_one_sat_above_balance_boundary() {
        let mut mgr = make_manager();
        // Live receiver side = 50_000 sats at $100k -> receiver_usd = $50.00 exactly.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        let target = 50.001;
        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, target);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 0);
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn trade_rejects_dollar_denominated_capacity_epsilon() {
        let mut mgr = make_manager();
        // Live receiver side = 50_000 sats at $100k -> receiver_usd = $50.00 exactly.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        seed_channel(&mut mgr, 189476124653200987495269098788434301048u128, COUNTERPARTY_HEX, CHANNEL_ID_HEX, 0.0, 0, 50_000, 50_000, 100_000.0);

        let target = 50.01;
        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, target);
        handle_trade_with_valid_fee(
            &mut mgr,
            &env,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 0);
        assert!(fake.sends.lock().unwrap().is_empty());
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

    #[test]
    fn splice_balance_change_records_direction_and_net_amount() {
        assert_eq!(splice_balance_change(50_000, 80_000), ("in", 30_000));
        assert_eq!(splice_balance_change(50_000, 5_000), ("out", 45_000));
        assert_eq!(splice_balance_change(50_000, 50_000), ("unchanged", 0));
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
            Some("splice-out-funding:0".to_owned()),
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
            Some("splice-in-funding:0".to_owned()),
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
                Some("replayed-splice-funding:0".to_owned()),
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
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let audit_path = dir.path().join("audit_log.txt");
        // OnceLock: this wins if unset, otherwise we read whichever path is live.
        set_audit_log_path(audit_path.to_str().unwrap());
        let path = get_audit_log_path()
            .expect("an audit log path is set")
            .to_string();

        let mut mgr = make_manager();
        // Record into this manager's ledger; another test's ledger may sit in an already-deleted temp dir.
        stable_channels::audit::set_audit_ledger((*mgr.db).clone());
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
            prev_channel_id: prev.into(),
            next_channel_id: next.into(),
            prev_user_channel_id: Some("10".into()),
            next_user_channel_id: Some("20".into()),
            prev_node_id: Some("02aa".into()),
            next_node_id: Some("02bb".into()),
            total_fee_earned_msat: Some(7),
            outbound_amount_forwarded_msat: Some(amt),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn backfill_emits_unseen_then_dedups() {
        // open_in_memory() is #[cfg(test)]-gated in the shared crate, unreachable across this crate boundary; use the tempdir pattern from make_manager() instead.
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let fake = FakeLdkServer::new(vec![]).with_forwarded(vec![fwd("aa", "bb", 1000), fwd("cc", "dd", 2000)]);
        assert_eq!(crate::backfill::backfill_forwards(&fake, &db, None).await.emitted, 2); // both unseen
        assert_eq!(crate::backfill::backfill_forwards(&fake, &db, None).await.emitted, 0); // both now seen
    }

    #[tokio::test]
    async fn forward_backfill_stops_on_repeated_cursor() {
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let fake = FakeLdkServer::new(vec![]).with_forward_cursor("same-page".to_owned());

        let result = crate::backfill::backfill_forwards(&fake, &db, None).await;
        assert!(result.failure.is_none());
        assert!(result.incomplete.as_deref().unwrap().contains("repeated"));
        assert_eq!(fake.forward_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn reconnect_reconstructs_channel_payment_forward_peer_and_sweep() {
        use ldk_server_client::ldk_server_grpc::types::{
            pending_sweep_balance, PendingBroadcast,
        };
        use stable_channels::ledger::{LedgerQuery, LedgerCompleteness};

        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            40_000_000,
            true,
        )])
        .with_payments(vec![GrpcPayment {
            payment_id: "payment-no-channel".into(),
            amount_msat: Some(21_000),
            fee_paid_msat: Some(10),
            direction: 1,
            status: 1,
            latest_update_timestamp: 123,
            ..Default::default()
        }])
        .with_forwarded(vec![fwd("aa", "bb", 1_000)])
        .with_peers(vec![GrpcPeer {
            node_id: COUNTERPARTY_HEX.into(),
            address: "127.0.0.1:9735".into(),
            is_connected: true,
            ..Default::default()
        }])
        .with_sweeps(vec![GrpcPendingSweepBalance {
            balance_type: Some(pending_sweep_balance::BalanceType::PendingBroadcast(
                PendingBroadcast { channel_id: Some(CHANNEL_ID_HEX.into()), amount_satoshis: 777 },
            )),
        }]);

        let counts = crate::backfill::reconcile_event_history(&fake, &db, None).await;
        assert_eq!(counts.channels, 1);
        assert_eq!(counts.payments, 1);
        assert_eq!(counts.forwards, 1);
        assert_eq!(counts.peers, 1);
        assert_eq!(counts.sweeps, 1);
        assert_eq!(counts.failed_scopes, 0);

        let page = db.list_ledger_events(&LedgerQuery {
            completeness: Some("reconstructed".into()),
            limit: 50,
            ..Default::default()
        }).unwrap();
        assert_eq!(page.events.len(), 5);
        assert!(page.events.iter().all(|event| event.completeness == LedgerCompleteness::Reconstructed));
        let payment = page.events.iter().find(|event| event.event_type == "PAYMENT_RECONSTRUCTED").unwrap();
        assert_eq!(payment.status, "completed");
        assert_eq!(payment.detail["ldk_status"], "SUCCEEDED");
        assert_eq!(payment.detail["channel_association"], "unavailable_from_ldk");
        assert!(!payment.refs.iter().any(|reference| reference.role.contains("channel")));

        let replay_counts = crate::backfill::reconcile_event_history(&fake, &db, None).await;
        assert_eq!(replay_counts.channels, 0);
        assert_eq!(replay_counts.forwards, 0);
        assert_eq!(replay_counts.peers, 0);
        assert_eq!(replay_counts.sweeps, 0);
        let replay_page = db
            .list_ledger_events(&LedgerQuery {
                completeness: Some("reconstructed".into()),
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(replay_page.events.len(), 5);
    }

    #[tokio::test]
    async fn reconnect_persists_missed_successful_settlement_before_live_dispatch() {
        use stable_channels::ledger::LedgerQuery;

        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        db.save_channel("physical", "stable", 10.0, 10_000, 5_000, None)
            .unwrap();
        db.record_stability_settlement_with_rollback(
            "successful-payment",
            "stable",
            "physical",
            10_000,
            9_000,
            5_000,
            10.0,
            0,
            1_000_000,
            "outbound",
            COUNTERPARTY_HEX,
            None,
        )
        .unwrap();
        let fake = FakeLdkServer::new(vec![]).with_payments(vec![GrpcPayment {
            payment_id: "successful-payment".into(),
            amount_msat: Some(1_000_000),
            fee_paid_msat: Some(25),
            direction: 1,
            status: PaymentStatus::Succeeded as i32,
            latest_update_timestamp: 123,
            ..Default::default()
        }]);

        let counts = crate::backfill::reconcile_event_history(&fake, &db, None).await;
        assert!(counts.settlement_outcomes_safe);
        assert!(db.list_pending_settlements().unwrap().is_empty());
        let terminal = db
            .list_ledger_events(&LedgerQuery {
                identifier: Some("successful-payment".into()),
                limit: 20,
                ..Default::default()
            })
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.event_type == "STABILITY_PAYMENT_SETTLED")
            .unwrap();
        assert_eq!(terminal.status, "completed");
    }

    #[tokio::test]
    async fn sweep_reconstruction_identity_survives_unrelated_list_shifts() {
        use ldk_server_client::ldk_server_grpc::types::{
            pending_sweep_balance, AwaitingThresholdConfirmations,
            BroadcastAwaitingConfirmation, PendingBroadcast,
        };

        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        let pending = GrpcPendingSweepBalance {
            balance_type: Some(pending_sweep_balance::BalanceType::PendingBroadcast(
                PendingBroadcast {
                    channel_id: Some("channel-a".into()),
                    amount_satoshis: 111,
                },
            )),
        };
        let broadcast = GrpcPendingSweepBalance {
            balance_type: Some(
                pending_sweep_balance::BalanceType::BroadcastAwaitingConfirmation(
                    BroadcastAwaitingConfirmation {
                        channel_id: Some("channel-b".into()),
                        latest_broadcast_height: 100,
                        latest_spending_txid: "sweep-txid".into(),
                        amount_satoshis: 222,
                    },
                ),
            ),
        };
        let fake = FakeLdkServer::new(vec![])
            .with_sweeps(vec![pending, broadcast.clone()]);
        assert_eq!(
            crate::backfill::reconcile_event_history(&fake, &db, None).await.sweeps,
            2
        );

        *fake.sweeps.lock().unwrap() = vec![broadcast];
        assert_eq!(
            crate::backfill::reconcile_event_history(&fake, &db, None).await.sweeps,
            0,
            "removing an earlier sweep must not make the remaining sweep look new"
        );

        *fake.sweeps.lock().unwrap() = vec![GrpcPendingSweepBalance {
            balance_type: Some(
                pending_sweep_balance::BalanceType::AwaitingThresholdConfirmations(
                    AwaitingThresholdConfirmations {
                        channel_id: Some("channel-b".into()),
                        latest_spending_txid: "sweep-txid".into(),
                        confirmation_hash: "block-hash".into(),
                        confirmation_height: 101,
                        amount_satoshis: 222,
                    },
                ),
            ),
        }];
        assert_eq!(
            crate::backfill::reconcile_event_history(&fake, &db, None).await.sweeps,
            1,
            "the same sweep changing confirmation state must remain visible"
        );
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

    fn signed_stability_record(
        settlement_id: &str,
        channel_id: &str,
        amount_msat: u64,
        direction: stable_channels::stable::StabilityPaymentDirection,
        expected_usd: f64,
    ) -> CustomTlvRecord {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload = stable_channels::stable::build_stability_payment_payload(
            settlement_id,
            channel_id,
            amount_msat,
            direction,
            expected_usd,
            now,
            now + STABILITY_PAYMENT_AUTH_TTL_SECS,
        )
        .unwrap();
        let envelope = stable_channels::stable::build_stability_signed_envelope(
            payload,
            "signed-by-test-peer".to_owned(),
        )
        .unwrap();
        CustomTlvRecord {
            type_num: stable_channels::constants::SIGNED_STABILITY_TLV_TYPE,
            value: envelope.into_bytes().into(),
        }
    }

    async fn manager_at_par_for_signed_stability() -> StableChannelManager {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(10.0),
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);
        mgr
    }

    #[tokio::test]
    async fn signed_stability_payment_is_bound_to_amount_and_applied_once() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = manager_at_par_for_signed_stability().await;
        let settlement_id = "11".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            CHANNEL_ID_HEX,
            909_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.0,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
        )]);

        mgr.handle_payment_received(
            vec![stability_marker(), record.clone()],
            Some("signed-payment-1".to_owned()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 9_091);
        assert_eq!(mgr.stable_channels[0].native_sats, 40_000);
        assert!(mgr
            .db
            .load_channel(USER_CHANNEL_ID_DECIMAL)
            .unwrap()
            .is_some());
        let verify_calls = fake.verify_calls.lock().unwrap();
        assert_eq!(verify_calls.len(), 1);
        assert_eq!(verify_calls[0].public_key, COUNTERPARTY_HEX);
        let envelope = stable_channels::stable::parse_stability_signed_envelope(
            std::str::from_utf8(record.value.as_ref()).unwrap(),
        )
        .unwrap();
        assert_eq!(verify_calls[0].message.as_ref(), envelope.payload.as_bytes());
        drop(verify_calls);
        assert_eq!(
            mgr.db
                .inbound_stability_settlement_state(&settlement_id)
                .unwrap()
                .as_deref(),
            Some("applied")
        );

        mgr.handle_payment_received(
            vec![stability_marker(), record],
            Some("signed-payment-1".to_owned()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;
        assert_eq!(
            mgr.stable_channels[0].backing_sats, 9_091,
            "replaying the event must not apply the amount twice"
        );
    }

    #[tokio::test]
    async fn signed_stability_uses_local_state_when_peer_expected_usd_differs() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = manager_at_par_for_signed_stability().await;
        let settlement_id = "66".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            CHANNEL_ID_HEX,
            909_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.5,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
        )]);

        mgr.handle_payment_received(
            vec![record],
            Some("signed-payment-divergent-target".to_owned()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 10.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 9_091);
        assert_eq!(
            mgr.db
                .inbound_stability_settlement_state(&settlement_id)
                .unwrap()
                .as_deref(),
            Some("applied"),
        );
    }

    #[tokio::test]
    async fn signed_stability_recovers_once_from_durable_backing_cas_conflict() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = manager_at_par_for_signed_stability().await;
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                10.0,
                9_999,
                40_001,
                None,
            )
            .unwrap();
        let settlement_id = "77".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            CHANNEL_ID_HEX,
            909_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.0,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
        )]);

        mgr.handle_payment_received(
            vec![record],
            Some("signed-payment-stale-allocation".to_owned()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].backing_sats, 9_090);
        assert_eq!(
            mgr.db
                .load_channel(USER_CHANNEL_ID_DECIMAL)
                .unwrap()
                .unwrap()
                .backing_sats,
            9_090,
        );
    }

    #[tokio::test]
    async fn signed_stability_migrates_a_legacy_noncanonical_channel_row() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = manager_at_par_for_signed_stability().await;
        let legacy_user_channel_id = format!(
            "{:032x}",
            USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap()
        );
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                &legacy_user_channel_id,
                10.0,
                10_000,
                40_000,
                None,
            )
            .unwrap();
        assert!(mgr.db.load_channel(USER_CHANNEL_ID_DECIMAL).unwrap().is_none());

        let settlement_id = "99".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            CHANNEL_ID_HEX,
            909_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.0,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
        )]);

        mgr.handle_payment_received(
            vec![record],
            Some("signed-payment-legacy-channel-row".to_owned()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;

        assert_eq!(
            mgr.db
                .load_channel(USER_CHANNEL_ID_DECIMAL)
                .unwrap()
                .unwrap()
                .backing_sats,
            9_091,
        );
        assert!(mgr
            .db
            .load_channel(&legacy_user_channel_id)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn signed_stability_invalidates_an_untracked_channel_instead_of_retrying_forever() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let settlement_id = "88".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            CHANNEL_ID_HEX,
            1_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.0,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_001_000,
            true,
        )]);

        mgr.handle_payment_received(
            vec![record],
            Some("signed-payment-untracked-channel".to_owned()),
            Some(1_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;

        assert_eq!(
            mgr.db
                .inbound_stability_settlement_state(&settlement_id)
                .unwrap()
                .as_deref(),
            Some("invalid"),
        );
        assert!(mgr
            .db
            .pending_inbound_stability_settlements(32)
            .unwrap()
            .is_empty());
        assert!(fake.verify_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn signed_stability_payment_rejects_amount_mismatch() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = manager_at_par_for_signed_stability().await;
        let settlement_id = "22".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            CHANNEL_ID_HEX,
            909_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.0,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            51_000_000,
            true,
        )]);

        mgr.handle_payment_received(
            vec![stability_marker(), record],
            Some("signed-payment-wrong-amount".to_owned()),
            Some(1_000_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);
        assert_eq!(
            mgr.db
                .inbound_stability_settlement_state(&settlement_id)
                .unwrap()
                .as_deref(),
            None
        );
    }

    #[tokio::test]
    async fn signed_stability_payment_is_bound_to_the_claimed_channel() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = manager_at_par_for_signed_stability().await;
        let settlement_id = "55".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            &"aa".repeat(32),
            909_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.0,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
        )]);

        mgr.handle_payment_received(
            vec![record],
            Some("signed-payment-wrong-channel".to_owned()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);
        assert_eq!(
            mgr.db
                .inbound_stability_settlement_state(&settlement_id)
                .unwrap()
                .as_deref(),
            Some("invalid")
        );
        assert!(fake.verify_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn signed_stability_payment_retries_from_durable_inbox() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = manager_at_par_for_signed_stability().await;
        let settlement_id = "44".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            CHANNEL_ID_HEX,
            909_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.0,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
        )]);

        // The event is durably registered, but cannot be accounted without a trusted price.
        mgr.handle_payment_received(
            vec![record],
            Some("signed-payment-retry".to_owned()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            0.0,
        )
        .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);
        assert_eq!(
            mgr.db
                .inbound_stability_settlement_state(&settlement_id)
                .unwrap()
                .as_deref(),
            Some("pending")
        );

        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 110_000.0)
            .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 9_091);
        assert_eq!(
            mgr.db
                .inbound_stability_settlement_state(&settlement_id)
                .unwrap()
                .as_deref(),
            Some("applied")
        );
    }

    #[tokio::test]
    async fn signed_stability_payment_rejects_invalid_signature_without_legacy_fallback() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = manager_at_par_for_signed_stability().await;
        let settlement_id = "33".repeat(32);
        let record = signed_stability_record(
            &settlement_id,
            CHANNEL_ID_HEX,
            909_000,
            stable_channels::stable::StabilityPaymentDirection::UserToLsp,
            10.0,
        );
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
        )])
        .with_verify_failure();

        mgr.handle_payment_received(
            vec![stability_marker(), record],
            Some("signed-payment-bad-signature".to_owned()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);
        assert_eq!(
            mgr.db
                .inbound_stability_settlement_state(&settlement_id)
                .unwrap()
                .as_deref(),
            Some("invalid")
        );
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
        // Amount-proportional: backing 10_000 minus the 909 sats actually paid = 9_091,
        // one sat above the truncated equilibrium (9_090) — that residual sat is the honest
        // rounding remainder the wallet under-paid, not stolen surplus.
        assert_eq!(sc.backing_sats, 9_091, "backing is reduced by the settled amount");
        // Native absorbs only rounding, never the settlement: 49_091 - 9_091 = 40_000.
        assert_eq!(sc.native_sats, 40_000, "native sats must be preserved across the settlement");
        assert!(sc.backing_sats <= sc.stable_receiver_btc.sats, "backing may never exceed live balance");
    }

    #[tokio::test]
    async fn token_stability_payment_settles_only_the_amount_paid() {
        // Exploit guard: a 1-sat payment against a large above-par surplus must settle
        // exactly 1 sat, NOT reset backing to equilibrium (which would hand the entire
        // surplus to the sender as free native BTC for a fraction of a cent).
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_000_000, true,
        )]);
        mgr.edit_stable_channel(CHANNEL_ID_HEX, Some(10.0), None, &fake0 as &dyn LdkServerCalls, 100_000.0).await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::push::PushService::new(&crate::config::PushConfig::default(), mgr.data_dir()),
        ));
        // Snapshot at par: backing = 10_000 sats, user side = 50_000.
        mgr.run_tick(&fake0 as &dyn LdkServerCalls, &push, 100_000.0).await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);

        // Price rises to $110k: the honest owed amount is ~910 sats. The attacker instead
        // keysends 1 sat with the same marker. The user paying 1 sat raises the LSP's
        // outbound by 1 sat (50_000_000 -> 50_001_000), so the user side drops 50_000 -> 49_999.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX, USER_CHANNEL_ID_HEX, COUNTERPARTY_HEX, 100_000, 50_001_000, true,
        )]);
        mgr.handle_payment_received(
            vec![stability_marker()],
            Some("attack-1".to_string()),
            Some(1_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;

        let sc = &mgr.stable_channels[0];
        // Only 1 sat of drift settled: 10_000 - 1 = 9_999. The surplus stays owed as backing,
        // NOT erased to the $110k equilibrium of 9_090.
        assert_eq!(sc.backing_sats, 9_999, "a 1-sat payment settles only 1 sat of drift");
        assert_ne!(sc.backing_sats, 9_090, "backing must NOT collapse to equilibrium for a token payment");
        assert_eq!(sc.native_sats, 40_000, "the surplus is not reclassified as free native BTC");
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

    const SPLICE_TXO: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b:1";

    fn ledger_for(db: &stable_channels::db::Database, identifier: &str) -> Vec<stable_channels::ledger::LedgerEvent> {
        db.list_ledger_events(&stable_channels::ledger::LedgerQuery {
            identifier: Some(identifier.into()),
            limit: 50,
            ..Default::default()
        })
        .unwrap()
        .events
    }

    #[tokio::test]
    async fn splice_lifecycle_events_land_in_the_user_channel_ledger() {
        use ldk_server_client::ldk_server_grpc::events::{
            event_envelope::Event, SpliceNegotiated, SpliceNegotiationFailed,
        };
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let db = mgr.db.clone();
        stable_channels::audit::set_audit_ledger((*db).clone());
        let fake = FakeLdkServer::new(vec![]);
        let negotiated = Event::SpliceNegotiated(SpliceNegotiated {
            channel_id: CHANNEL_ID_HEX.into(),
            user_channel_id: USER_CHANNEL_ID_DECIMAL.into(),
            counterparty_node_id: COUNTERPARTY_HEX.into(),
            new_funding_txo: SPLICE_TXO.into(),
        });
        let failed = Event::SpliceNegotiationFailed(SpliceNegotiationFailed {
            channel_id: CHANNEL_ID_HEX.into(),
            user_channel_id: USER_CHANNEL_ID_DECIMAL.into(),
            counterparty_node_id: COUNTERPARTY_HEX.into(),
        });
        for event in [negotiated.clone(), negotiated, failed] {
            let outcome = crate::event_loop::dispatch_event(Some(event), &mut mgr, &db, &fake, 80_000.0).await;
            assert_eq!(outcome, crate::event_loop::DispatchOutcome::Continue);
        }
        let events = ledger_for(&db, USER_CHANNEL_ID_DECIMAL);
        let negotiated: Vec<_> = events.iter().filter(|e| e.event_type == "SPLICE_NEGOTIATED").collect();
        assert_eq!(negotiated.len(), 1, "a replayed SpliceNegotiated is recorded once");
        assert_eq!(negotiated[0].category, "channel");
        assert!(negotiated[0].refs.iter().any(|r| r.role == "transaction_id" && r.value == SPLICE_TXO));
        let failed = events.iter().find(|e| e.event_type == "SPLICE_NEGOTIATION_FAILED").expect("failed round must be audited");
        assert_eq!(failed.category, "channel");
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.detail["counterparty_node_id"], COUNTERPARTY_HEX);
        assert!(mgr.stable_channels.is_empty(), "splice events are audit-only");
        assert!(fake.sends.lock().unwrap().is_empty(), "splice events are audit-only");
    }

    #[tokio::test]
    async fn failed_payment_row_records_the_ldk_failure_reason() {
        use ldk_server_client::ldk_server_grpc::events::{
            event_envelope::Event, PaymentFailed, PaymentFailureReason,
        };
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let db = mgr.db.clone();
        stable_channels::audit::set_audit_ledger((*db).clone());
        let fake = FakeLdkServer::new(vec![]);
        for (id, reason) in [("route-miss", Some(PaymentFailureReason::RouteNotFound as i32)), ("no-reason", None)] {
            let event = Event::PaymentFailed(PaymentFailed {
                payment_id: id.into(),
                payment: Some(GrpcPayment {
                    payment_id: id.into(),
                    amount_msat: Some(1),
                    direction: 1,
                    status: PaymentStatus::Failed as i32,
                    ..Default::default()
                }),
                reason,
            });
            crate::event_loop::dispatch_event(Some(event), &mut mgr, &db, &fake, 80_000.0).await;
        }
        let row = |id: &str| ledger_for(&db, id).into_iter().find(|e| e.event_type == "PAYMENT_FAILED").unwrap();
        assert_eq!(row("route-miss").detail["reason"], "ROUTE_NOT_FOUND");
        assert!(row("no-reason").detail["reason"].is_null());
    }

    #[tokio::test]
    async fn pending_channel_row_records_funding_outpoint_and_temporary_id() {
        use ldk_server_client::ldk_server_grpc::events::{
            event_envelope::Event, ChannelState, ChannelStateChanged,
        };
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let db = mgr.db.clone();
        stable_channels::audit::set_audit_ledger((*db).clone());
        let fake = FakeLdkServer::new(vec![]);
        let event = Event::ChannelStateChanged(ChannelStateChanged {
            channel_id: CHANNEL_ID_HEX.into(),
            user_channel_id: USER_CHANNEL_ID_DECIMAL.into(),
            counterparty_node_id: Some(COUNTERPARTY_HEX.into()),
            state: ChannelState::Pending as i32,
            funding_txo: Some(SPLICE_TXO.into()),
            former_temporary_channel_id: Some("7e3a8b".into()),
            ..Default::default()
        });
        crate::event_loop::dispatch_event(Some(event), &mut mgr, &db, &fake, 80_000.0).await;
        let row = ledger_for(&db, USER_CHANNEL_ID_DECIMAL)
            .into_iter()
            .find(|e| e.event_type == "CHANNEL_PENDING")
            .unwrap();
        assert_eq!(row.detail["funding_txo"], SPLICE_TXO);
        assert_eq!(row.detail["former_temporary_channel_id"], "7e3a8b");
        assert!(row.refs.iter().any(|r| r.role == "transaction_id" && r.value == SPLICE_TXO));
    }

    fn channel_with_snapshot_fields() -> GrpcChannel {
        use ldk_server_client::ldk_server_grpc::types::{ChannelShutdownState, ReserveType};
        GrpcChannel {
            short_channel_id: Some(934_190_049_236_975_617),
            outbound_scid_alias: Some(17_592_186_044_417),
            inbound_htlc_minimum_msat: 1_000,
            reserve_type: Some(ReserveType::Adaptive as i32),
            channel_shutdown_state: Some(ChannelShutdownState::NotShuttingDown as i32),
            ..make_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 40_000_000, true)
        }
    }

    #[tokio::test]
    async fn ready_tracked_row_records_channel_snapshot_fields() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let db = mgr.db.clone();
        stable_channels::audit::set_audit_ledger((*db).clone());
        let fake = FakeLdkServer::new(vec![channel_with_snapshot_fields()]);
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.into(),
            USER_CHANNEL_ID_DECIMAL.into(),
            Some(SPLICE_TXO.into()),
            &fake,
            80_000.0,
        )
        .await;
        let row = ledger_for(&db, USER_CHANNEL_ID_DECIMAL)
            .into_iter()
            .find(|e| e.event_type == "CHANNEL_READY_TRACKED")
            .unwrap();
        assert_eq!(row.detail["short_channel_id"], 934_190_049_236_975_617u64);
        assert_eq!(row.detail["outbound_scid_alias"], 17_592_186_044_417u64);
        assert_eq!(row.detail["inbound_htlc_minimum_msat"], 1_000u64);
        assert_eq!(row.detail["reserve_type"], "ADAPTIVE");
        assert_eq!(row.detail["channel_shutdown_state"], "NOT_SHUTTING_DOWN");
        assert_eq!(row.detail["funding_txo"], SPLICE_TXO);
    }

    #[tokio::test]
    async fn reconstructed_channel_row_records_channel_snapshot_fields() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        let fake = FakeLdkServer::new(vec![channel_with_snapshot_fields()]);
        crate::backfill::reconcile_event_history(&fake, &db, None).await;
        let row = ledger_for(&db, USER_CHANNEL_ID_DECIMAL)
            .into_iter()
            .find(|e| e.event_type == "CHANNEL_RECONSTRUCTED")
            .unwrap();
        assert_eq!(row.detail["short_channel_id"], 934_190_049_236_975_617u64);
        assert_eq!(row.detail["reserve_type"], "ADAPTIVE");
        assert_eq!(row.detail["channel_shutdown_state"], "NOT_SHUTTING_DOWN");
        crate::backfill::reconcile_event_history(&fake, &db, None).await;
        let rows = ledger_for(&db, USER_CHANNEL_ID_DECIMAL)
            .into_iter()
            .filter(|e| e.event_type == "CHANNEL_RECONSTRUCTED")
            .count();
        assert_eq!(rows, 1, "an unchanged snapshot is not reconstructed again");
    }

    #[tokio::test]
    async fn shutdown_stage_rows_reach_the_ledger_once_across_restarts() {
        use ldk_server_client::ldk_server_grpc::types::ChannelShutdownState;
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        let closing = GrpcChannel {
            channel_shutdown_state: Some(ChannelShutdownState::ShutdownInitiated as i32),
            ..make_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 40_000_000, false)
        };
        let seen = crate::observability::record_shutdown_stages(&std::collections::HashMap::new(), &[closing.clone()]);
        // A daemon restart forgets the in-memory baseline and sees the same stage again.
        crate::observability::record_shutdown_stages(&std::collections::HashMap::new(), &[closing.clone()]);
        let resolving = GrpcChannel {
            channel_shutdown_state: Some(ChannelShutdownState::ResolvingHtlcs as i32),
            ..closing
        };
        crate::observability::record_shutdown_stages(&seen, &[resolving]);
        let rows: Vec<_> = ledger_for(&db, USER_CHANNEL_ID_DECIMAL)
            .into_iter()
            .filter(|e| e.event_type == "CHANNEL_SHUTDOWN_STATE_CHANGED")
            .collect();
        let mut stages: Vec<_> = rows.iter().map(|e| e.detail["shutdown_state"].as_str().unwrap().to_owned()).collect();
        stages.sort();
        assert_eq!(stages, vec!["RESOLVING_HTLCS", "SHUTDOWN_INITIATED"]);
        assert!(rows.iter().all(|e| e.category == "channel"));
    }

    #[tokio::test]
    async fn forwarded_rows_record_the_jit_skimmed_fee() {
        use ldk_server_client::ldk_server_grpc::events::{event_envelope::Event, PaymentForwarded};
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let db = mgr.db.clone();
        stable_channels::audit::set_audit_ledger((*db).clone());
        let locator = |channel_id: &str, user_channel_id: &str, node_id: &str| HtlcLocator {
            channel_id: channel_id.into(),
            user_channel_id: Some(user_channel_id.into()),
            node_id: Some(node_id.into()),
            amount_msat: None,
        };
        let missed = GrpcForwardedPayment { skimmed_fee_msat: Some(4_000), total_fee_earned_msat: Some(4_500), ..fwd("gap-prev", "gap-next", 80_000) };
        let fake = FakeLdkServer::new(vec![]).with_forwarded(vec![missed]);
        let event = Event::PaymentForwarded(PaymentForwarded {
            prev_htlcs: vec![locator("live-prev", "10", "02aa")],
            next_htlcs: vec![locator("live-next", "20", "02bb")],
            total_fee_earned_msat: Some(3_000),
            skimmed_fee_msat: Some(2_500),
            outbound_amount_forwarded_msat: 90_000,
            ..Default::default()
        });
        crate::event_loop::dispatch_event(Some(event), &mut mgr, &db, &fake, 80_000.0).await;
        crate::backfill::backfill_forwards(&fake, &db, None).await;
        let live_row = ledger_for(&db, "live-prev").into_iter().find(|e| e.event_type == "PAYMENT_FORWARDED").unwrap();
        assert_eq!(live_row.detail["skimmed_fee_msat"], 2_500u64);
        let backfill_row = ledger_for(&db, "gap-prev").into_iter().find(|e| e.event_type == "PAYMENT_FORWARDED_BACKFILL").unwrap();
        assert_eq!(backfill_row.detail["skimmed_fee_msat"], 4_000u64);
    }

    #[tokio::test]
    async fn forward_backfill_reports_a_gap_when_ldk_server_keeps_only_stats() {
        use ldk_server_client::ldk_server_grpc::types::ForwardedPaymentTrackingMode;
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let stats = FakeLdkServer::new(vec![])
            .with_forwarded(vec![fwd("aa", "bb", 1_000)])
            .with_tracking_mode(ForwardedPaymentTrackingMode::Stats);
        let result = crate::backfill::backfill_forwards(&stats, &db, None).await;
        assert_eq!(result.emitted, 0);
        assert!(result.failure.is_none());
        assert!(result.incomplete.as_deref().unwrap().contains("forwarded_payment_tracking_mode"));
        assert_eq!(stats.forward_calls.load(Ordering::SeqCst), 0, "stats mode has no per-payment list to read");

        let detailed = FakeLdkServer::new(vec![])
            .with_forwarded(vec![fwd("aa", "bb", 1_000)])
            .with_tracking_mode(ForwardedPaymentTrackingMode::Detailed);
        let result = crate::backfill::backfill_forwards(&detailed, &db, None).await;
        assert_eq!(result.emitted, 1);
        assert!(result.incomplete.is_none());
    }

    #[tokio::test]
    async fn backfilled_forward_rows_carry_the_ldk_forward_id_and_time() {
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let dated = GrpcForwardedPayment {
            id: "7f3a9c0e41d2".into(),
            forwarded_at_timestamp: 1_758_000_000,
            ..fwd("dated-prev", "dated-next", 5_000)
        };
        let undated = fwd("undated-prev", "undated-next", 6_000);
        let fake = FakeLdkServer::new(vec![]).with_forwarded(vec![dated, undated]);
        crate::backfill::backfill_forwards(&fake, &db, None).await;
        let row = ledger_for(&db, "dated-prev").into_iter().find(|e| e.event_type == "PAYMENT_FORWARDED_BACKFILL").unwrap();
        assert_eq!(row.detail["forwarded_payment_id"], "7f3a9c0e41d2");
        assert_eq!(row.occurred_at_ms, 1_758_000_000_000, "the row is placed at the real forwarding time");
        let row = ledger_for(&db, "undated-prev").into_iter().find(|e| e.event_type == "PAYMENT_FORWARDED_BACKFILL").unwrap();
        assert!(row.occurred_at_ms > 1_758_000_000_000, "an unknown forwarding time falls back to now");
    }

    fn onchain_channel_tx(
        payment_id: &str,
        tx_type: ldk_server_client::ldk_server_grpc::types::transaction_type::Kind,
        confirmed_at: Option<u32>,
    ) -> GrpcPayment {
        use ldk_server_client::ldk_server_grpc::types::{
            confirmation_status, payment_kind, ConfirmationStatus, Confirmed, Onchain, PaymentKind,
            TransactionType, Unconfirmed,
        };
        let status = match confirmed_at {
            Some(height) => confirmation_status::Status::Confirmed(Confirmed { block_hash: "00ab".into(), height, timestamp: 1_758_000_000 }),
            None => confirmation_status::Status::Unconfirmed(Unconfirmed {}),
        };
        GrpcPayment {
            payment_id: payment_id.into(),
            kind: Some(PaymentKind {
                kind: Some(payment_kind::Kind::Onchain(Onchain {
                    txid: format!("{payment_id}-txid"),
                    status: Some(ConfirmationStatus { status: Some(status) }),
                    tx_type: Some(TransactionType { kind: Some(tx_type) }),
                })),
            }),
            amount_msat: Some(100_000_000),
            direction: 1,
            status: if confirmed_at.is_some() { PaymentStatus::Succeeded } else { PaymentStatus::Pending } as i32,
            ..Default::default()
        }
    }

    fn funding_of(channel_id: &str) -> ldk_server_client::ldk_server_grpc::types::transaction_type::Kind {
        use ldk_server_client::ldk_server_grpc::types::{transaction_type, Funding, TransactionChannel};
        transaction_type::Kind::Funding(Funding {
            channels: vec![TransactionChannel { counterparty_node_id: COUNTERPARTY_HEX.into(), channel_id: channel_id.into() }],
        })
    }

    #[tokio::test]
    async fn onchain_funding_rows_follow_the_transaction_until_it_confirms() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        let channel = make_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 40_000_000, true);
        let fake = FakeLdkServer::new(vec![channel.clone()])
            .with_payments(vec![onchain_channel_tx("funding", funding_of(CHANNEL_ID_HEX), None)])
            .with_payments_page_size(1);
        let mut pending = std::collections::HashSet::new();
        crate::observability::record_onchain_channel_txs(&fake, &db, &[channel.clone()], &mut pending).await;
        assert!(pending.contains("funding"), "an unconfirmed channel transaction is tracked");

        // A newer Lightning payment pushes the funding transaction off the first page before it confirms.
        *fake.payments.lock().unwrap() = vec![
            GrpcPayment { payment_id: "newer-lightning".into(), ..Default::default() },
            onchain_channel_tx("funding", funding_of(CHANNEL_ID_HEX), Some(861_204)),
        ];
        crate::observability::record_onchain_channel_txs(&fake, &db, &[channel.clone()], &mut pending).await;
        assert!(pending.is_empty(), "a confirmed transaction is no longer tracked");

        // After a restart the confirmed state is seen again but not written twice.
        *fake.payments_page_size.lock().unwrap() = None;
        crate::observability::record_onchain_channel_txs(&fake, &db, &[channel], &mut std::collections::HashSet::new()).await;

        let rows: Vec<_> = ledger_for(&db, USER_CHANNEL_ID_DECIMAL)
            .into_iter()
            .filter(|e| e.event_type == "CHANNEL_ONCHAIN_TX")
            .collect();
        let mut statuses: Vec<_> = rows.iter().map(|e| e.status.clone()).collect();
        statuses.sort();
        assert_eq!(statuses, vec!["completed", "pending"]);
        assert!(rows.iter().all(|e| e.category == "channel" && e.detail["tx_type"] == "FUNDING"));
    }

    #[tokio::test]
    async fn reconnect_reconstruction_links_a_close_transaction_to_its_closed_channel() {
        use ldk_server_client::ldk_server_grpc::types::{transaction_type, CooperativeClose};
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        db.save_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 0.0, 0, 0, None).unwrap();
        db.mark_channel_closed(USER_CHANNEL_ID_DECIMAL).unwrap();
        let close = transaction_type::Kind::CooperativeClose(CooperativeClose {
            counterparty_node_id: COUNTERPARTY_HEX.into(),
            channel_id: CHANNEL_ID_HEX.into(),
        });
        let fake = FakeLdkServer::new(vec![]).with_payments(vec![onchain_channel_tx("close", close, Some(861_300))]);
        crate::backfill::reconcile_event_history(&fake, &db, None).await;
        let rows: Vec<_> = ledger_for(&db, USER_CHANNEL_ID_DECIMAL)
            .into_iter()
            .filter(|e| e.event_type == "CHANNEL_ONCHAIN_TX")
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].detail["tx_type"], "COOPERATIVE_CLOSE");
        assert_eq!(rows[0].status, "completed");
    }

    #[tokio::test]
    async fn forward_backfill_reports_forwards_lost_past_ldk_retention_but_still_backfills() {
        use ldk_server_client::ldk_server_grpc::types::ForwardedPaymentTrackingMode;
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let fake = FakeLdkServer::new(vec![])
            .with_forwarded(vec![fwd("aa", "bb", 1_000)])
            .with_tracking_mode(ForwardedPaymentTrackingMode::Detailed);
        let long_ago = StableChannelManager::unix_time_secs() as i64 * 1_000 - 3 * 3_600_000;
        let result = crate::backfill::backfill_forwards(&fake, &db, Some(long_ago)).await;
        assert_eq!(result.emitted, 1, "what LDK Server still holds is backfilled");
        assert!(result.incomplete.is_none());
        assert!(result.lost.as_deref().unwrap().contains("hourly"));

        let just_now = StableChannelManager::unix_time_secs() as i64 * 1_000 - 60_000;
        let result = crate::backfill::backfill_forwards(&fake, &db, Some(just_now)).await;
        assert!(result.lost.is_none(), "a short gap is fully inside LDK Server's detailed history");
    }

    #[tokio::test]
    async fn reconnect_counts_retention_loss_without_blocking_the_gap_from_closing() {
        use ldk_server_client::ldk_server_grpc::types::ForwardedPaymentTrackingMode;
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        let fake = FakeLdkServer::new(vec![]).with_tracking_mode(ForwardedPaymentTrackingMode::Detailed);
        let counts = crate::backfill::reconcile_event_history(&fake, &db, Some(1_000)).await;
        assert_eq!(counts.lost_scopes, 1);
        assert_eq!(counts.incomplete_scopes, 0, "unrecoverable history must not keep the gap open forever");
        assert_eq!(counts.failed_scopes, 0);
        let row = db
            .list_ledger_events(&stable_channels::ledger::LedgerQuery { limit: 50, ..Default::default() })
            .unwrap()
            .events
            .into_iter()
            .find(|e| e.event_type == "RECONCILIATION_GAP_DETECTED")
            .unwrap();
        assert_eq!(row.detail["scope"], "forwards");
        assert_eq!(row.detail["recoverable"], false);
        assert_eq!(row.detail["gap_started_ms"], 1_000);
    }

    #[tokio::test]
    async fn an_unreadable_tracking_mode_marks_forwards_incomplete() {
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let mut fake = FakeLdkServer::new(vec![]).with_forwarded(vec![fwd("aa", "bb", 1_000)]);
        fake.tracking_mode_fails = true;
        let result = crate::backfill::backfill_forwards(&fake, &db, None).await;
        assert!(result.incomplete.as_deref().unwrap().contains("tracking mode"));
        assert_eq!(result.emitted, 1, "the list is still read");
    }

    #[tokio::test]
    async fn a_restart_keeps_following_transactions_that_were_unconfirmed() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        let channel = make_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 100_000, 40_000_000, true);
        let fake = FakeLdkServer::new(vec![channel.clone()])
            .with_payments(vec![onchain_channel_tx("funding", funding_of(CHANNEL_ID_HEX), None)]);
        crate::observability::record_onchain_channel_txs(&fake, &db, &[channel.clone()], &mut std::collections::HashSet::new()).await;

        // The daemon restarts; meanwhile newer payments push the transaction off the first page and it confirms.
        *fake.payments.lock().unwrap() = vec![
            GrpcPayment { payment_id: "newer-lightning".into(), ..Default::default() },
            onchain_channel_tx("funding", funding_of(CHANNEL_ID_HEX), Some(861_204)),
        ];
        *fake.payments_page_size.lock().unwrap() = Some(1);
        let mut pending = crate::observability::pending_onchain_payment_ids(&db);
        assert!(pending.contains("funding"), "the ledger's unsettled rows seed the new process");
        crate::observability::record_onchain_channel_txs(&fake, &db, &[channel], &mut pending).await;

        let mut statuses: Vec<_> = ledger_for(&db, USER_CHANNEL_ID_DECIMAL)
            .into_iter()
            .filter(|e| e.event_type == "CHANNEL_ONCHAIN_TX")
            .map(|e| e.status)
            .collect();
        statuses.sort();
        assert_eq!(statuses, vec!["completed", "pending"]);
        assert!(pending.is_empty());
    }
}
