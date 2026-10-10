//! Long-running SubscribeEvents loop: connects to LDK Server's event stream, reconnects with exponential backoff, and dispatches each EventEnvelope to its handler.

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
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
#[cfg(test)]
pub(crate) fn buffer_events(
    source: impl EventSource,
) -> (
    tokio::task::JoinSet<()>,
    tokio::sync::mpsc::UnboundedReceiver<EventItem>,
) {
    let (reader, receiver, _) = buffer_events_with_health(source);
    (reader, receiver)
}

struct ReaderActivity(Arc<AtomicBool>);

impl Drop for ReaderActivity {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn buffer_events_with_health(
    mut source: impl EventSource,
) -> (
    tokio::task::JoinSet<()>,
    tokio::sync::mpsc::UnboundedReceiver<EventItem>,
    Arc<AtomicBool>,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let active = Arc::new(AtomicBool::new(false));
    let activity = ReaderActivity(Arc::clone(&active));
    let mut reader = tokio::task::JoinSet::new();
    reader.spawn(async move {
        let activity = activity;
        activity.0.store(true, Ordering::Release);
        while let Some(item) = source.next_event().await {
            let failed = item.is_err();
            if failed {
                activity.0.store(false, Ordering::Release);
            }
            if sender.send(item).is_err() || failed {
                break;
            }
        }
    });
    (reader, receiver, active)
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// No per-event heartbeat writes. A delayed handler skips missed ticks rather than generating a
/// burst of checkpoint writes, and the timer shares a fair select with live event dispatch.
const STREAM_HEALTH_INTERVAL: Duration = Duration::from_secs(30);

struct StreamHealth {
    gap: Option<(i64, String)>,
    reconciled: Option<serde_json::Value>,
}

impl StreamHealth {
    fn new(gap: (i64, String)) -> Self {
        Self { gap: Some(gap), reconciled: None }
    }

    fn reconciliation_finished(
        &mut self,
        counts: &crate::backfill::ReconstructedCounts,
        result: serde_json::Value,
        result_recorded: bool,
    ) {
        self.reconciled = (reconciliation_allows_health(counts) && result_recorded).then_some(result);
    }

    fn checkpoint(
        &mut self,
        db: &stable_channels::db::Database,
        sampled_at_ms: i64,
        reader_active: bool,
        queue_empty: bool,
    ) -> rusqlite::Result<bool> {
        let Some(reconciliation) = &self.reconciled else { return Ok(false) };
        // A successful subscribe is not recovery. Buffered/failed terminal events must be handled
        // before the checkpoint can pass them; a reader that ended during backfill is not healthy.
        if !reader_active || !queue_empty {
            return Ok(false);
        }
        if let Some((_, id)) = &self.gap {
            if !db.close_event_stream_gap(id, sampled_at_ms, reconciliation)? {
                return Err(rusqlite::Error::InvalidParameterName("stream gap changed before closure".into()));
            }
            self.gap = None;
            Ok(true)
        } else {
            if !db.checkpoint_event_stream_health(sampled_at_ms)? {
                return Err(rusqlite::Error::InvalidParameterName("stream health blocked by unresolved gap".into()));
            }
            Ok(true)
        }
    }
}

fn reconciliation_allows_health(counts: &crate::backfill::ReconstructedCounts) -> bool {
    counts.failed_scopes == 0 && counts.incomplete_scopes == 0 && counts.settlement_outcomes_safe
}

fn health_interval(period: Duration) -> tokio::time::Interval {
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    timer
}

fn checkpoint_failed(op: &str, error: &rusqlite::Error) {
    warn!("[event_loop] {} failed; retaining conservative stream coverage: {}", op, error);
    stable_channels::audit::audit_event(
        "DB_WRITE_FAILED",
        serde_json::json!({"op": op, "error": error.to_string(), "scope": "event_stream_checkpoint"}),
    );
}

async fn begin_gap_with_retry(db: &stable_channels::db::Database) -> (i64, String) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match db.begin_event_stream_gap(now_millis() as i64) {
            Ok(gap) => return gap,
            Err(error) => checkpoint_failed("begin_event_stream_gap", &error),
        }
        tokio::time::sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
    }
}

fn checkpoint_if_ready(
    health: &mut StreamHealth,
    db: &stable_channels::db::Database,
    active: &AtomicBool,
    events: &tokio::sync::mpsc::UnboundedReceiver<EventItem>,
) {
    // Sample BEFORE checking liveness/drainage, so racing new arrivals are after this boundary.
    let now_ms = now_millis() as i64;
    if let Err(error) = health.checkpoint(db, now_ms, active.load(Ordering::Acquire), events.is_empty()) {
        checkpoint_failed("checkpoint_event_stream_health", &error);
    }
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
    // Reload the durable boundary, including unresolved gaps surviving earlier processes. The
    // compatibility AppState timestamp is never used as a substitute for a failed database read.
    let mut health = StreamHealth::new(begin_gap_with_retry(state.db.as_ref()).await);
    loop {
        let stream = match state.ldk_server.subscribe_events().await {
            Ok(s) => {
                backoff = Duration::from_secs(1);
                s
            },
            Err(e) => {
                let correlation_id = health.gap.as_ref().map(|gap| gap.1.as_str());
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
        let (_reader, mut events, active) = buffer_events_with_health(stream);
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
            connected_audit_data(health.gap.as_ref().map(|gap| gap.1.as_str()), ldk_server_version.as_deref()),
        );
        {
            stable_channels::audit::audit_event(
                "RECONCILIATION_STARTED",
                serde_json::json!({
                    "correlation_id": health.gap.as_ref().map(|gap| gap.1.as_str()),
                    "scopes": ["channels", "payments", "forwards", "peers", "sweeps"],
                }),
            );
            // Finish failed accounting writes before reconnect hydration or backfill can
            // change the same books. The existing stream remains open while we retry.
            let mut mgr = StableChannelManager::lock_for_event_with_top_ups_resolved(
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
                health.gap.as_ref().map(|gap| gap.0),
            ).await;
            let reconciliation_complete = reconciliation_allows_health(&counts);
            let reconciliation = serde_json::json!({
                "correlation_id": health.gap.as_ref().map(|gap| gap.1.as_str()),
                "gap_started_ms": health.gap.as_ref().map(|gap| gap.0),
                "counts": counts,
                "status": if !reconciliation_complete { "partial" }
                    else if counts.lost_scopes > 0 { "completed_with_loss" } else { "completed" },
                "history_complete": reconciliation_complete && counts.lost_scopes == 0,
                "coverage": "current_gap_only",
            });
            // Only a committed ledger row counts; a JSONL mirror would make the guard pass vacuously.
            let result_recorded = match stable_channels::audit::record_event(
                "RECONCILIATION_RESULT",
                reconciliation.clone(),
            ) {
                Ok(outcome) => outcome.event_id > 0,
                Err(error) => {
                    checkpoint_failed("record_reconciliation_result", &error);
                    false
                },
            };
            health.reconciliation_finished(&counts, reconciliation, result_recorded);
            if !counts.settlement_outcomes_safe {
                warn!(
                    "[event_loop] terminal settlement reconciliation incomplete; retrying before live dispatch"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        }
        checkpoint_if_ready(&mut health, state.db.as_ref(), &active, &events);
        let mut heartbeat = health_interval(STREAM_HEALTH_INTERVAL);
        loop {
            tokio::select! {
                item = events.recv() => {
                    let Some(item) = item else { break };
                    if dispatch(item, &state).await == DispatchOutcome::Reconnect {
                        break;
                    }
                },
                _ = heartbeat.tick() => {
                    checkpoint_if_ready(&mut health, state.db.as_ref(), &active, &events);
                },
            }
        }
        warn!("[event_loop] stream ended; reconnecting");
        // Persist before announcing the transition or attempting another subscription. If this
        // write fails, the prior durable health still bounds a crash/restart conservatively.
        health = StreamHealth::new(begin_gap_with_retry(state.db.as_ref()).await);
        stable_channels::audit::audit_event(
            "EVENT_STREAM_DISCONNECTED",
            serde_json::json!({ "correlation_id": health.gap.as_ref().map(|gap| gap.1.as_str()) }),
        );
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
    let mut mgr = StableChannelManager::lock_for_event_with_top_ups_resolved(&state.stable_manager, ldk).await;
    let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
    let outcome = dispatch_event(envelope.event, &mut mgr, &state.db, ldk, btc_price).await;
    drop(mgr);
    if outcome == DispatchOutcome::Continue {
        // The last event may itself have queued a failed accounting correction. Finish that
        // existing retry barrier before idle health can pass it, even if no next event arrives.
        drop(StableChannelManager::lock_for_event(&state.stable_manager, ldk).await);
    }
    outcome
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
                // Safe to book directly: dispatch runs under lock_for_event, which has already saved any correction that would replace these books.
                match mgr.settle_stability_top_up(payment_id, amount_msat, fee_paid_msat) {
                    Ok(crate::stable_manager::TopUpSettlement::NotOnTheWay) => {}
                    Ok(_) => settlement_handled = true,
                    Err(error) => {
                        stable_channels::audit::audit_event(
                            "DB_WRITE_FAILED",
                            serde_json::json!({
                                "op": "claim_stability_top_up",
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
            let top_up_channel = match payment_id.as_deref().map(|payment_id| mgr.fail_stability_top_up(payment_id)) {
                Some(Ok(channel)) => channel,
                Some(Err(error)) => {
                    stable_channels::audit::audit_event(
                        "DB_WRITE_FAILED",
                        serde_json::json!({
                            "op": "fail_stability_top_up",
                            "payment_id": payment_id,
                            "error": error.to_string(),
                        }),
                    );
                    return DispatchOutcome::Reconnect;
                }
                None => None,
            };
            if let (None, Some(payment_id)) = (&top_up_channel, payment_id.as_deref()) {
                mgr.note_failure_of_booked_top_up(payment_id);
            }
            // Only a top-up booked when it was sent has anything to roll back; for any other payment this finds nothing.
            let rollback = payment_id
                .as_deref()
                .and_then(|payment_id| mgr.handle_failed_stability_payment(payment_id));
            let user_channel_id = top_up_channel
                .or_else(|| rollback.as_ref().map(|rollback| rollback.user_channel_id.clone()))
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

    fn complete_counts() -> crate::backfill::ReconstructedCounts {
        crate::backfill::ReconstructedCounts {
            settlement_outcomes_safe: true,
            ..Default::default()
        }
    }

    fn completed_result() -> serde_json::Value {
        serde_json::json!({"status": "completed", "history_complete": true})
    }

    #[test]
    fn partial_failed_terminal_and_unsaved_reconciliation_keep_gap_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let gap = db.begin_event_stream_gap(1_000).unwrap();
        let mut health = StreamHealth::new(gap.clone());
        assert!(!health.checkpoint(&db, 2_000, true, true).unwrap(), "subscribe alone is not recovery");
        for (counts, recorded) in [
            (crate::backfill::ReconstructedCounts { incomplete_scopes: 1, ..complete_counts() }, true),
            (crate::backfill::ReconstructedCounts { failed_scopes: 1, ..complete_counts() }, true),
            (crate::backfill::ReconstructedCounts { settlement_outcomes_safe: false, ..complete_counts() }, true),
            (complete_counts(), false),
        ] {
            health.reconciliation_finished(&counts, completed_result(), recorded);
            assert!(!health.checkpoint(&db, 90_000_000, true, true).unwrap());
            let restarted = stable_channels::db::Database::open(dir.path()).unwrap();
            assert_eq!(restarted.begin_event_stream_gap(100_000_000).unwrap(), gap);
        }
        health.reconciliation_finished(&complete_counts(), completed_result(), true);
        assert!(!health.checkpoint(&db, 110_000_000, true, false).unwrap(), "buffered events precede health");
        assert!(!health.checkpoint(&db, 110_000_000, false, true).unwrap(), "ended reader is not healthy");
        assert_eq!(db.begin_event_stream_gap(110_000_001).unwrap(), gap);
        assert!(health.checkpoint(&db, 120_000_000, true, true).unwrap());
        assert!(health.gap.is_none());
    }

    #[test]
    fn checkpoint_failure_retains_memory_and_disk_gap_until_successful_retry() {
        let dir = tempfile::tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let conn = rusqlite::Connection::open(dir.path().join(stable_channels::db::DB_FILENAME)).unwrap();
        let gap = db.begin_event_stream_gap(1_000).unwrap();
        let mut health = StreamHealth::new(gap.clone());
        health.reconciliation_finished(&complete_counts(), completed_result(), true);
        conn.execute_batch(
            "CREATE TRIGGER fail_gap_audit BEFORE INSERT ON ledger_events
             WHEN NEW.event_type = 'EVENT_STREAM_GAP_CLOSED'
             BEGIN SELECT RAISE(FAIL, 'injected audit failure'); END;",
        ).unwrap();
        assert!(health.checkpoint(&db, 2_000, true, true).is_err());
        assert_eq!(health.gap.as_ref(), Some(&gap));
        assert_eq!(db.begin_event_stream_gap(3_000).unwrap(), gap);
        conn.execute_batch("DROP TRIGGER fail_gap_audit").unwrap();
        assert!(health.checkpoint(&db, 4_000, true, true).unwrap());
        assert!(health.gap.is_none());
        conn.execute_batch(
            "CREATE TRIGGER fail_health BEFORE UPDATE ON event_stream_checkpoint
             BEGIN SELECT RAISE(FAIL, 'injected heartbeat failure'); END;",
        ).unwrap();
        assert!(health.checkpoint(&db, 9_000_000, true, true).is_err());
        conn.execute_batch("DROP TRIGGER fail_health").unwrap();
        let restarted = stable_channels::db::Database::open(dir.path()).unwrap();
        assert_eq!(restarted.begin_event_stream_gap(9_000_001).unwrap().0, 4_000);
    }

    struct TestSource(tokio::sync::mpsc::UnboundedReceiver<EventItem>);

    #[async_trait::async_trait]
    impl EventSource for TestSource {
        async fn next_event(&mut self) -> Option<EventItem> {
            self.0.recv().await
        }
    }

    #[tokio::test]
    async fn idle_live_reader_can_checkpoint_but_eof_during_reconcile_cannot_close_gap() {
        let dir = tempfile::tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        health.reconciliation_finished(&complete_counts(), completed_result(), true);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (mut reader, events, active) = buffer_events_with_health(TestSource(receiver));
        tokio::time::timeout(Duration::from_secs(1), async {
            while !active.load(Ordering::Acquire) { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(events.is_empty());
        assert!(health.checkpoint(&db, 2_000, active.load(Ordering::Acquire), events.is_empty()).unwrap());
        // Many hours of idle service advance coverage without manufacturing payment/audit activity.
        assert!(health.checkpoint(&db, 90_000_000, active.load(Ordering::Acquire), events.is_empty()).unwrap());
        let gap = db.begin_event_stream_gap(90_005_000).unwrap();
        assert_eq!(gap.0, 90_000_000);
        health = StreamHealth::new(gap.clone());
        health.reconciliation_finished(&complete_counts(), completed_result(), true);
        drop(sender);
        reader.join_next().await.unwrap().unwrap();
        assert!(!active.load(Ordering::Acquire));
        assert!(!health.checkpoint(&db, 90_006_000, active.load(Ordering::Acquire), events.is_empty()).unwrap());
        assert_eq!(db.begin_event_stream_gap(90_007_000).unwrap(), gap);
    }

    #[tokio::test]
    async fn aborted_reader_is_not_reported_healthy() {
        let (_sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (mut reader, _events, active) = buffer_events_with_health(TestSource(receiver));
        tokio::time::timeout(Duration::from_secs(1), async {
            while !active.load(Ordering::Acquire) { tokio::task::yield_now().await; }
        }).await.unwrap();
        reader.abort_all();
        assert!(reader.join_next().await.unwrap().unwrap_err().is_cancelled());
        assert!(!active.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn buffered_error_marks_reader_unhealthy_before_dispatch_reaches_it() {
        use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (mut reader, mut events, active) = buffer_events_with_health(TestSource(receiver));
        sender.send(Ok(EventEnvelope::default())).unwrap();
        sender.send(Err(LdkServerError::new(
            LdkServerErrorCode::InternalServerError, "stream failed during backfill",
        ))).unwrap();
        reader.join_next().await.unwrap().unwrap();
        assert!(!active.load(Ordering::Acquire));
        assert!(events.recv().await.unwrap().is_ok());
        assert!(!active.load(Ordering::Acquire), "an earlier buffered success cannot restore health");
        assert!(events.recv().await.unwrap().is_err());
    }

    #[tokio::test]
    async fn heartbeat_is_delayed_and_skips_catchup_bursts() {
        let period = Duration::from_millis(20);
        let started = tokio::time::Instant::now();
        let mut heartbeat = health_interval(period);
        // No immediate tick that could write on every fresh event-loop iteration.
        assert!(heartbeat.tick().await >= started + period);
        tokio::time::sleep(period * 5).await;
        let after_delay = tokio::time::Instant::now();
        heartbeat.tick().await;
        let next = heartbeat.tick().await;
        assert!(next > after_delay,
            "a delayed handler must skip stale heartbeat deadlines rather than burst writes");
    }

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
