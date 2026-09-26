//! Long-running SubscribeEvents loop: connects to LDK Server's event stream, reconnects with exponential backoff, and dispatches each EventEnvelope to its handler.

use std::time::Duration;

use tracing::{info, warn};

use ldk_server_client::ldk_server_grpc::events::event_envelope::Event as EventVariant;
use ldk_server_client::ldk_server_grpc::events::{ChannelState, EventEnvelope};

use crate::stable_manager::{LdkServerCalls, StableChannelManager};
use crate::state::AppState;

pub(crate) type EventItem = Result<EventEnvelope, ldk_server_client::error::LdkServerError>;

#[async_trait::async_trait]
pub(crate) trait EventSource: Send + 'static {
    async fn next_event(&mut self) -> Option<EventItem>;
}

#[async_trait::async_trait]
impl EventSource for ldk_server_client::client::EventStream {
    async fn next_event(&mut self) -> Option<EventItem> {
        self.next_message().await
    }
}

/// Keep draining the subscription while accounting retries. A bounded queue here would push
/// the wait back to LDK Server, whose broadcast stream silently drops events when it falls behind.
/// The caller owns the JoinSet so reconnect/cancellation also stops the old reader.
pub(crate) fn buffer_events(
    mut source: impl EventSource,
) -> (
    tokio::task::JoinSet<()>,
    tokio::sync::mpsc::UnboundedReceiver<EventItem>,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut reader = tokio::task::JoinSet::new();
    reader.spawn(async move {
        while let Some(item) = source.next_event().await {
            let failed = item.is_err();
            if sender.send(item).is_err() || failed {
                break;
            }
        }
    });
    (reader, receiver)
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchOutcome {
    Continue,
    Reconnect,
}

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

/// Decode LDK's PaymentFailureReason for the audit row; unknown values keep their raw number.
fn payment_failure_reason_name(reason: Option<i32>) -> Option<String> {
    use ldk_server_client::ldk_server_grpc::events::PaymentFailureReason;
    reason.map(|value| {
        PaymentFailureReason::from_i32(value)
            .map(|r| crate::channel_close::short(r.as_str_name(), "PAYMENT_FAILURE_REASON_"))
            .unwrap_or_else(|| format!("UNKNOWN({})", value))
    })
}

/// LDK Server rev this daemon's ldk-server-client is pinned to; a test keeps it equal to Cargo.toml.
const PINNED_LDK_SERVER_REV: &str = "bd95e187";

/// Whether LDK Server's reported build (`<version> (<git commit>)`) is the pinned rev; None when it reports no build.
fn ldk_server_matches_pin(ldk_server_version: Option<&str>) -> Option<bool> {
    ldk_server_version
        .filter(|version| !version.is_empty())
        .map(|version| version.contains(PINNED_LDK_SERVER_REV))
}

/// Audit data for a (re)connected event stream, naming the LDK Server build and whether it matches the pin.
fn connected_audit_data(correlation_id: Option<&str>, ldk_server_version: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "correlation_id": correlation_id,
        "ldk_server_version": ldk_server_version.filter(|version| !version.is_empty()),
        "expected_ldk_server_rev": PINNED_LDK_SERVER_REV,
        "ldk_server_matches_pin": ldk_server_matches_pin(ldk_server_version),
    })
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    let mut backoff = Duration::from_secs(1);
    let mut gap_correlation_id: Option<String> = None;
    // A cold start is a gap that began at the previous run's last ledger entry.
    let mut gap_started_ms: Option<i64> = state.last_event_before_start_ms;
    loop {
        let stream = match state.ldk_server.subscribe_events().await {
            Ok(s) => {
                backoff = Duration::from_secs(1);
                s
            },
            Err(e) => {
                if gap_correlation_id.is_none() {
                    let correlation_id = format!("event-stream-gap-{}", now_millis());
                    stable_channels::audit::audit_event(
                        "EVENT_STREAM_GAP_STARTED",
                        serde_json::json!({ "correlation_id": correlation_id }),
                    );
                    gap_correlation_id = Some(correlation_id);
                    gap_started_ms.get_or_insert(now_millis() as i64);
                }
                let correlation_id = gap_correlation_id.as_deref();
                stable_channels::audit::audit_event(
                    "EVENT_STREAM_CONNECT_FAILED",
                    serde_json::json!({
                        "correlation_id": correlation_id,
                        "error": e.to_string(),
                        "retry_delay_ms": backoff.as_millis(),
                    }),
                );
                warn!(
                    "[event_loop] subscribe_events failed: {}; retry in {:?}",
                    e, backoff
                );
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                continue;
            },
        };
        let (_reader, mut events) = buffer_events(stream);
        info!("[event_loop] subscribed");
        let ldk_server_version = match state
            .ldk_server
            .get_node_info(ldk_server_client::ldk_server_grpc::api::GetNodeInfoRequest {})
            .await
        {
            Ok(info) if !info.version.is_empty() => {
                if ldk_server_matches_pin(Some(&info.version)) == Some(false) {
                    warn!("[event_loop] LDK Server reports {}, but this daemon is built for LDK Server {}; run them at the same rev", info.version, PINNED_LDK_SERVER_REV);
                }
                Some(info.version)
            },
            Ok(_) => {
                warn!("[event_loop] LDK Server reported no build version, so it is likely older than {} and its payment events may not decode", PINNED_LDK_SERVER_REV);
                None
            },
            Err(e) => {
                warn!("[event_loop] get_node_info failed: {}", e);
                None
            },
        };
        stable_channels::audit::audit_event(
            "EVENT_STREAM_CONNECTED",
            connected_audit_data(gap_correlation_id.as_deref(), ldk_server_version.as_deref()),
        );
        {
            stable_channels::audit::audit_event(
                "RECONCILIATION_STARTED",
                serde_json::json!({
                    "correlation_id": gap_correlation_id.as_deref(),
                    "scopes": ["channels", "payments", "forwards", "peers", "sweeps"],
                }),
            );
            // Finish failed accounting writes before reconnect hydration or backfill can
            // change the same books. The existing stream remains open while we retry.
            let mut mgr = StableChannelManager::lock_for_event(
                &state.stable_manager,
                state.ldk_server.as_ref(),
            )
            .await;
            let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
            if btc_price > 0.0 {
                mgr.reconcile_from_grpc(state.ldk_server.as_ref(), btc_price)
                    .await;
            } else {
                warn!("[event_loop] reconnect reconcile skipped: price cache cold");
            }
            drop(mgr);
            let counts = crate::backfill::reconcile_event_history(
                state.ldk_server.as_ref(),
                state.db.as_ref(),
                gap_started_ms,
            ).await;
            let reconciliation_complete =
                counts.failed_scopes == 0 && counts.incomplete_scopes == 0;
            stable_channels::audit::audit_event(
                "RECONCILIATION_RESULT",
                serde_json::json!({
                    "correlation_id": gap_correlation_id.as_deref(),
                    "counts": counts,
                    "status": if reconciliation_complete { "completed" } else { "partial" },
                }),
            );
            if !counts.settlement_outcomes_safe {
                warn!(
                    "[event_loop] terminal settlement reconciliation incomplete; retrying before live dispatch"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            if reconciliation_complete {
                gap_started_ms = None;
                if let Some(correlation_id) = gap_correlation_id.take() {
                    stable_channels::audit::audit_event(
                        "EVENT_STREAM_GAP_CLOSED",
                        serde_json::json!({ "correlation_id": correlation_id }),
                    );
                }
            }
        }
        while let Some(item) = events.recv().await {
            if dispatch(item, &state).await == DispatchOutcome::Reconnect {
                break;
            }
        }
        warn!("[event_loop] stream ended; reconnecting");
        let correlation_id = gap_correlation_id
            .clone()
            .unwrap_or_else(|| format!("event-stream-gap-{}", now_millis()));
        stable_channels::audit::audit_event(
            "EVENT_STREAM_DISCONNECTED",
            serde_json::json!({ "correlation_id": correlation_id }),
        );
        if gap_correlation_id.is_none() {
            stable_channels::audit::audit_event(
                "EVENT_STREAM_GAP_STARTED",
                serde_json::json!({ "correlation_id": correlation_id }),
            );
            gap_correlation_id = Some(correlation_id);
            gap_started_ms.get_or_insert(now_millis() as i64);
        }
    }
}

async fn dispatch(
    item: Result<EventEnvelope, ldk_server_client::error::LdkServerError>,
    state: &AppState,
) -> DispatchOutcome {
    let envelope = match item {
        Ok(e) => e,
        Err(e) => {
            warn!("[event_loop] item error: {}", e);
            return DispatchOutcome::Reconnect;
        },
    };
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    // Keep this envelope on the stack while an earlier correction is unsaved. Returning or
    // reconnecting here would lose the event because LDK Server does not replay its stream.
    let mut mgr = StableChannelManager::lock_for_event(&state.stable_manager, ldk).await;
    let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
    dispatch_event(envelope.event, &mut mgr, &state.db, ldk, btc_price).await
}

pub(crate) async fn dispatch_event(
    event: Option<EventVariant>,
    mgr: &mut crate::stable_manager::StableChannelManager,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    btc_price: f64,
) -> DispatchOutcome {
    match event {
        Some(EventVariant::ChannelStateChanged(e)) => {
            if e.state == ChannelState::Ready as i32 {
                mgr.handle_channel_ready(
                    e.channel_id.clone(),
                    e.user_channel_id.clone(),
                    e.funding_txo.clone(),
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
                    crate::channel_audit::pending_audit_data(
                        &e.channel_id,
                        &e.user_channel_id,
                        e.counterparty_node_id.as_deref(),
                        e.funding_txo.as_deref(),
                        e.former_temporary_channel_id.as_deref(),
                    ),
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
            let payment_id = e.payment.as_ref().map(|p| p.payment_id.clone());
            let amount_msat = e.payment.as_ref().and_then(|p| p.amount_msat);
            mgr.handle_payment_received(e.custom_records, payment_id, amount_msat, ldk, btc_price)
                .await;
        },
        Some(EventVariant::PaymentForwarded(e)) => {
            // The event carries per-HTLC locators; take the first of each list as the representative channel/node.
            let prev = e.prev_htlcs.first();
            let next = e.next_htlcs.first();
            let prev_channel_id = prev.map(|h| h.channel_id.clone()).unwrap_or_default();
            let next_channel_id = next.map(|h| h.channel_id.clone()).unwrap_or_default();
            mgr.handle_payment_forwarded(
                prev.and_then(|h| h.user_channel_id.clone()).unwrap_or_default(),
                next.and_then(|h| h.user_channel_id.clone()),
                prev_channel_id,
                next_channel_id,
                prev.and_then(|h| h.node_id.clone()).unwrap_or_default(),
                next.and_then(|h| h.node_id.clone()).unwrap_or_default(),
                e.outbound_amount_forwarded_msat,
                e.total_fee_earned_msat.unwrap_or(0),
                e.skimmed_fee_msat,
                ldk,
                btc_price,
            )
            .await;
        },
        Some(EventVariant::PaymentSuccessful(e)) => {
            let payment_id = e.payment.as_ref().map(|p| p.payment_id.clone());
            let amount_msat = e.payment.as_ref().and_then(|p| p.amount_msat);
            let fee_paid_msat = e.payment.as_ref().and_then(|p| p.fee_paid_msat);
            let direction = e.payment.as_ref().map(|p| if p.direction == 1 { "outbound" } else { "inbound" });
            let mut settlement_handled = false;
            if let Some(payment_id) = payment_id.as_deref() {
                match db.mark_trade_response_delivered(
                    payment_id,
                    crate::stable_manager::StableChannelManager::unix_time_secs(),
                ) {
                    Ok(true) => settlement_handled = true,
                    Ok(false) => {}
                    Err(error) => {
                        stable_channels::audit::audit_event(
                            "DB_WRITE_FAILED",
                            serde_json::json!({
                                "op": "mark_trade_response_delivered",
                                "payment_id": payment_id,
                                "error": error.to_string(),
                            }),
                        );
                        return DispatchOutcome::Reconnect;
                    }
                }
                let known_settlement = db
                    .settlement_exists(payment_id)
                    .ok()
                    .unwrap_or(false);
                match db.mark_settlement_succeeded(
                    payment_id,
                    amount_msat,
                    fee_paid_msat,
                    direction,
                ) {
                    Ok(true) => settlement_handled = true,
                    Ok(false) if known_settlement => settlement_handled = true,
                    Ok(false) => {},
                    Err(error) => {
                        stable_channels::audit::audit_event(
                            "DB_WRITE_FAILED",
                            serde_json::json!({
                                "op": "mark_settlement_succeeded",
                                "payment_id": payment_id,
                                "error": error.to_string(),
                            }),
                        );
                        return DispatchOutcome::Reconnect;
                    },
                }
            }
            if !settlement_handled {
                stable_channels::audit::audit_event(
                    "PAYMENT_SETTLED",
                    serde_json::json!({
                        "payment_id": payment_id,
                        "amount_msat": amount_msat,
                        "fee_paid_msat": fee_paid_msat,
                        "direction": direction,
                    }),
                );
            }
        },
        Some(EventVariant::PaymentFailed(e)) => {
            let payment_id = e.payment.as_ref().map(|p| p.payment_id.clone());
            let amount_msat = e.payment.as_ref().and_then(|p| p.amount_msat);
            let fee_paid_msat = e.payment.as_ref().and_then(|p| p.fee_paid_msat);
            let direction = e.payment.as_ref().map(|p| if p.direction == 1 { "outbound" } else { "inbound" });
            if let Some(payment_id) = payment_id.as_deref() {
                // Persist the failure before continuing. The retry tick derives ordinary SYNC
                // obligations from these outcomes, including after a restart or reconnect.
                if let Err(error) = db.mark_sync_payment_failed(payment_id) {
                    stable_channels::audit::audit_event(
                        "DB_WRITE_FAILED",
                        serde_json::json!({
                            "op": "mark_sync_payment_failed",
                            "payment_id": payment_id,
                            "error": error.to_string(),
                        }),
                    );
                    return DispatchOutcome::Reconnect;
                }
                if let Err(error) = db.mark_trade_response_failed(
                    payment_id,
                    crate::stable_manager::StableChannelManager::unix_time_secs(),
                ) {
                    stable_channels::audit::audit_event(
                        "DB_WRITE_FAILED",
                        serde_json::json!({
                            "op": "mark_trade_response_failed",
                            "payment_id": payment_id,
                            "error": error.to_string(),
                        }),
                    );
                    return DispatchOutcome::Reconnect;
                }
            }
            let rollback = payment_id
                .as_deref()
                .and_then(|payment_id| mgr.handle_failed_stability_payment(payment_id));
            let user_channel_id = rollback
                .as_ref()
                .map(|rollback| rollback.user_channel_id.clone())
                .or_else(|| {
                    payment_id.as_deref()
                        .and_then(|pid| db.get_settlement_channel(pid).ok().flatten())
                });
            stable_channels::audit::audit_event(
                "PAYMENT_FAILED",
                serde_json::json!({
                    "payment_id": payment_id,
                    "amount_msat": amount_msat,
                    "fee_paid_msat": fee_paid_msat,
                    "direction": direction,
                    "user_channel_id": user_channel_id,
                    "stability_rollback_applied": rollback.map(|rollback| rollback.applied),
                    "reason": payment_failure_reason_name(e.reason),
                }),
            );
        },
        Some(EventVariant::PaymentClaimable(e)) => {
            let payment_id = e.payment.as_ref().map(|p| p.payment_id.clone());
            let amount_msat = e.payment.as_ref().and_then(|p| p.amount_msat);
            let has_custom_records = !e.custom_records.is_empty();
            stable_channels::audit::audit_event(
                "PAYMENT_CLAIMABLE",
                claimable_audit_data(payment_id.as_deref(), amount_msat, has_custom_records),
            );
        },
        // Audit-only: balances are reconciled when the spliced channel re-fires Ready.
        Some(EventVariant::SpliceNegotiated(e)) => {
            stable_channels::audit::audit_event(
                "SPLICE_NEGOTIATED",
                crate::channel_audit::splice_negotiated_audit_data(&e),
            );
        },
        Some(EventVariant::SpliceNegotiationFailed(e)) => {
            stable_channels::audit::audit_event(
                "SPLICE_NEGOTIATION_FAILED",
                crate::channel_audit::splice_failed_audit_data(&e),
            );
        },
        // prost decodes a oneof variant this pin does not know as None.
        None => warn!("[event_loop] unrecognized event variant; LDK Server is likely newer than this daemon's ldk-server-client pin"),
    }
    DispatchOutcome::Continue
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
    fn failure_reason_decodes_ldk_variants_and_keeps_unknown_numbers() {
        use ldk_server_client::ldk_server_grpc::events::PaymentFailureReason;
        assert_eq!(
            payment_failure_reason_name(Some(PaymentFailureReason::RouteNotFound as i32)).as_deref(),
            Some("ROUTE_NOT_FOUND")
        );
        assert_eq!(
            payment_failure_reason_name(Some(PaymentFailureReason::RecipientRejected as i32)).as_deref(),
            Some("RECIPIENT_REJECTED")
        );
        assert_eq!(payment_failure_reason_name(Some(99)).as_deref(), Some("UNKNOWN(99)"));
        assert_eq!(payment_failure_reason_name(None), None);
    }

    #[test]
    fn connected_row_names_the_ldk_server_build_and_whether_it_matches_the_pin() {
        let d = connected_audit_data(Some("event-stream-gap-1"), Some("0.1.0 (bd95e187b0c08b3fb90fc42a96f0f8a2b6773495)"));
        assert_eq!(d["correlation_id"], "event-stream-gap-1");
        assert_eq!(d["ldk_server_version"], "0.1.0 (bd95e187b0c08b3fb90fc42a96f0f8a2b6773495)");
        assert_eq!(d["expected_ldk_server_rev"], PINNED_LDK_SERVER_REV);
        assert_eq!(d["ldk_server_matches_pin"], true);
        assert_eq!(connected_audit_data(None, Some("0.1.0 (0e4434d7083ae9926c73f26e7cd52f00bde44d37)"))["ldk_server_matches_pin"], false);
        let unreported = connected_audit_data(None, Some(""));
        assert!(unreported["ldk_server_version"].is_null());
        assert!(unreported["ldk_server_matches_pin"].is_null());
    }

    #[test]
    fn pinned_ldk_server_rev_matches_the_cargo_dependency() {
        let manifest = include_str!("../Cargo.toml");
        let rev = manifest
            .lines()
            .find(|line| line.starts_with("ldk-server-client"))
            .and_then(|line| line.split("rev = \"").nth(1))
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        assert!(rev.starts_with(PINNED_LDK_SERVER_REV), "update PINNED_LDK_SERVER_REV with the ldk-server-client pin");
    }

    #[test]
    fn claimable_reflects_custom_records_and_omits_absent_id() {
        let d = claimable_audit_data(None, None, true);
        assert_eq!(d["has_custom_records"], true);
        assert!(d.get("payment_id").is_none());
    }
}
