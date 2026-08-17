//! In-memory stable-channel manager, backed by the shared sqlite channels table.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use ldk_server_client::client::LdkServerClient;
use ldk_server_client::error::LdkServerError;
use ldk_server_client::ldk_server_grpc::api::{
    GetBalancesRequest, GetBalancesResponse, GetPaymentDetailsRequest, GetPaymentDetailsResponse,
    ListChannelsRequest, ListChannelsResponse, ListForwardedPaymentsRequest,
    ListForwardedPaymentsResponse, ListPaymentsRequest, ListPaymentsResponse, ListPeersRequest,
    ListPeersResponse, SignMessageRequest, SignMessageResponse, SpontaneousSendRequest,
    SpontaneousSendResponse, VerifySignatureRequest, VerifySignatureResponse,
};
use ldk_server_client::ldk_server_grpc::events::ChannelStateChangeReason;
use ldk_server_client::ldk_server_grpc::types::{Channel, CustomTlvRecord, PaymentStatus};
use stable_channels::constants::{
    MAX_TRADE_QUOTE_DEVIATION_PERCENT, SIGNED_STABILITY_TLV_TYPE, STABILITY_PAYMENT_AUTH_TTL_SECS,
};
use stable_channels::db::{Database, InboundStabilityRegistration, PendingTradeResponse};
use stable_channels::stable::StabilityPaymentDirection;
use stable_channels::trade::TradeRejectionReason;
use stable_channels::types::{Bitcoin, StableChannel, USD};
use tracing::{error, info};

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

fn splice_balance_change(before_sats: u64, after_sats: u64) -> (&'static str, u64) {
    if after_sats > before_sats {
        ("in", after_sats - before_sats)
    } else if after_sats < before_sats {
        ("out", before_sats - after_sats)
    } else {
        ("unchanged", 0)
    }
}

fn durable_trade_response_is_sync(response_envelope: &str) -> bool {
    crate::messages::parse_envelope(response_envelope)
        .and_then(|envelope| serde_json::from_str::<serde_json::Value>(&envelope.payload).ok())
        .and_then(|payload| {
            payload
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(|kind| kind == stable_channels::constants::SYNC_MESSAGE_TYPE)
        })
        .unwrap_or(false)
}

fn stability_mutation_allowed<T, E>(reservation: &Result<Option<T>, E>) -> bool {
    matches!(reservation, Ok(None))
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
    ((expected_msat as f64 * MAX_TRADE_QUOTE_DEVIATION_PERCENT / 100.0).ceil() as u64).max(1000)
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
    (old_target.floor() as u64).saturating_sub(new_target.floor() as u64) >= current_backing_sats
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
    async fn list_peers(&self, req: ListPeersRequest) -> Result<ListPeersResponse, LdkServerError> {
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
        let is_sync = durable_trade_response_is_sync(&response.response_envelope);
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
                custom_tlvs: vec![CustomTlvRecord {
                    type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
                    value: response.response_envelope.clone().into_bytes().into(),
                }],
            })
            .await;
        match send {
            Ok(sent) if !sent.payment_id.is_empty() => {
                match db
                    .mark_trade_response_in_flight(&response.inbound_payment_id, &sent.payment_id)
                {
                    Ok(true) => {
                        if is_sync {
                            if let Err(error) = db.record_settlement(&sent.payment_id, "sync") {
                                tracing::error!(
                                    "[stable] record_settlement (durable trade sync) failed: {}",
                                    error
                                );
                                stable_channels::audit::audit_event(
                                    "DB_WRITE_FAILED",
                                    serde_json::json!({
                                        "op": "record_settlement",
                                        "kind": "sync",
                                        "payment_id": sent.payment_id,
                                        "trade_payment_id": response.inbound_payment_id,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                        }
                        stable_channels::audit::audit_event(
                            "TRADE_RESPONSE_SENT",
                            serde_json::json!({
                                "trade_payment_id": response.inbound_payment_id,
                                "response_payment_id": sent.payment_id,
                                "attempt": response.attempts.saturating_add(1),
                            }),
                        );
                    }
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
                    let _ = db.mark_trade_response_delivered(&payment_id, Self::unix_time_secs());
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

    async fn reject_reserved_trade(
        &self,
        ldk: &dyn LdkServerCalls,
        trade_id: &str,
        payment_id: &str,
        request_hash: &str,
        channel_id: &str,
        reason: TradeRejectionReason,
    ) {
        let decided_at = Self::unix_time_secs();
        let payload = crate::messages::build_trade_rejected_payload(
            channel_id,
            trade_id,
            payment_id,
            request_hash,
            reason,
            decided_at.max(0) as u64,
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
        match self.db.reject_trade_reservation(
            trade_id,
            payment_id,
            request_hash,
            reason.as_str(),
            decided_at,
            &envelope,
        ) {
            Ok(true) => stable_channels::audit::audit_event(
                "TRADE_REJECTION_QUEUED",
                serde_json::json!({
                    "trade_id": trade_id,
                    "trade_payment_id": payment_id,
                    "request_hash": request_hash,
                    "reason_code": reason.as_str(),
                }),
            ),
            Ok(false) => {}
            Err(error) => stable_channels::audit::audit_event(
                "DB_WRITE_FAILED",
                serde_json::json!({
                    "op": "reject_trade_reservation",
                    "trade_id": trade_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }

    async fn handle_trade_proposal(
        &mut self,
        payload: &crate::messages::TradePayload,
        envelope: &crate::messages::SignedEnvelope,
        chan: &Channel,
        inbound_payment_id: Option<&str>,
        amount_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let Some(trade_id) = payload
            .trade_id
            .as_deref()
            .filter(|value| stable_channels::trade::is_trade_id(value))
        else {
            return;
        };
        let Some(proposal_payment_id) =
            inbound_payment_id.filter(|value| stable_channels::trade::is_payment_id(value))
        else {
            return;
        };
        let proposal_hash = stable_channels::trade::request_hash(envelope.payload.as_bytes());
        if let Ok(Some(existing)) = self.db.trade_reservation_by_trade_id(trade_id) {
            if existing.proposal_payment_id == proposal_payment_id
                && existing.proposal_hash == proposal_hash
                && existing.outcome == "reserved"
            {
                let _ = self.db.requeue_exact_trade_response(
                    &existing.proposal_payment_id,
                    trade_id,
                    &proposal_hash,
                    Self::unix_time_secs(),
                );
            }
            return;
        }

        macro_rules! reject_proposal {
            ($reason:expr) => {{
                self.reject_correlated_trade(
                    ldk,
                    proposal_payment_id,
                    trade_id,
                    &proposal_hash,
                    &chan.channel_id,
                    &chan.user_channel_id,
                    &chan.counterparty_node_id,
                    $reason,
                )
                .await;
                return;
            }};
        }

        let Some(peer_user_channel_id) = payload
            .user_channel_id
            .as_deref()
            .filter(|value| stable_channels::trade::is_user_channel_id(value))
        else {
            reject_proposal!(TradeRejectionReason::InvalidConfirmation);
        };
        if amount_msat != Some(1)
            || payload.channel_id.as_deref() != Some(chan.channel_id.as_str())
            || payload.proposal_payment_id.is_some()
            || payload.proposal_hash.is_some()
            || payload.confirmation_id.is_some()
            || payload.fee_msat.is_some()
        {
            reject_proposal!(TradeRejectionReason::InvalidConfirmation);
        }
        let Some(base_sync_version) = payload.base_sync_version else {
            reject_proposal!(TradeRejectionReason::StaleState);
        };
        if payload
            .replaces_trade_id
            .as_deref()
            .is_some_and(|value| !stable_channels::trade::is_trade_id(value))
        {
            reject_proposal!(TradeRejectionReason::InvalidConfirmation);
        }
        let now = Self::unix_time_secs();
        if payload.ts == 0
            || now < 0
            || (now as u64).abs_diff(payload.ts) > 300
            || !payload.expected_usd.is_finite()
            || payload.expected_usd < 0.0
        {
            reject_proposal!(TradeRejectionReason::StaleRequest);
        }
        if let Ok(Some(active_trade_id)) = self.db.active_trade_reservation(&chan.channel_id, now) {
            if payload.replaces_trade_id.as_deref() != Some(active_trade_id.as_str()) {
                reject_proposal!(TradeRejectionReason::ChannelBusy);
            }
        } else if payload.replaces_trade_id.is_some() {
            reject_proposal!(TradeRejectionReason::StaleState);
        }
        let Some(target_uid) = parse_user_channel_id(&chan.user_channel_id) else {
            reject_proposal!(TradeRejectionReason::InternalFailure);
        };
        let Some(current) = self
            .stable_channels
            .iter()
            .find(|channel| channel.user_channel_id == target_uid)
            .cloned()
        else {
            reject_proposal!(TradeRejectionReason::InternalFailure);
        };
        let current_version = match self.db.candidate_sync_version(&chan.user_channel_id) {
            Ok(candidate) => candidate.saturating_sub(1),
            Err(_) => reject_proposal!(TradeRejectionReason::InternalFailure),
        };
        if base_sync_version != current_version {
            reject_proposal!(TradeRejectionReason::StaleState);
        }
        match self.db.load_channel(&chan.user_channel_id) {
            Ok(Some(durable))
                if stable_channels::trade::target_matches(
                    durable.expected_usd,
                    current.expected_usd.0,
                ) && durable.backing_sats == current.backing_sats => {}
            Ok(Some(_)) => reject_proposal!(TradeRejectionReason::StaleState),
            _ => reject_proposal!(TradeRejectionReason::InternalFailure),
        }
        let new_expected =
            stable_channels::stable::normalize_trade_expected_usd(payload.expected_usd);
        if stable_channels::trade::target_matches(current.expected_usd.0, new_expected) {
            reject_proposal!(TradeRejectionReason::InvalidAmount);
        }
        let Some(wallet_quote) = payload.quote_price else {
            reject_proposal!(TradeRejectionReason::InvalidQuote);
        };
        if !wallet_quote.is_finite()
            || wallet_quote <= 0.0
            || !btc_price.is_finite()
            || btc_price <= 0.0
        {
            reject_proposal!(TradeRejectionReason::InvalidQuote);
        }
        let quote_deviation_percent = ((wallet_quote - btc_price) / btc_price * 100.0).abs();
        if quote_deviation_percent > MAX_TRADE_QUOTE_DEVIATION_PERCENT {
            reject_proposal!(TradeRejectionReason::QuoteDeviation);
        }
        let Some(fee_msat) =
            expected_trade_fee_msat(current.expected_usd.0, new_expected, btc_price)
        else {
            reject_proposal!(TradeRejectionReason::InvalidFee);
        };
        let (our_sats, their_sats) = channel_peer_balances(chan);
        let projected_receiver_sats = their_sats.saturating_sub(fee_msat / 1000);
        let receiver_usd =
            USD::from_bitcoin(Bitcoin::from_sats(projected_receiver_sats), btc_price).0;
        if new_expected > receiver_usd {
            reject_proposal!(TradeRejectionReason::InsufficientCapacity);
        }
        let mut reserved = current.clone();
        reserved.stable_provider_btc = Bitcoin::from_sats(our_sats.saturating_add(fee_msat / 1000));
        reserved.stable_receiver_btc = Bitcoin::from_sats(projected_receiver_sats);
        reserved.stable_provider_usd = USD::from_bitcoin(reserved.stable_provider_btc, btc_price);
        reserved.stable_receiver_usd = USD::from_bitcoin(reserved.stable_receiver_btc, btc_price);
        reserved.latest_price = btc_price;
        if !stable_channels::stable::apply_trade(&mut reserved, new_expected, btc_price) {
            let reason = if new_expected < current.expected_usd.0
                && (new_expected == 0.0
                    || trade_reduction_exhausts_backing(
                        current.backing_sats,
                        current.expected_usd.0,
                        new_expected,
                        btc_price,
                    )) {
                TradeRejectionReason::SettlementRequired
            } else {
                TradeRejectionReason::UnsafeAllocation
            };
            reject_proposal!(reason);
        }
        let native_sats = projected_receiver_sats.saturating_sub(reserved.backing_sats);
        let confirmation_id = hex::encode(rand::random::<[u8; 32]>());
        let confirmed_at = now.max(0) as u64;
        let expires_at =
            confirmed_at.saturating_add(stable_channels::constants::TRADE_CONFIRMATION_TTL_SECS);
        let confirmation_payload = crate::messages::build_confirm_trade_payload(
            &chan.channel_id,
            peer_user_channel_id,
            trade_id,
            proposal_payment_id,
            &proposal_hash,
            &confirmation_id,
            reserved.expected_usd.0,
            btc_price,
            fee_msat,
            base_sync_version,
            confirmed_at,
            expires_at,
        );
        let signature = match ldk
            .sign_message(SignMessageRequest {
                message: confirmation_payload.as_bytes().to_vec().into(),
            })
            .await
        {
            Ok(response) => response.signature,
            Err(_) => reject_proposal!(TradeRejectionReason::InternalFailure),
        };
        let confirmation_envelope =
            crate::messages::build_envelope(confirmation_payload, signature);
        match self.db.reserve_trade_proposal(
            proposal_payment_id,
            trade_id,
            &proposal_hash,
            &chan.channel_id,
            &chan.user_channel_id,
            peer_user_channel_id,
            &chan.counterparty_node_id,
            &confirmation_id,
            reserved.expected_usd.0,
            reserved.backing_sats,
            native_sats,
            btc_price,
            fee_msat,
            base_sync_version,
            now,
            expires_at.min(i64::MAX as u64) as i64,
            payload.replaces_trade_id.as_deref(),
            &confirmation_envelope,
        ) {
            Ok(true) => stable_channels::audit::audit_event(
                "TRADE_RESERVED",
                serde_json::json!({
                    "trade_id": trade_id,
                    "proposal_payment_id": proposal_payment_id,
                    "proposal_hash": proposal_hash,
                    "confirmation_id": confirmation_id,
                    "fee_msat": fee_msat,
                    "expires_at": expires_at,
                }),
            ),
            Ok(false) => reject_proposal!(TradeRejectionReason::StaleState),
            Err(_) => reject_proposal!(TradeRejectionReason::InternalFailure),
        }
    }

    async fn handle_trade_execution(
        &mut self,
        payload: &crate::messages::TradePayload,
        envelope: &crate::messages::SignedEnvelope,
        chan: &Channel,
        inbound_payment_id: Option<&str>,
        amount_msat: Option<u64>,
        settled_at: Option<u64>,
        ldk: &dyn LdkServerCalls,
    ) {
        let Some(trade_id) = payload
            .trade_id
            .as_deref()
            .filter(|value| stable_channels::trade::is_trade_id(value))
        else {
            return;
        };
        let Some(execution_payment_id) =
            inbound_payment_id.filter(|value| stable_channels::trade::is_payment_id(value))
        else {
            return;
        };
        let execution_hash = stable_channels::trade::request_hash(envelope.payload.as_bytes());
        let Ok(Some(reservation)) = self.db.trade_reservation_by_trade_id(trade_id) else {
            self.reject_correlated_trade(
                ldk,
                execution_payment_id,
                trade_id,
                &execution_hash,
                &chan.channel_id,
                &chan.user_channel_id,
                &chan.counterparty_node_id,
                TradeRejectionReason::InvalidConfirmation,
            )
            .await;
            return;
        };
        let settled_at = settled_at
            .unwrap_or_else(|| Self::unix_time_secs().max(0) as u64)
            .min(i64::MAX as u64) as i64;
        if reservation.outcome == "accepted"
            && reservation.execution_payment_id.as_deref() == Some(execution_payment_id)
            && reservation.execution_hash.as_deref() == Some(execution_hash.as_str())
        {
            let _ = self.db.requeue_exact_trade_response(
                &reservation.proposal_payment_id,
                trade_id,
                &execution_hash,
                Self::unix_time_secs(),
            );
            return;
        }
        if !matches!(reservation.outcome.as_str(), "reserved" | "expired") {
            return;
        }
        let valid = amount_msat == Some(reservation.fee_msat)
            && payload.fee_msat == Some(reservation.fee_msat)
            && payload.channel_id.as_deref() == Some(reservation.channel_id.as_str())
            && payload.user_channel_id.as_deref()
                == Some(reservation.peer_user_channel_id.as_str())
            && payload.proposal_payment_id.as_deref()
                == Some(reservation.proposal_payment_id.as_str())
            && payload.proposal_hash.as_deref() == Some(reservation.proposal_hash.as_str())
            && payload.confirmation_id.as_deref() == Some(reservation.confirmation_id.as_str())
            && stable_channels::trade::target_matches(
                payload.expected_usd,
                reservation.expected_usd,
            )
            && payload
                .quote_price
                .is_some_and(|price| price.to_bits() == reservation.quote_price.to_bits())
            && payload.ts != 0;
        if !valid {
            self.reject_reserved_trade(
                ldk,
                trade_id,
                execution_payment_id,
                &execution_hash,
                &reservation.channel_id,
                TradeRejectionReason::InvalidConfirmation,
            )
            .await;
            return;
        }
        if settled_at > reservation.expires_at {
            self.reject_reserved_trade(
                ldk,
                trade_id,
                execution_payment_id,
                &execution_hash,
                &reservation.channel_id,
                TradeRejectionReason::ConfirmationExpired,
            )
            .await;
            return;
        }
        let acceptance_payload = crate::messages::build_trade_sync_payload(
            &reservation.channel_id,
            &reservation.peer_user_channel_id,
            reservation.expected_usd,
            reservation.backing_sats,
            reservation.sync_version,
            trade_id,
            execution_payment_id,
            &execution_hash,
        );
        let signature = match ldk
            .sign_message(SignMessageRequest {
                message: acceptance_payload.as_bytes().to_vec().into(),
            })
            .await
        {
            Ok(response) => response.signature,
            Err(_) => {
                self.reject_reserved_trade(
                    ldk,
                    trade_id,
                    execution_payment_id,
                    &execution_hash,
                    &reservation.channel_id,
                    TradeRejectionReason::InternalFailure,
                )
                .await;
                return;
            }
        };
        let response_envelope = crate::messages::build_envelope(acceptance_payload, signature);
        let decided_at = Self::unix_time_secs();
        match self.db.execute_trade_reservation(
            trade_id,
            &reservation.confirmation_id,
            execution_payment_id,
            &execution_hash,
            settled_at,
            decided_at,
            &response_envelope,
        ) {
            Ok(true) => {
                let committed = self
                    .db
                    .load_channel(&reservation.user_channel_id)
                    .ok()
                    .flatten();
                if let Some(target_uid) = parse_user_channel_id(&reservation.user_channel_id) {
                    if let Some(in_memory) = self
                        .stable_channels
                        .iter_mut()
                        .find(|channel| channel.user_channel_id == target_uid)
                    {
                        let expected_usd = committed
                            .as_ref()
                            .map(|row| row.expected_usd)
                            .unwrap_or(reservation.expected_usd);
                        let backing_sats = committed
                            .as_ref()
                            .map(|row| row.backing_sats)
                            .unwrap_or(reservation.backing_sats);
                        let native_sats = committed
                            .as_ref()
                            .map(|row| row.native_sats)
                            .unwrap_or(reservation.native_sats);
                        in_memory.expected_usd = USD(expected_usd);
                        in_memory.backing_sats = backing_sats;
                        in_memory.native_sats = native_sats;
                        in_memory.native_channel_btc = Bitcoin::from_sats(native_sats);
                    }
                }
                stable_channels::audit::audit_event(
                    "TRADE_ACCEPTED",
                    serde_json::json!({
                        "trade_id": trade_id,
                        "trade_payment_id": execution_payment_id,
                        "request_hash": execution_hash,
                        "expected_usd": reservation.expected_usd,
                        "backing_sats": reservation.backing_sats,
                        "sync_version": reservation.sync_version,
                    }),
                );
            }
            Ok(false) => {
                self.reject_reserved_trade(
                    ldk,
                    trade_id,
                    execution_payment_id,
                    &execution_hash,
                    &reservation.channel_id,
                    TradeRejectionReason::StaleState,
                )
                .await;
            }
            Err(_) => {
                self.reject_reserved_trade(
                    ldk,
                    trade_id,
                    execution_payment_id,
                    &execution_hash,
                    &reservation.channel_id,
                    TradeRejectionReason::InternalFailure,
                )
                .await;
            }
        }
    }

    async fn handle_trade_cancellation(
        &mut self,
        payload: &crate::messages::TradePayload,
        envelope: &crate::messages::SignedEnvelope,
        chan: &Channel,
        inbound_payment_id: Option<&str>,
        amount_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
    ) {
        let (Some(trade_id), Some(cancel_payment_id)) =
            (payload.trade_id.as_deref(), inbound_payment_id)
        else {
            return;
        };
        if amount_msat != Some(1)
            || !stable_channels::trade::is_trade_id(trade_id)
            || !stable_channels::trade::is_payment_id(cancel_payment_id)
            || payload
                .confirmation_id
                .as_deref()
                .is_some_and(|value| !stable_channels::trade::is_confirmation_id(value))
        {
            return;
        }
        let Ok(Some(reservation)) = self.db.trade_reservation_by_trade_id(trade_id) else {
            return;
        };
        if reservation.outcome != "reserved"
            || reservation.channel_id != chan.channel_id
            || payload
                .confirmation_id
                .as_deref()
                .is_some_and(|value| reservation.confirmation_id != value)
            || payload.proposal_payment_id.as_deref()
                != Some(reservation.proposal_payment_id.as_str())
            || payload.proposal_hash.as_deref() != Some(reservation.proposal_hash.as_str())
        {
            return;
        }
        let request_hash = stable_channels::trade::request_hash(envelope.payload.as_bytes());
        self.reject_reserved_trade(
            ldk,
            trade_id,
            cancel_payment_id,
            &request_hash,
            &reservation.channel_id,
            TradeRejectionReason::ClientCancelled,
        )
        .await;
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
            status: format!(
                "Set expected_usd={} on channel {}",
                expected_usd_f, channel_id
            ),
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
                user_channel_id,
                e
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
    pub async fn reconcile_from_grpc(&mut self, ldk: &dyn LdkServerCalls, btc_price: f64) {
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
                        record.user_channel_id,
                        e
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
            self.handle_channel_ready_splice(target_uid, funding_txo.as_deref(), ldk, btc_price)
                .await;
            return;
        }

        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r.channels,
            Err(e) => {
                tracing::error!("[stable] handle_channel_ready: list_channels failed: {}", e);
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
                "funding_txo": funding_txo,
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
        self.handle_payment_received_at(
            custom_records,
            payment_id,
            amount_msat,
            None,
            ldk,
            btc_price,
        )
        .await;
    }

    pub async fn handle_payment_received_at(
        &mut self,
        custom_records: Vec<CustomTlvRecord>,
        payment_id: Option<String>,
        amount_msat: Option<u64>,
        settled_at: Option<u64>,
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
                self.handle_trade_payment_at(
                    &raw,
                    payment_id.as_deref(),
                    amount_msat,
                    settled_at,
                    ldk,
                    btc_price,
                )
                .await;
            } else if rec.value.as_ref() == [1u8] {
                if let Some(pid) = payment_id.as_deref() {
                    if let Err(e) = self.db.record_settlement(pid, "stability") {
                        tracing::error!(
                            "[stable] record_settlement (inbound stability) failed: {}",
                            e
                        );
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
                self.reconcile_incoming_stability(
                    payment_id.as_deref(),
                    amount_msat,
                    ldk,
                    btc_price,
                )
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
        if record.value.len() > stable_channels::constants::MAX_SIGNED_STABILITY_TLV_VALUE_BYTES {
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
        if payload.expected_usd.to_bits() != self.stable_channels[idx].expected_usd.0.to_bits() {
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
        let Some(mut backing_after) = stable_channels::stable::backing_after_user_to_lsp_stability(
            backing_before,
            allocation_expected_usd,
            btc_price,
            amount_sats,
            their_sats,
        ) else {
            invalidate("allocation");
            return;
        };
        let mut native_after = their_sats.saturating_sub(backing_after);
        let amount_usd =
            amount_sats as f64 / stable_channels::constants::SATS_IN_BTC as f64 * btc_price;
        let persist = |backing_sats_before, backing_sats_after, native_sats_after| {
            self.db
                .record_signed_stability_payment_and_update_allocation(
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
            Err(ref error) if stable_channels::db::is_stale_inbound_stability_allocation(error) => {
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

    async fn retry_pending_signed_stability(&mut self, ldk: &dyn LdkServerCalls, btc_price: f64) {
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
            if sc.expected_usd.0 < 0.01 {
                continue;
            }
            let Some(c) = channels
                .iter()
                .find(|c| parse_user_channel_id(&c.user_channel_id) == Some(sc.user_channel_id))
            else {
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
        let _ = self.db.expire_trade_reservations(now);

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
            let Some(c) = by_user_channel_id.get(&sc.user_channel_id) else {
                continue;
            };
            // The confirmed allocation and fee are locked for the short review window.
            let reservation = self.db.active_trade_reservation(&c.channel_id, now);
            if !stability_mutation_allowed(&reservation) {
                if let Err(error) = reservation {
                    stable_channels::audit::audit_event(
                        "DB_READ_FAILED",
                        serde_json::json!({
                            "op": "active_trade_reservation",
                            "context": "stability_tick",
                            "channel_id": c.channel_id,
                            "error": error.to_string(),
                        }),
                    );
                }
                continue;
            }

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

            if percent_from_par < percent_threshold || dollars_from_par < dollar_threshold {
                continue;
            }
            if sc.risk_level > stable_channels::constants::MAX_RISK_LEVEL {
                let (lo, lv) = self
                    .stability_throttle
                    .get(&sc.user_channel_id)
                    .cloned()
                    .unwrap_or_default();
                if stability_should_log(
                    &lo,
                    "high_risk",
                    lv,
                    stable_usd_value,
                    target,
                    dollar_threshold,
                    percent_threshold,
                    false,
                ) {
                    stable_channels::audit::audit_event(
                        "STABILITY_SKIP_HIGH_RISK",
                        serde_json::json!({
                            "channel_id": sc.channel_id.to_string(),
                            "user_channel_id": format!("{}", sc.user_channel_id),
                            "risk_level": sc.risk_level,
                        }),
                    );
                    self.stability_throttle.insert(
                        sc.user_channel_id,
                        ("high_risk".to_string(), stable_usd_value),
                    );
                }
                continue;
            }
            if now - sc.last_stability_payment < cooldown {
                let (lo, lv) = self
                    .stability_throttle
                    .get(&sc.user_channel_id)
                    .cloned()
                    .unwrap_or_default();
                if stability_should_log(
                    &lo,
                    "cooldown",
                    lv,
                    stable_usd_value,
                    target,
                    dollar_threshold,
                    percent_threshold,
                    false,
                ) {
                    stable_channels::audit::audit_event(
                        "STABILITY_COOLDOWN",
                        serde_json::json!({
                            "channel_id": sc.channel_id.to_string(),
                            "user_channel_id": format!("{}", sc.user_channel_id),
                            "seconds_since_last": now - sc.last_stability_payment,
                            "cooldown_secs": cooldown,
                        }),
                    );
                    self.stability_throttle.insert(
                        sc.user_channel_id,
                        ("cooldown".to_string(), stable_usd_value),
                    );
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
                        payload, signature,
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
                    let backing_after = ((sc.expected_usd.0 / btc_price) * 100_000_000.0) as u64;
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
                                    }
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
                                    }
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
                                (
                                    if persisted {
                                        "payment_sent"
                                    } else {
                                        "payment_persist_failed"
                                    }
                                    .to_string(),
                                    stable_usd_value,
                                ),
                            );
                        }
                        Err(e) => {
                            tracing::warn!("[stable] run_tick: spontaneous_send failed: {}", e);
                            stable_channels::audit::audit_event(
                                "STABILITY_PAYMENT_FAILED",
                                serde_json::json!({
                                    "channel_id": channel_id_clone,
                                    "user_channel_id": user_channel_id_clone.clone(),
                                    "direction": direction,
                                    "error": e.to_string(),
                                }),
                            );
                            self.stability_throttle.insert(
                                sc.user_channel_id,
                                ("payment_failed".to_string(), stable_usd_value),
                            );
                            // Do not bump last_stability_payment so retry can fire.
                        }
                    }
                } else {
                    // User above par: CHECK_ONLY. The LSP can only push value, not pull, so do nothing here (no cooldown bump).
                    let (lo, lv) = self
                        .stability_throttle
                        .get(&sc.user_channel_id)
                        .cloned()
                        .unwrap_or_default();
                    if stability_should_log(
                        &lo,
                        "check_only",
                        lv,
                        stable_usd_value,
                        target,
                        dollar_threshold,
                        percent_threshold,
                        true,
                    ) {
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
                        self.stability_throttle.insert(
                            sc.user_channel_id,
                            ("check_only".to_string(), stable_usd_value),
                        );
                    }
                }
            } else {
                let mut p = push.lock().await;
                p.notify(&sc.counterparty.to_string(), direction);
                drop(p);
                let key = format!("push_queued:{}", direction);
                let (lo, lv) = self
                    .stability_throttle
                    .get(&sc.user_channel_id)
                    .cloned()
                    .unwrap_or_default();
                if stability_should_log(
                    &lo,
                    &key,
                    lv,
                    stable_usd_value,
                    target,
                    dollar_threshold,
                    percent_threshold,
                    true,
                ) {
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
                    self.stability_throttle
                        .insert(sc.user_channel_id, (key, stable_usd_value));
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
                        tracing::error!("[stable] record_settlement (outbound sync) failed: {}", e);
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
            }
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
        let forward_detail = serde_json::json!({
            "prev_user_channel_id": prev_user_channel_id,
            "next_user_channel_id": next_user_channel_id,
            "prev_channel_id": prev_channel_id,
            "next_channel_id": next_channel_id,
            "prev_node_id": prev_node_id,
            "next_node_id": next_node_id,
            "forwarded_msat": outbound_amount_forwarded_msat,
            "fee_msat": fee_msat,
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
        funding_txo: Option<&str>,
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
            let before_receiver_sats = sc.stable_receiver_btc.sats;
            let (splice_direction, splice_amount_sats) =
                splice_balance_change(before_receiver_sats, their_sats);
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
                splice_direction,
                splice_amount_sats,
                before_receiver_sats,
            )
        };

        let (
            ucid_str,
            expected_usd_f,
            backing,
            native,
            note,
            counterparty_hex,
            deducted,
            splice_direction,
            splice_amount_sats,
            before_receiver_sats,
        ) = persisted;
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
                "deducted": deducted,
            }),
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
    pub async fn handle_trade_payment(
        &mut self,
        raw: &str,
        inbound_payment_id: Option<&str>,
        amount_msat: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        self.handle_trade_payment_at(raw, inbound_payment_id, amount_msat, None, ldk, btc_price)
            .await;
    }

    async fn handle_trade_payment_at(
        &mut self,
        raw: &str,
        inbound_payment_id: Option<&str>,
        amount_msat: Option<u64>,
        settled_at: Option<u64>,
        ldk: &dyn LdkServerCalls,
        btc_price: f64,
    ) {
        let Some(envelope) = crate::messages::parse_envelope(raw) else {
            stable_channels::audit::audit_event("TRADE_PARSE_SIGNED_FAILED", serde_json::json!({}));
            return;
        };
        let Some(payload) = crate::messages::parse_trade_payload(&envelope.payload) else {
            stable_channels::audit::audit_event(
                "TRADE_PARSE_PAYLOAD_FAILED",
                serde_json::json!({}),
            );
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
        let chan = channels
            .into_iter()
            .find(|c| match payload.channel_id.as_deref() {
                Some(channel_id) => c.channel_id == channel_id,
                None => payload
                    .user_channel_id
                    .as_deref()
                    .is_some_and(|user_channel_id| {
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

        if let Some(phase) = payload.phase.as_deref() {
            match phase {
                "propose" => {
                    self.handle_trade_proposal(
                        &payload,
                        &envelope,
                        &chan,
                        inbound_payment_id,
                        amount_msat,
                        ldk,
                        btc_price,
                    )
                    .await;
                }
                "execute" => {
                    self.handle_trade_execution(
                        &payload,
                        &envelope,
                        &chan,
                        inbound_payment_id,
                        amount_msat,
                        settled_at,
                        ldk,
                    )
                    .await;
                }
                "cancel" => {
                    self.handle_trade_cancellation(
                        &payload,
                        &envelope,
                        &chan,
                        inbound_payment_id,
                        amount_msat,
                        ldk,
                    )
                    .await;
                }
                _ => stable_channels::audit::audit_event(
                    "TRADE_PHASE_INVALID",
                    serde_json::json!({ "phase": phase, "channel_id": chan.channel_id }),
                ),
            }
            return;
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
            let Some(expected_fee_msat) =
                expected_trade_fee_msat(current.expected_usd.0, new_expected, quote_price)
            else {
                reject_correlated!(TradeRejectionReason::InvalidFee);
            };
            let tolerance_msat = trade_fee_tolerance_msat(expected_fee_msat, true);
            if received_msat.abs_diff(expected_fee_msat) > tolerance_msat {
                reject_correlated!(TradeRejectionReason::InvalidFee);
            }
            let quote_deviation_percent = ((quote_price - btc_price) / btc_price * 100.0).abs();
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
            updated.stable_provider_usd = USD::from_bitcoin(updated.stable_provider_btc, btc_price);
            updated.stable_receiver_usd = USD::from_bitcoin(updated.stable_receiver_btc, btc_price);
            updated.latest_price = btc_price;
            if !stable_channels::stable::apply_trade(&mut updated, new_expected, btc_price) {
                let reason = if new_expected < current.expected_usd.0
                    && (new_expected == 0.0
                        || trade_reduction_exhausts_backing(
                            current.backing_sats,
                            current.expected_usd.0,
                            new_expected,
                            btc_price,
                        )) {
                    TradeRejectionReason::SettlementRequired
                } else {
                    TradeRejectionReason::UnsafeAllocation
                };
                reject_correlated!(reason);
            }
            let sync_version = match self.db.candidate_sync_version(&format!("{}", target_uid)) {
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
            let response_envelope = crate::messages::build_envelope(acceptance_payload, signature);
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
            stable_channels::audit::audit_event(
                "TRADE_CHANNEL_UID_UNPARSEABLE",
                serde_json::json!({ "channel_id": chan.channel_id.clone(), "user_channel_id": chan.user_channel_id.clone() }),
            );
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
        let Some(expected_fee_msat) =
            expected_trade_fee_msat(current_expected_usd, new_expected, fee_price)
        else {
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
                let quote_deviation_percent = ((quote_price - btc_price) / btc_price * 100.0).abs();
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
    last_outcome: &str,
    outcome: &str,
    last_value: f64,
    value: f64,
    target: f64,
    usd_threshold: f64,
    pct_threshold: f64,
    track_value: bool,
) -> bool {
    if last_outcome != outcome {
        return true;
    }
    if !track_value {
        return false;
    }
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
        Channel as GrpcChannel, ForwardedPayment as GrpcForwardedPayment, HtlcLocator, PageToken,
        Payment as GrpcPayment, PaymentStatus, Peer as GrpcPeer,
        PendingSweepBalance as GrpcPendingSweepBalance,
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
        pub forward_next_page_token: StdMutex<Option<PageToken>>,
        pub forward_calls: AtomicUsize,
        pub sweeps: StdMutex<Vec<GrpcPendingSweepBalance>>,
        pub peers: StdMutex<Vec<GrpcPeer>>,
        pub payments: StdMutex<Vec<GrpcPayment>>,
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
        pub fn with_forwarded(self, f: Vec<GrpcForwardedPayment>) -> Self {
            *self.forwarded.lock().unwrap() = f;
            self
        }
        pub fn with_forward_cursor(self, token: PageToken) -> Self {
            *self.forward_next_page_token.lock().unwrap() = Some(token);
            self
        }
        pub fn with_sweeps(self, s: Vec<GrpcPendingSweepBalance>) -> Self {
            *self.sweeps.lock().unwrap() = s;
            self
        }
        pub fn with_peers(self, p: Vec<GrpcPeer>) -> Self {
            *self.peers.lock().unwrap() = p;
            self
        }
        pub fn with_payments(self, p: Vec<GrpcPayment>) -> Self {
            *self.payments.lock().unwrap() = p;
            self
        }
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
            req: VerifySignatureRequest,
        ) -> Result<VerifySignatureResponse, LdkServerError> {
            self.verify_calls.lock().unwrap().push(req);
            Ok(VerifySignatureResponse {
                valid: self.verify_should_pass,
            })
        }
        async fn list_forwarded_payments(
            &self,
            _req: ListForwardedPaymentsRequest,
        ) -> Result<ListForwardedPaymentsResponse, LdkServerError> {
            self.forward_calls.fetch_add(1, Ordering::SeqCst);
            Ok(ListForwardedPaymentsResponse {
                forwarded_payments: self.forwarded.lock().unwrap().clone(),
                next_page_token: self.forward_next_page_token.lock().unwrap().clone(),
            })
        }
        async fn get_balances(
            &self,
            _req: GetBalancesRequest,
        ) -> Result<GetBalancesResponse, LdkServerError> {
            Ok(GetBalancesResponse {
                pending_balances_from_channel_closures: self.sweeps.lock().unwrap().clone(),
                ..Default::default()
            })
        }
        async fn list_peers(
            &self,
            _req: ListPeersRequest,
        ) -> Result<ListPeersResponse, LdkServerError> {
            Ok(ListPeersResponse {
                peers: self.peers.lock().unwrap().clone(),
            })
        }
        async fn list_payments(
            &self,
            _req: ListPaymentsRequest,
        ) -> Result<ListPaymentsResponse, LdkServerError> {
            Ok(ListPaymentsResponse {
                payments: self.payments.lock().unwrap().clone(),
                next_page_token: None,
            })
        }
        async fn get_payment_details(
            &self,
            req: GetPaymentDetailsRequest,
        ) -> Result<GetPaymentDetailsResponse, LdkServerError> {
            Ok(GetPaymentDetailsResponse {
                payment: self
                    .payments
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|payment| payment.id == req.payment_id)
                    .cloned(),
            })
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
        let fwd = fake
            .list_forwarded_payments(ListForwardedPaymentsRequest { page_token: None })
            .await
            .unwrap()
            .forwarded_payments;
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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(10.0),
            Some("note".to_string()),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels.len(), 1);

        mgr.handle_channel_closed(
            "".to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            None,
            None,
            0,
            None,
        );
        assert_eq!(mgr.stable_channels.len(), 0);
    }

    #[tokio::test]
    async fn reconcile_drops_channels_no_longer_on_server() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
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
        assert_eq!(mgr.stable_channels.len(), 1);

        // LDK Server no longer reports the channel.
        let empty_server = FakeLdkServer::new(vec![]);
        mgr.reconcile_from_grpc(&empty_server as &dyn LdkServerCalls, 100_000.0)
            .await;
        assert_eq!(mgr.stable_channels.len(), 0);
    }

    #[tokio::test]
    async fn reconcile_refreshes_known_channel() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
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

        // Same channel, different balance: outbound drops from 50_000 to 30_000 sats.
        let fake2 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            30_000_000,
            true,
        )]);
        mgr.reconcile_from_grpc(&fake2 as &dyn LdkServerCalls, 100_000.0)
            .await;
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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.reconcile_from_grpc(&fake as &dyn LdkServerCalls, 100_000.0)
            .await;

        assert_eq!(
            mgr.stable_channels.len(),
            1,
            "channel must be hydrated from db"
        );
        let sc = &mgr.stable_channels[0];
        assert_eq!(sc.expected_usd.0, 25.0, "persisted expected_usd preserved");
        assert_eq!(sc.backing_sats, 40_000, "persisted backing_sats preserved");
        assert_eq!(
            sc.note.as_deref(),
            Some("persisted"),
            "persisted note preserved"
        );
        assert_eq!(
            sc.counterparty.to_string(),
            COUNTERPARTY_HEX,
            "counterparty resolved from live channel"
        );
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
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_HEX,
                25.0,
                40_000,
                10_000,
                Some("persisted"),
            )
            .unwrap();
        assert_eq!(mgr.stable_channels.len(), 0, "fresh manager starts empty");

        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        // Empty vec -> self-heal repopulates from truth.
        mgr.reconcile_if_empty(&fake as &dyn LdkServerCalls, 100_000.0)
            .await;
        assert_eq!(mgr.stable_channels.len(), 1, "empty list is hydrated");

        // Populated vec -> guard skips reconcile, so a transient empty snapshot can't wipe it.
        let empty_server = FakeLdkServer::new(vec![]);
        mgr.reconcile_if_empty(&empty_server as &dyn LdkServerCalls, 100_000.0)
            .await;
        assert_eq!(
            mgr.stable_channels.len(),
            1,
            "populated list is left untouched"
        );
    }

    #[tokio::test]
    async fn handle_channel_ready_auto_registers_new_channel() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels.len(), 1);
        assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
    }

    #[tokio::test]
    async fn handle_channel_ready_is_idempotent() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        assert_eq!(mgr.stable_channels.len(), 1);
    }

    #[tokio::test]
    async fn payment_received_trade_tlv_applies() {
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
        seed_channel(
            &mut mgr,
            1u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            5.0,
            5_000,
            45_000,
            50_000,
            100_000.0,
        );

        mgr.handle_payment_received(vec![], None, None, &fake as &dyn LdkServerCalls, 100_000.0)
            .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 5.0).abs() < 1e-6); // untouched
        assert!(mgr.db.list_settlements().unwrap().is_empty());
    }

    #[tokio::test]
    async fn payment_received_marker_records_settlement() {
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

        let records = vec![CustomTlvRecord {
            type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
            value: vec![1u8].into(),
        }];
        let before = mgr.stable_channels[0].expected_usd.0;
        mgr.handle_payment_received(
            records,
            Some("pay_settlement_1".to_string()),
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
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
        )
        .await;

        // 5_000 overflow sats * $100k / 1e8 = $5.00 deducted: $10 -> $5.
        let exp = mgr.stable_channels[0].expected_usd.0;
        assert!(
            (exp - 5.0).abs() < 0.01,
            "expected_usd should drop to ~5.0, got {}",
            exp
        );
        // native_sats and native_channel_btc must agree after reconcile.
        assert_eq!(
            mgr.stable_channels[0].native_channel_btc.sats, mgr.stable_channels[0].native_sats,
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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            70_000_000,
            true,
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
        )
        .await;

        let exp = mgr.stable_channels[0].expected_usd.0;
        assert!(
            (exp - 10.0).abs() < 0.01,
            "expected_usd must stay ~10.0, got {}",
            exp
        );
        // Native buffer shrank by the spend: 40_000 - 20_000 = 20_000.
        assert_eq!(mgr.stable_channels[0].native_sats, 20_000);
        // native_sats and native_channel_btc must agree after reconcile.
        assert_eq!(
            mgr.stable_channels[0].native_channel_btc.sats, mgr.stable_channels[0].native_sats,
            "native_channel_btc must match native_sats after a forward",
        );
    }

    #[tokio::test]
    async fn handle_payment_forwarded_untracked_channel_is_noop() {
        let mut mgr = make_manager();
        // Untracked channel: a forward on an unknown channel must not panic or invent a record.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
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
        )
        .await;
        assert!(mgr.stable_channels.is_empty());
    }

    #[tokio::test]
    async fn forwarded_deduction_sends_sync() {
        let mut mgr = make_manager();
        // Post-forward channel snapshot: their = 5,000 sats (our 95k via outbound 95M msat).
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )]);
        // expected $10 -> backing 10,000; native 40,000; receiver 50,000 at $100k.
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            10.0,
            10_000,
            40_000,
            50_000,
            100_000.0,
        );

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
        assert_eq!(
            sends.len(),
            1,
            "a SYNC should be sent after a stable deduction"
        );
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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
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
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
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
        assert_eq!(
            data.detail["prev_user_channel_id"], USER_CHANNEL_ID_DECIMAL,
            "inbound leg must be recorded"
        );
        assert_eq!(
            data.detail["next_user_channel_id"], "outbound-ucid",
            "outbound leg must be recorded"
        );
        assert_eq!(data.detail["prev_node_id"], "prev-node-pubkey");
        assert_eq!(data.detail["next_node_id"], "next-node-pubkey");
    }

    #[tokio::test]
    async fn run_tick_skips_zero_target() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        // expected_usd defaulted to 0; tick must not attempt any send.
        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_HEX.to_string(),
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn run_tick_skips_cooldown_active() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
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
        // Pretend we just paid: bump last_stability_payment to "now".
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        mgr.stable_channels[0].last_stability_payment = now;

        // Force a large drift by swapping in a channel with no outbound capacity.
        let fake2 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            0,
            true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake2 as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
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
            &fake_initial as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        // Price drops 20% to 80_000 (receiver USD below 50), peer connected.
        let fake_drift = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));

        mgr.run_tick(&fake_drift as &dyn LdkServerCalls, &push, 80_000.0)
            .await;

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
        assert!(
            mgr.stable_channels[0].last_stability_payment > 0,
            "cooldown timestamp should be set"
        );
    }

    #[tokio::test]
    async fn run_tick_send_failure_keeps_cooldown_unset() {
        let mut mgr = make_manager();
        let fake_initial = FakeLdkServer::new(vec![make_channel(
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
            &fake_initial as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let fake_drift = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )])
        .with_send_failure();
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));

        mgr.run_tick(&fake_drift as &dyn LdkServerCalls, &push, 80_000.0)
            .await;

        assert_eq!(
            mgr.stable_channels[0].last_stability_payment, 0,
            "failed send must not start cooldown"
        );
    }

    #[tokio::test]
    async fn run_tick_pushes_when_offline_and_drift_exceeds_threshold() {
        let mut mgr = make_manager();
        let fake_initial = FakeLdkServer::new(vec![make_channel(
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
            &fake_initial as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        // Peer disconnected: is_usable=false.
        let fake_offline = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            false,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));

        mgr.run_tick(&fake_offline as &dyn LdkServerCalls, &push, 80_000.0)
            .await;

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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        // expected_usd=50 at price 100k -> backing_sats = 50_000
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(50.0),
            None,
            &fake0 as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        // Price RISES to 120k: stable_usd_value = 50_000/1e8*120k = $60 > $50 target -> user_to_lsp.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 120_000.0)
            .await;
        assert!(
            fake.sends.lock().unwrap().is_empty(),
            "LSP must NOT send when user is above par (CHECK_ONLY)"
        );
    }

    #[tokio::test]
    async fn run_tick_resets_backing_to_equilibrium_after_send() {
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        // expected_usd=50 at price 100k -> backing_sats = 50_000
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(50.0),
            None,
            &fake0 as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        // Price DROPS to 80k: stable_usd_value = 50_000/1e8*80k = $40 < $50 -> lsp_to_user -> send.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 80_000.0)
            .await;

        assert_eq!(
            fake.sends.lock().unwrap().len(),
            1,
            "should send in lsp_to_user direction"
        );
        // backing reset to target/price = 50/80000*1e8 = 62_500 (NOT left at stale 50_000).
        assert_eq!(
            mgr.stable_channels[0].backing_sats, 62_500,
            "backing must reset to equilibrium, preventing oscillation"
        );
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
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));

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
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            50.0,
            50_000,
            0,
            50_000,
            100_000.0,
        );
        mgr.stable_channels[0].risk_level = stable_channels::constants::MAX_RISK_LEVEL + 1;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        // Price drops 20% -> would normally pay lsp_to_user; high risk must skip.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 80_000.0)
            .await;
        assert!(
            fake.sends.lock().unwrap().is_empty(),
            "a channel above MAX_RISK_LEVEL must not trigger a stability send"
        );
    }

    #[tokio::test]
    async fn backstop_deducts_and_syncs_after_two_low_ticks() {
        let mut mgr = make_manager();
        // expected $10 -> backing 10_000; receiver 50_000 (native 40_000) at $100k.
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            10.0,
            10_000,
            40_000,
            50_000,
            100_000.0,
        );
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        // Live balance dropped to 5_000 (< backing 10_000): a spend the forwarded event missed.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )]);

        // Tick 1: debounce only.
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        assert!(
            (mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6,
            "tick 1 must not deduct"
        );
        assert!(
            fake.sends.lock().unwrap().is_empty(),
            "tick 1 must not SYNC"
        );

        // Tick 2: act.
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        let exp = mgr.stable_channels[0].expected_usd.0;
        assert!(
            (exp - 5.0).abs() < 0.01,
            "tick 2 must deduct ~$5 (10_000-5_000 sats), got {}",
            exp
        );
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "tick 2 must send exactly one SYNC");
        assert_eq!(
            sends[0].custom_tlvs.len(),
            1,
            "SYNC must carry exactly one stable TLV"
        );
        assert_eq!(
            sends[0].custom_tlvs[0].type_num,
            stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
            "SYNC TLV must be the stable-channel type",
        );
    }

    #[tokio::test]
    async fn backstop_single_tick_dip_does_not_deduct() {
        let mut mgr = make_manager();
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            10.0,
            10_000,
            40_000,
            50_000,
            100_000.0,
        );
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        // Tick 1: transient dip to 5_000 (in-flight outbound HTLC; outbound_capacity excludes it).
        let dip = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )]);
        mgr.run_tick(&dip as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        // Tick 2: balance restored to 50_000 (HTLC resolved without spending stable).
        let restored = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.run_tick(&restored as &dyn LdkServerCalls, &push, 100_000.0)
            .await;

        assert!(
            (mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6,
            "a transient dip must not deduct"
        );
        assert!(
            restored.sends.lock().unwrap().is_empty(),
            "no SYNC for a transient dip"
        );
    }

    #[tokio::test]
    async fn backstop_noop_when_balance_healthy() {
        let mut mgr = make_manager();
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            10.0,
            10_000,
            40_000,
            50_000,
            100_000.0,
        );
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        // Healthy: their 50_000 >= backing 10_000.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        assert!((mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6);
        assert!(
            fake.sends.lock().unwrap().is_empty(),
            "no backstop action when healthy"
        );
    }

    #[tokio::test]
    async fn reconcile_hydrates_channel_with_decimal_user_channel_id() {
        let mut mgr = make_manager();
        // Persist a row whose user_channel_id is the realistic decimal form.
        mgr.db
            .save_channel(
                CHANNEL_ID_HEX,
                USER_CHANNEL_ID_DECIMAL,
                12.0,
                30_000,
                5_000,
                Some("dec"),
            )
            .unwrap();

        // The live channel reports the SAME decimal user_channel_id (as real gRPC does).
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

        assert_eq!(
            mgr.stable_channels.len(),
            1,
            "decimal-id channel MUST hydrate (not be dropped)"
        );
        assert_eq!(mgr.stable_channels[0].expected_usd.0, 12.0);
        // The in-memory u128 must equal the decimal parse, not a hex misparse.
        assert_eq!(
            mgr.stable_channels[0].user_channel_id,
            189476124653200987495269098788434301048u128
        );
    }

    #[test]
    fn parse_user_channel_id_prefers_decimal() {
        assert_eq!(
            parse_user_channel_id("189476124653200987495269098788434301048"),
            Some(189476124653200987495269098788434301048u128)
        );
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
            .sign_message(SignMessageRequest {
                message: b"hello".to_vec().into(),
            })
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
        seed_channel(
            &mut mgr,
            1u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            50.0,
            0,
            0,
            50_000,
            100_000.0,
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        // Set last_stability_payment in the future so (now - future) < 0 <= cooldown, activating the gate even when cooldown_secs=0.
        mgr.stable_channels[0].last_stability_payment = now + 100;
        // Channel with 50k their side; price 80k -> drift 20% -> exceeds threshold -> hits cooldown gate.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 80_000.0)
            .await;
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let cd = events
            .iter()
            .find(|(e, _)| e == "STABILITY_COOLDOWN")
            .expect("STABILITY_COOLDOWN should be emitted on a cooldown-blocked tick");
        assert!(
            cd.1.get("user_channel_id").is_some(),
            "must carry user_channel_id"
        );
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

    async fn correlated_rejection_reason(
        manager: &mut StableChannelManager,
        fake: &FakeLdkServer,
        envelope: &str,
        payment_id: &str,
        amount_msat: u64,
        lsp_price: f64,
    ) -> TradeRejectionReason {
        let before = manager.stable_channels.first().map(|channel| {
            (
                channel.expected_usd.0,
                channel.backing_sats,
                channel.native_sats,
            )
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
            (
                channel.expected_usd.0,
                channel.backing_sats,
                channel.native_sats,
            )
        });
        assert_eq!(
            after, before,
            "rejection must not mutate in-memory allocation"
        );
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
        assert_eq!(expected_trade_fee_msat(50.0, 50.0, 100_000.0), Some(1));
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
        assert!(
            wallet_fee_msat.abs_diff(reconstructed)
                <= trade_fee_tolerance_msat(reconstructed, true)
        );
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

        mgr.handle_trade_payment(&envelope, Some(&payment_id), Some(fee), &fake, 100_000.0)
            .await;
        assert_eq!(mgr.stable_channels[0].expected_usd.0, 60.0);
        assert_eq!(
            mgr.db.get_sync_version(USER_CHANNEL_ID_DECIMAL).unwrap(),
            Some(1)
        );
        assert!(mgr
            .db
            .trade_decision_by_payment(&payment_id)
            .unwrap()
            .is_some());

        mgr.handle_trade_payment(&envelope, Some(&payment_id), Some(fee), &fake, 100_000.0)
            .await;
        assert_eq!(
            mgr.db.get_sync_version(USER_CHANNEL_ID_DECIMAL).unwrap(),
            Some(1)
        );
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
    async fn swept_two_phase_trade_uses_in_time_settlement_for_execution() {
        let (mut mgr, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let trade_id = "a1".repeat(32);
        let proposal_payment_id = "a2".repeat(32);
        let wallet_user_channel_id = "42";
        let proposal_payload = serde_json::json!({
            "type": "TRADE_V1",
            "phase": "propose",
            "channel_id": CHANNEL_ID_HEX,
            "user_channel_id": wallet_user_channel_id,
            "trade_id": trade_id,
            "expected_usd": 60.0,
            "quote_price": 100_000.0,
            "base_sync_version": 0,
            "ts": test_unix_now(),
        })
        .to_string();
        let proposal = serde_json::json!({
            "payload": proposal_payload,
            "signature": "wallet-sig",
        })
        .to_string();
        mgr.handle_trade_payment(
            &proposal,
            Some(&proposal_payment_id),
            Some(1),
            &fake,
            100_000.0,
        )
        .await;

        let reservation = mgr
            .db
            .trade_reservation_by_trade_id(&trade_id)
            .unwrap()
            .unwrap();
        assert_eq!(reservation.outcome, "reserved");
        assert_eq!(reservation.user_channel_id, USER_CHANNEL_ID_DECIMAL);
        assert_eq!(reservation.peer_user_channel_id, wallet_user_channel_id);
        assert_eq!(mgr.stable_channels[0].expected_usd.0, 50.0);
        assert_eq!(
            mgr.db.get_sync_version(USER_CHANNEL_ID_DECIMAL).unwrap(),
            Some(0)
        );

        StableChannelManager::retry_pending_trade_responses(mgr.db.as_ref(), &fake).await;
        {
            let sends = fake.sends.lock().unwrap();
            assert_eq!(sends.len(), 1);
            let raw = std::str::from_utf8(sends[0].custom_tlvs[0].value.as_ref()).unwrap();
            let response = crate::messages::parse_envelope(raw).unwrap();
            let confirmation: stable_channels::trade::ConfirmTradeV1 =
                serde_json::from_str(&response.payload).unwrap();
            assert_eq!(confirmation.user_channel_id, wallet_user_channel_id);
        }

        let execution_payment_id = "a3".repeat(32);
        let execution_payload = serde_json::json!({
            "type": "TRADE_V1",
            "phase": "execute",
            "channel_id": reservation.channel_id,
            "user_channel_id": reservation.peer_user_channel_id,
            "trade_id": reservation.trade_id,
            "proposal_payment_id": reservation.proposal_payment_id,
            "proposal_hash": reservation.proposal_hash,
            "confirmation_id": reservation.confirmation_id,
            "expected_usd": reservation.expected_usd,
            "quote_price": reservation.quote_price,
            "fee_msat": reservation.fee_msat,
            "ts": test_unix_now(),
        })
        .to_string();
        let execution_hash = stable_channels::trade::request_hash(execution_payload.as_bytes());
        let execution = serde_json::json!({
            "payload": execution_payload,
            "signature": "wallet-sig",
        })
        .to_string();
        assert_eq!(
            mgr.db
                .expire_trade_reservations(reservation.expires_at)
                .unwrap(),
            1
        );
        assert_eq!(
            mgr.db
                .trade_reservation_by_trade_id(&trade_id)
                .unwrap()
                .unwrap()
                .outcome,
            "expired"
        );
        mgr.handle_trade_payment_at(
            &execution,
            Some(&execution_payment_id),
            Some(reservation.fee_msat),
            Some(reservation.expires_at as u64),
            &fake,
            120_000.0,
        )
        .await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 60.0);
        assert_eq!(
            mgr.db.get_sync_version(USER_CHANNEL_ID_DECIMAL).unwrap(),
            Some(1)
        );
        let terminal = mgr
            .db
            .trade_reservation_by_trade_id(&trade_id)
            .unwrap()
            .unwrap();
        assert_eq!(terminal.outcome, "accepted");
        assert_eq!(
            terminal.execution_payment_id.as_deref(),
            Some(execution_payment_id.as_str())
        );
        assert_eq!(
            terminal.execution_hash.as_deref(),
            Some(execution_hash.as_str())
        );

        StableChannelManager::retry_pending_trade_responses(mgr.db.as_ref(), &fake).await;
        let sends = fake.sends.lock().unwrap();
        assert_eq!(sends.len(), 2);
        let raw = std::str::from_utf8(sends[1].custom_tlvs[0].value.as_ref()).unwrap();
        let response = crate::messages::parse_envelope(raw).unwrap();
        let sync: serde_json::Value = serde_json::from_str(&response.payload).unwrap();
        assert_eq!(sync["type"], "SYNC_V1");
        assert_eq!(sync["user_channel_id"], wallet_user_channel_id);
        assert_eq!(sync["trade_payment_id"], execution_payment_id);
        assert_eq!(sync["request_hash"], execution_hash);
        drop(sends);
        assert!(mgr
            .db
            .list_settlements()
            .unwrap()
            .contains(&("fake-payment-id".to_string(), "sync".to_string())));
    }

    #[tokio::test]
    async fn two_phase_full_peg_absorbs_lsp_drift_before_confirmation() {
        let (mut mgr, fake) = correlated_rejection_context(34.8404, 55_278, 68_550);
        let trade_id = "b1".repeat(32);
        let proposal_payment_id = "b2".repeat(32);
        let proposal_payload = serde_json::json!({
            "type": "TRADE_V1",
            "phase": "propose",
            "channel_id": CHANNEL_ID_HEX,
            "user_channel_id": "42",
            "trade_id": trade_id,
            "expected_usd": 43.1366,
            "quote_price": 63_052.275,
            "base_sync_version": 0,
            "ts": test_unix_now(),
        })
        .to_string();
        let proposal = serde_json::json!({
            "payload": proposal_payload,
            "signature": "wallet-sig",
        })
        .to_string();

        mgr.handle_trade_payment(
            &proposal,
            Some(&proposal_payment_id),
            Some(1),
            &fake,
            63_052.275,
        )
        .await;

        let reservation = mgr
            .db
            .trade_reservation_by_trade_id(&trade_id)
            .unwrap()
            .unwrap();
        assert_eq!(reservation.outcome, "reserved");
        assert_eq!(reservation.fee_msat, 132_000);
        assert_eq!(reservation.backing_sats, 68_418);
        assert_eq!(reservation.native_sats, 0);
    }

    #[tokio::test]
    async fn two_phase_full_peg_absorbs_sub_cent_usd_headroom_before_sat_flooring() {
        let (mut mgr, fake) = correlated_rejection_context(0.0, 0, 67_735);
        let trade_id = "b3".repeat(32);
        let proposal_payment_id = "b4".repeat(32);
        let proposal_payload = serde_json::json!({
            "type": "TRADE_V1",
            "phase": "propose",
            "channel_id": CHANNEL_ID_HEX,
            "user_channel_id": "42",
            "trade_id": trade_id,
            "expected_usd": 42.2532,
            "quote_price": 63_024.5675,
            "base_sync_version": 0,
            "ts": test_unix_now(),
        })
        .to_string();
        let proposal = serde_json::json!({
            "payload": proposal_payload,
            "signature": "wallet-sig",
        })
        .to_string();

        mgr.handle_trade_payment(
            &proposal,
            Some(&proposal_payment_id),
            Some(1),
            &fake,
            63_024.15,
        )
        .await;

        let reservation = mgr
            .db
            .trade_reservation_by_trade_id(&trade_id)
            .unwrap()
            .unwrap();
        assert_eq!(reservation.outcome, "reserved");
        assert_eq!(reservation.fee_msat, 677_000);
        assert_eq!(reservation.backing_sats, 67_058);
        assert_eq!(reservation.native_sats, 0);
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
            manager.db.in_flight_trade_response_payment_ids().unwrap(),
            vec!["fake-payment-id".to_string()]
        );
        fake.payments.lock().unwrap().push(GrpcPayment {
            id: "fake-payment-id".to_string(),
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
        assert_eq!(
            mgr.db
                .load_channel(USER_CHANNEL_ID_DECIMAL)
                .unwrap()
                .unwrap()
                .expected_usd,
            50.0
        );
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
        let envelope =
            correlated_trade_envelope_at(&trade_id, 50.0, Some(100_000.0), test_unix_now());
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
        let envelope =
            correlated_trade_envelope_at(&trade_id, 60.0, Some(100_000.0), test_unix_now());
        assert_eq!(
            correlated_rejection_reason(&mut manager, &fake, &envelope, &payment_id, 1, 100_000.0)
                .await,
            TradeRejectionReason::InvalidFee
        );

        let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let envelope = correlated_trade_envelope_at(&trade_id, 60.0, Some(-1.0), test_unix_now());
        assert_eq!(
            correlated_rejection_reason(&mut manager, &fake, &envelope, &payment_id, 1, 100_000.0)
                .await,
            TradeRejectionReason::InvalidQuote
        );

        let (mut manager, fake) = correlated_rejection_context(50.0, 50_000, 100_000);
        let envelope =
            correlated_trade_envelope_at(&trade_id, 60.0, Some(90_000.0), test_unix_now());
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
        let envelope =
            correlated_trade_envelope_at(&trade_id, 110.0, Some(100_000.0), test_unix_now());
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
        let envelope =
            correlated_trade_envelope_at(&trade_id, 0.0, Some(90_000.0), test_unix_now());
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
        let envelope =
            correlated_trade_envelope_at(&trade_id, 20.0, Some(100_000.0), test_unix_now());
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
        let envelope =
            correlated_trade_envelope_at(&trade_id, 20.0, Some(100_000.0), test_unix_now());
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

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0);
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

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
        handle_trade_with_valid_fee(&mut mgr, &tiny, &fake as &dyn LdkServerCalls, 110_000.0).await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 100_009);

        let noop = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 100.01);
        handle_trade_with_valid_fee(&mut mgr, &noop, &fake as &dyn LdkServerCalls, 90_000.0).await;
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

        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

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

            handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, price).await;

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

        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_001.0).await;

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

        mgr.handle_trade_message(&env, None, Some(1), &fake as &dyn LdkServerCalls, 100_000.0)
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
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

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
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

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
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 49.95);
        assert_eq!(mgr.stable_channels[0].backing_sats, 49_950);
        assert_eq!(fake.sends.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn trade_rejects_invalid_signature() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )])
        .with_verify_failure();
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            3.0,
            3_000,
            47_000,
            50_000,
            100_000.0,
        );

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0);
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 3.0).abs() < 1e-6); // unchanged
    }

    #[tokio::test]
    async fn trade_rejects_over_balance() {
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

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 999.0);
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 0.0).abs() < 1e-6); // unchanged
    }

    #[tokio::test]
    async fn trade_rejects_even_one_sat_above_balance_boundary() {
        let mut mgr = make_manager();
        // Live receiver side = 50_000 sats at $100k -> receiver_usd = $50.00 exactly.
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

        let target = 50.001;
        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, target);
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 0);
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn trade_rejects_dollar_denominated_capacity_epsilon() {
        let mut mgr = make_manager();
        // Live receiver side = 50_000 sats at $100k -> receiver_usd = $50.00 exactly.
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

        let target = 50.01;
        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, target);
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

        assert_eq!(mgr.stable_channels[0].expected_usd.0, 0.0);
        assert_eq!(mgr.stable_channels[0].backing_sats, 0);
        assert!(fake.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn trade_channel_not_found_is_noop() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![]);
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

        let env = trade_envelope(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0);
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 5.0).abs() < 1e-6); // unchanged
    }

    #[tokio::test]
    async fn trade_rejects_stale_ts() {
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

        // A captured signed trade replayed a day later must be rejected (replay protection).
        let stale = test_unix_now() - 86_400;
        let env = trade_envelope_with_ts(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0, stale);
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

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
        let stale = test_unix_now() - 86_400;
        let env = trade_envelope_with_ts(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, 10.0, stale);
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let stale_ev = events
            .iter()
            .find(|(e, _)| e == "TRADE_STALE")
            .expect("TRADE_STALE must be emitted for a stale signed trade");
        assert!(
            stale_ev.1.get("user_channel_id").is_some(),
            "TRADE_STALE must carry user_channel_id"
        );
    }

    #[tokio::test]
    async fn trade_accepts_fresh_ts() {
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

        // A trade signed just now is within the window and applies normally.
        let env = trade_envelope_with_ts(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            10.0,
            test_unix_now(),
        );
        handle_trade_with_valid_fee(&mut mgr, &env, &fake as &dyn LdkServerCalls, 100_000.0).await;

        assert!(
            (mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6,
            "a fresh signed trade must apply"
        );
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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )]);
        // expected $10 -> backing 10,000; receiver was 50,000.
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            10.0,
            10_000,
            40_000,
            50_000,
            100_000.0,
        );

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
        assert_eq!(
            fake.sends.lock().unwrap().len(),
            1,
            "splice-out should SYNC"
        );
    }

    #[tokio::test]
    async fn splice_in_does_not_sync() {
        let mut mgr = make_manager();
        // Post-splice snapshot: their grew to 80,000 (our 20k via outbound 20M msat).
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            20_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            10.0,
            10_000,
            40_000,
            50_000,
            100_000.0,
        );

        mgr.handle_channel_ready(
            CHANNEL_ID_HEX.to_string(),
            USER_CHANNEL_ID_DECIMAL.to_string(),
            Some("splice-in-funding:0".to_owned()),
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

        assert!((mgr.stable_channels[0].expected_usd.0 - 10.0).abs() < 1e-6); // unchanged
        assert_eq!(
            fake.sends.lock().unwrap().len(),
            0,
            "splice-in must not SYNC"
        );
    }

    #[tokio::test]
    async fn splice_replay_does_not_double_deduct() {
        let mut mgr = make_manager();
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            95_000_000,
            true,
        )]);
        seed_channel(
            &mut mgr,
            189476124653200987495269098788434301048u128,
            COUNTERPARTY_HEX,
            CHANNEL_ID_HEX,
            10.0,
            10_000,
            40_000,
            50_000,
            100_000.0,
        );

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
        assert_eq!(
            fake.sends.lock().unwrap().len(),
            1,
            "second pass deducts nothing, no second SYNC"
        );
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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_DECIMAL,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(7.5),
            None,
            &fake as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;

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
        let fake = FakeLdkServer::new(vec![])
            .with_forwarded(vec![fwd("aa", "bb", 1000), fwd("cc", "dd", 2000)]);
        assert_eq!(
            crate::backfill::backfill_forwards(&fake, &db).await.emitted,
            2
        ); // both unseen
        assert_eq!(
            crate::backfill::backfill_forwards(&fake, &db).await.emitted,
            0
        ); // both now seen
    }

    #[tokio::test]
    async fn forward_backfill_stops_on_repeated_cursor() {
        let dir = tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let fake = FakeLdkServer::new(vec![]).with_forward_cursor(PageToken {
            token: "same-page".to_owned(),
            index: 7,
        });

        let result = crate::backfill::backfill_forwards(&fake, &db).await;
        assert!(result.failure.is_none());
        assert!(result.incomplete.as_deref().unwrap().contains("repeated"));
        assert_eq!(fake.forward_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn reconnect_reconstructs_channel_payment_forward_peer_and_sweep() {
        use ldk_server_client::ldk_server_grpc::types::{pending_sweep_balance, PendingBroadcast};
        use stable_channels::ledger::{LedgerCompleteness, LedgerQuery};

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
            id: "payment-no-channel".into(),
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
                PendingBroadcast {
                    channel_id: Some(CHANNEL_ID_HEX.into()),
                    amount_satoshis: 777,
                },
            )),
        }]);

        let counts = crate::backfill::reconcile_event_history(&fake, &db).await;
        assert_eq!(counts.channels, 1);
        assert_eq!(counts.payments, 1);
        assert_eq!(counts.forwards, 1);
        assert_eq!(counts.peers, 1);
        assert_eq!(counts.sweeps, 1);
        assert_eq!(counts.failed_scopes, 0);

        let page = db
            .list_ledger_events(&LedgerQuery {
                completeness: Some("reconstructed".into()),
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.events.len(), 5);
        assert!(page
            .events
            .iter()
            .all(|event| event.completeness == LedgerCompleteness::Reconstructed));
        let payment = page
            .events
            .iter()
            .find(|event| event.event_type == "PAYMENT_RECONSTRUCTED")
            .unwrap();
        assert_eq!(payment.status, "completed");
        assert_eq!(payment.detail["ldk_status"], "SUCCEEDED");
        assert_eq!(
            payment.detail["channel_association"],
            "unavailable_from_ldk"
        );
        assert!(!payment
            .refs
            .iter()
            .any(|reference| reference.role.contains("channel")));

        let replay_counts = crate::backfill::reconcile_event_history(&fake, &db).await;
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
            id: "successful-payment".into(),
            amount_msat: Some(1_000_000),
            fee_paid_msat: Some(25),
            direction: 1,
            status: PaymentStatus::Succeeded as i32,
            latest_update_timestamp: 123,
            ..Default::default()
        }]);

        let counts = crate::backfill::reconcile_event_history(&fake, &db).await;
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
            pending_sweep_balance, AwaitingThresholdConfirmations, BroadcastAwaitingConfirmation,
            PendingBroadcast,
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
        let fake = FakeLdkServer::new(vec![]).with_sweeps(vec![pending, broadcast.clone()]);
        assert_eq!(
            crate::backfill::reconcile_event_history(&fake, &db)
                .await
                .sweeps,
            2
        );

        *fake.sweeps.lock().unwrap() = vec![broadcast];
        assert_eq!(
            crate::backfill::reconcile_event_history(&fake, &db)
                .await
                .sweeps,
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
            crate::backfill::reconcile_event_history(&fake, &db)
                .await
                .sweeps,
            1,
            "the same sweep changing confirmation state must remain visible"
        );
    }

    #[test]
    fn should_log_on_outcome_change() {
        assert!(stability_should_log(
            "",
            "check_only",
            0.0,
            90.0,
            90.0,
            0.25,
            1.0,
            true
        ));
        assert!(stability_should_log(
            "cooldown",
            "check_only",
            90.0,
            90.0,
            90.0,
            0.25,
            1.0,
            true
        ));
    }

    #[test]
    fn should_log_on_significant_value_move_when_tracking() {
        // same outcome, move > $0.25 and > 1% -> true
        assert!(stability_should_log(
            "check_only",
            "check_only",
            90.0,
            92.0,
            90.0,
            0.25,
            1.0,
            true
        ));
        // same outcome, sub-threshold move -> false
        assert!(!stability_should_log(
            "check_only",
            "check_only",
            90.0,
            90.10,
            90.0,
            0.25,
            1.0,
            true
        ));
    }

    #[test]
    fn no_value_trigger_when_not_tracking() {
        // same outcome, huge move, but track_value=false -> false
        assert!(!stability_should_log(
            "high_risk",
            "high_risk",
            90.0,
            200.0,
            90.0,
            0.25,
            1.0,
            false
        ));
    }

    #[tokio::test]
    async fn run_tick_throttles_repeated_check_only() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
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
            &fake0 as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        stable_channels::audit::enable_test_capture();
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 120_000.0)
            .await; // above par -> CHECK_ONLY (emit)
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 120_000.0)
            .await; // identical -> throttled
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let n = events
            .iter()
            .filter(|(e, _)| e == "STABILITY_CHECK_ONLY")
            .count();
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
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
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
        assert_eq!(
            verify_calls[0].message.as_ref(),
            envelope.payload.as_bytes()
        );
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
        let legacy_user_channel_id =
            format!("{:032x}", USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap());
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
        assert!(mgr
            .db
            .load_channel(USER_CHANNEL_ID_DECIMAL)
            .unwrap()
            .is_none());

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

        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
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
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(10.0),
            None,
            &fake0 as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        // Snapshot balances at par (no drift at $100k, so nothing fires).
        mgr.run_tick(&fake0 as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);
        assert_eq!(mgr.stable_channels[0].stable_receiver_btc.sats, 50_000);

        // Price rises to $110k: user is $1 above par and settles 909 sats to the LSP.
        // Live user side drops 50_000 -> 49_091 (our side gains).
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
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
        assert_eq!(
            sc.backing_sats, 9_091,
            "backing is reduced by the settled amount"
        );
        // Native absorbs only rounding, never the settlement: 49_091 - 9_091 = 40_000.
        assert_eq!(
            sc.native_sats, 40_000,
            "native sats must be preserved across the settlement"
        );
        assert!(
            sc.backing_sats <= sc.stable_receiver_btc.sats,
            "backing may never exceed live balance"
        );
    }

    #[tokio::test]
    async fn token_stability_payment_settles_only_the_amount_paid() {
        // Exploit guard: a 1-sat payment against a large above-par surplus must settle
        // exactly 1 sat, NOT reset backing to equilibrium (which would hand the entire
        // surplus to the sender as free native BTC for a fraction of a cent).
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(10.0),
            None,
            &fake0 as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        // Snapshot at par: backing = 10_000 sats, user side = 50_000.
        mgr.run_tick(&fake0 as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        assert_eq!(mgr.stable_channels[0].backing_sats, 10_000);

        // Price rises to $110k: the honest owed amount is ~910 sats. The attacker instead
        // keysends 1 sat with the same marker. The user paying 1 sat raises the LSP's
        // outbound by 1 sat (50_000_000 -> 50_001_000), so the user side drops 50_000 -> 49_999.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_001_000,
            true,
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
        assert_eq!(
            sc.backing_sats, 9_999,
            "a 1-sat payment settles only 1 sat of drift"
        );
        assert_ne!(
            sc.backing_sats, 9_090,
            "backing must NOT collapse to equilibrium for a token payment"
        );
        assert_eq!(
            sc.native_sats, 40_000,
            "the surplus is not reclassified as free native BTC"
        );
    }

    #[tokio::test]
    async fn incoming_stability_payment_prevents_backstop_misfire() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let fake0 = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_000_000,
            true,
        )]);
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(10.0),
            None,
            &fake0 as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake0 as &dyn LdkServerCalls, &push, 100_000.0)
            .await;

        // User settles 909 sats; books reconciled at receive.
        let fake = FakeLdkServer::new(vec![make_channel(
            CHANNEL_ID_HEX,
            USER_CHANNEL_ID_HEX,
            COUNTERPARTY_HEX,
            100_000,
            50_909_000,
            true,
        )]);
        mgr.handle_payment_received(
            vec![stability_marker()],
            Some("pay-2".to_string()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;
        let expected_before = mgr.stable_channels[0].expected_usd.0;

        // Two ticks at the new price: without receive-time reconcile the backstop
        // would read the settled sats as an unreconciled spend and deduct expected_usd.
        stable_channels::audit::enable_test_capture();
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 110_000.0)
            .await;
        mgr.run_tick(&fake as &dyn LdkServerCalls, &push, 110_000.0)
            .await;
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
                make_channel(
                    CHANNEL_ID_HEX,
                    USER_CHANNEL_ID_HEX,
                    COUNTERPARTY_HEX,
                    100_000,
                    outbound_msat,
                    true,
                ),
                make_channel(
                    CHAN2_ID,
                    UID2_HEX,
                    COUNTERPARTY_HEX,
                    100_000,
                    outbound_msat,
                    true,
                ),
            ]
        };
        let fake0 = FakeLdkServer::new(mk(50_000_000));
        mgr.edit_stable_channel(
            CHANNEL_ID_HEX,
            Some(10.0),
            None,
            &fake0 as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        mgr.edit_stable_channel(
            CHAN2_ID,
            Some(10.0),
            None,
            &fake0 as &dyn LdkServerCalls,
            100_000.0,
        )
        .await;
        let push = std::sync::Arc::new(tokio::sync::Mutex::new(crate::push::PushService::new(
            &crate::config::PushConfig::default(),
            mgr.data_dir(),
        )));
        mgr.run_tick(&fake0 as &dyn LdkServerCalls, &push, 100_000.0)
            .await;
        let backing_before: Vec<u64> = mgr.stable_channels.iter().map(|s| s.backing_sats).collect();

        let fake = FakeLdkServer::new(mk(50_909_000));
        stable_channels::audit::enable_test_capture();
        mgr.handle_payment_received(
            vec![stability_marker()],
            Some("pay-3".to_string()),
            Some(909_000),
            &fake as &dyn LdkServerCalls,
            110_000.0,
        )
        .await;
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();

        let backing_after: Vec<u64> = mgr.stable_channels.iter().map(|s| s.backing_sats).collect();
        assert_eq!(
            backing_before, backing_after,
            "ambiguous attribution must not touch the books"
        );
        assert!(
            events
                .iter()
                .any(|(e, d)| e == "STABILITY_RECEIVE_UNATTRIBUTED"
                    && d.get("candidates").and_then(|v| v.as_u64()) == Some(2)),
            "the miss must be audited with the candidate count"
        );
    }

    #[test]
    fn stability_reservation_guard_fails_closed() {
        assert!(stability_mutation_allowed(&Ok::<Option<&str>, &str>(None)));
        assert!(!stability_mutation_allowed(&Ok::<Option<&str>, &str>(
            Some("trade-id")
        )));
        assert!(!stability_mutation_allowed(&Err::<Option<&str>, &str>(
            "database locked"
        )));
    }
}
