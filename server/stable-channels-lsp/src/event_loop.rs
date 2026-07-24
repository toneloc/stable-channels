//! Long-running SubscribeEvents loop: connects to LDK Server's event stream, reconnects with exponential backoff, and dispatches each EventEnvelope to its handler.

use std::time::Duration;

use tracing::{info, warn};

use ldk_server_client::ldk_server_grpc::events::event_envelope::Event as EventVariant;
use ldk_server_client::ldk_server_grpc::events::{ChannelState, EventEnvelope};

use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

/// Build the audit `data` for a claimable (unclaimed inbound) payment; no user_channel_id exists yet.
fn claimable_audit_data(
    payment_id: Option<&str>,
    amount_msat: Option<u64>,
    has_custom_records: bool,
) -> serde_json::Value {
    let mut data = serde_json::json!({ "has_custom_records": has_custom_records });
    if let Some(pid) = payment_id {
        data["payment_id"] = serde_json::json!(pid);
    }
    if let Some(amt) = amount_msat {
        data["amount_msat"] = serde_json::json!(amt);
    }
    data
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut stream = match state.ldk_server.subscribe_events().await {
            Ok(s) => {
                backoff = Duration::from_secs(1);
                s
            },
            Err(e) => {
                warn!(
                    "[event_loop] subscribe_events failed: {}; retry in {:?}",
                    e, backoff
                );
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                continue;
            },
        };
        info!("[event_loop] subscribed");
        {
            let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
            if btc_price > 0.0 {
                state
                    .stable_manager
                    .lock()
                    .await
                    .reconcile_from_grpc(state.ldk_server.as_ref(), btc_price)
                    .await;
            } else {
                warn!("[event_loop] reconnect reconcile skipped: price cache cold");
            }
            let n = crate::backfill::backfill_forwards(state.ldk_server.as_ref(), state.db.as_ref()).await;
            if n > 0 {
                info!("[event_loop] backfilled {} forward(s)", n);
            }
        }
        while let Some(item) = stream.next_message().await {
            dispatch(item, &state).await;
        }
        warn!("[event_loop] stream ended; reconnecting");
    }
}

async fn dispatch(
    item: Result<EventEnvelope, ldk_server_client::error::LdkServerError>,
    state: &AppState,
) {
    let envelope = match item {
        Ok(e) => e,
        Err(e) => {
            warn!("[event_loop] item error: {}", e);
            return;
        },
    };
    let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
    let mut mgr = state.stable_manager.lock().await;
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    match envelope.event {
        Some(EventVariant::ChannelStateChanged(e)) => {
            if e.state == ChannelState::Ready as i32 {
                mgr.handle_channel_ready(
                    e.channel_id.clone(),
                    e.user_channel_id.clone(),
                    ldk,
                    btc_price,
                )
                .await;
            } else if e.state == ChannelState::Closed as i32 {
                mgr.handle_channel_closed(
                    e.channel_id.clone(),
                    e.user_channel_id.clone(),
                    e.counterparty_node_id.clone(),
                    e.funding_txo.clone(),
                    e.closure_initiator,
                    e.reason.clone(),
                );
            } else if e.state == ChannelState::Pending as i32 {
                stable_channels::audit::audit_event(
                    "CHANNEL_PENDING",
                    serde_json::json!({
                        "channel_id": e.channel_id,
                        "user_channel_id": e.user_channel_id,
                        "counterparty_node_id": e.counterparty_node_id,
                    }),
                );
            } else if e.state == ChannelState::OpenFailed as i32 {
                stable_channels::audit::audit_event(
                    "CHANNEL_OPEN_FAILED",
                    crate::channel_close::close_audit_data(
                        &e.channel_id,
                        &e.user_channel_id,
                        e.counterparty_node_id.as_deref(),
                        e.funding_txo.as_deref(),
                        e.closure_initiator,
                        e.reason.as_ref(),
                    ),
                );
            } else {
                stable_channels::audit::audit_event(
                    "CHANNEL_STATE_UNKNOWN",
                    crate::channel_close::unknown_state_audit_data(
                        &e.channel_id,
                        &e.user_channel_id,
                        e.counterparty_node_id.as_deref(),
                        e.state,
                    ),
                );
            }
        },
        Some(EventVariant::PaymentReceived(e)) => {
            let payment_id = e.payment.as_ref().map(|p| p.id.clone());
            let amount_msat = e.payment.as_ref().and_then(|p| p.amount_msat);
            mgr.handle_payment_received(e.custom_records, payment_id, amount_msat, ldk, btc_price)
                .await;
        },
        Some(EventVariant::PaymentForwarded(e)) => {
            if let Some(fp) = e.forwarded_payment {
                // ForwardedPayment now carries per-HTLC locators; take the first of each list as the representative channel/node.
                let prev = fp.prev_htlcs.first();
                let next = fp.next_htlcs.first();
                let prev_channel_id = prev.map(|h| h.channel_id.clone()).unwrap_or_default();
                let next_channel_id = next.map(|h| h.channel_id.clone()).unwrap_or_default();
                let fp_key = stable_channels::db::forward_fingerprint(
                    &prev_channel_id,
                    &next_channel_id,
                    fp.outbound_amount_forwarded_msat,
                    fp.total_fee_earned_msat,
                );
                mgr.handle_payment_forwarded(
                    prev.and_then(|h| h.user_channel_id.clone()).unwrap_or_default(),
                    next.and_then(|h| h.user_channel_id.clone()),
                    prev_channel_id,
                    next_channel_id,
                    prev.and_then(|h| h.node_id.clone()).unwrap_or_default(),
                    next.and_then(|h| h.node_id.clone()).unwrap_or_default(),
                    fp.outbound_amount_forwarded_msat.unwrap_or(0),
                    fp.total_fee_earned_msat.unwrap_or(0),
                    ldk,
                    btc_price,
                )
                .await;
                let _ = state.db.record_forwarded_seen(&fp_key);
            }
        },
        Some(EventVariant::PaymentSuccessful(e)) => {
            let payment_id = e.payment.as_ref().map(|p| p.id.clone());
            let amount_msat = e.payment.as_ref().and_then(|p| p.amount_msat);
            let fee_paid_msat = e.payment.as_ref().and_then(|p| p.fee_paid_msat);
            let direction = e.payment.as_ref().map(|p| if p.direction == 1 { "outbound" } else { "inbound" });
            let user_channel_id = payment_id.as_deref()
                .and_then(|pid| state.db.get_settlement_channel(pid).ok().flatten());
            stable_channels::audit::audit_event(
                "PAYMENT_SETTLED",
                serde_json::json!({
                    "payment_id": payment_id,
                    "amount_msat": amount_msat,
                    "fee_paid_msat": fee_paid_msat,
                    "direction": direction,
                    "user_channel_id": user_channel_id,
                }),
            );
        },
        Some(EventVariant::PaymentFailed(e)) => {
            let payment_id = e.payment.as_ref().map(|p| p.id.clone());
            let amount_msat = e.payment.as_ref().and_then(|p| p.amount_msat);
            let fee_paid_msat = e.payment.as_ref().and_then(|p| p.fee_paid_msat);
            let direction = e.payment.as_ref().map(|p| if p.direction == 1 { "outbound" } else { "inbound" });
            let user_channel_id = payment_id.as_deref()
                .and_then(|pid| state.db.get_settlement_channel(pid).ok().flatten());
            stable_channels::audit::audit_event(
                "PAYMENT_FAILED",
                serde_json::json!({
                    "payment_id": payment_id,
                    "amount_msat": amount_msat,
                    "fee_paid_msat": fee_paid_msat,
                    "direction": direction,
                    "user_channel_id": user_channel_id,
                }),
            );
        },
        Some(EventVariant::PaymentClaimable(e)) => {
            let payment_id = e.payment.as_ref().map(|p| p.id.clone());
            let amount_msat = e.payment.as_ref().and_then(|p| p.amount_msat);
            let has_custom_records = !e.custom_records.is_empty();
            stable_channels::audit::audit_event(
                "PAYMENT_CLAIMABLE",
                claimable_audit_data(payment_id.as_deref(), amount_msat, has_custom_records),
            );
        },
        _ => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claimable_full_fields_no_uid() {
        let d = claimable_audit_data(Some("abc123"), Some(150_000), false);
        assert_eq!(d["payment_id"], "abc123");
        assert_eq!(d["amount_msat"], 150_000u64);
        assert_eq!(d["has_custom_records"], false);
        assert!(d.get("user_channel_id").is_none());
    }

    #[test]
    fn claimable_omits_absent_amount() {
        let d = claimable_audit_data(Some("abc123"), None, false);
        assert!(d.get("amount_msat").is_none());
        assert_eq!(d["payment_id"], "abc123");
    }

    #[test]
    fn claimable_reflects_custom_records_and_omits_absent_id() {
        let d = claimable_audit_data(None, None, true);
        assert_eq!(d["has_custom_records"], true);
        assert!(d.get("payment_id").is_none());
    }
}
