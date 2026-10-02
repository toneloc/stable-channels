//! Periodic stability tick: every STABILITY_CHECK_INTERVAL_SECS, run_tick detects USD drift and either sends a stability payment or wakes the offline peer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ldk_server_client::ldk_server_grpc::api::ListChannelsRequest;
use ldk_server_client::ldk_server_grpc::types::Channel;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{timeout, MissedTickBehavior};
use tracing::{info, warn};

use crate::stable_manager::{
    parse_user_channel_id, LdkServerCalls, StabilityTickPlan, WakeSettlementRequest,
};
use crate::state::AppState;

// Allow time for FCM/APNs delivery, mobile service startup, LDK startup, and channel
// reconnection before giving up on the wake-triggered settlement attempt.
const WAKE_SETTLEMENT_POLL_TIMEOUT_SECS: u64 = 60;
const WAKE_SETTLEMENT_POLL_INTERVAL_SECS: u64 = 1;

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    let interval_secs = stable_channels::constants::STABILITY_CHECK_INTERVAL_SECS;
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let wake_polls = Arc::new(Mutex::new(HashSet::<String>::new()));
    let dispatches = Arc::new(Mutex::new(HashSet::<String>::new()));
    let mut wake_tasks = JoinSet::new();
    info!("[stability_tick] running every {}s", interval_secs);
    loop {
        ticker.tick().await;

        // Reap completed polls. Keeping the tasks in a JoinSet means cancellation of this
        // scheduler also aborts its children instead of leaving detached work behind.
        while let Some(result) = wake_tasks.try_join_next() {
            if let Err(error) = result {
                warn!(
                    "[stability_tick] wake poll task exited unexpectedly: {}",
                    error
                );
            }
        }

        let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
        if btc_price <= 0.0 {
            warn!("[stability_tick] price cache cold; skipping");
            continue;
        }
        let tick_plan = {
            let mut mgr = state.stable_manager.lock().await;
            mgr.reconcile_if_empty(state.ldk_server.as_ref() as &dyn LdkServerCalls, btc_price)
                .await;
            mgr.run_tick_plan(
                state.ldk_server.as_ref() as &dyn LdkServerCalls,
                btc_price,
                None,
                None,
            )
            .await
        };
        let mut peers: HashMap<String, StabilityTickPlan> = HashMap::new();
        for notification in tick_plan.notifications {
            peers.entry(notification.node_id.clone()).or_default()
                .notifications.push(notification);
        }
        for (peer, plan) in peers {
            if !dispatches.lock().await.insert(peer.clone()) {
                continue;
            }
            let state_for_poll = state.clone();
            let polls_for_task = wake_polls.clone();
            let cleanup = ActiveWakePoll::new(dispatches.clone(), peer);
            wake_tasks.spawn(async move {
                let _cleanup = cleanup;
                let requests = dispatch_wake_notifications(&state_for_poll.push, plan).await;
                let mut polls = JoinSet::new();
                for request in requests {
                    let key = format!("{}:{}", request.counterparty, request.user_channel_id);
                    if !polls_for_task.lock().await.insert(key) {
                        continue;
                    }
                    polls.spawn(poll_for_wake_settlement(
                        state_for_poll.clone(), polls_for_task.clone(), request,
                    ));
                }
                while polls.join_next().await.is_some() {}
            });
        }
    }
}

pub(crate) async fn dispatch_wake_notifications(
    push: &Arc<Mutex<crate::push::PushService>>,
    plan: StabilityTickPlan,
) -> Vec<WakeSettlementRequest> {
    let mut wake_requests = Vec::new();
    for notification in plan.notifications {
        let (sent, recent) = {
            let mut service = push.lock().await;
            let sent = service
                .notify(&notification.node_id, &notification.direction)
                .await;
            let recent = service.has_recent_notification(&notification.node_id);
            (sent, recent)
        };
        if (sent || recent) && notification.direction == "lsp_to_user" {
            if let Some(request) = notification.settlement {
                wake_requests.push(request);
            }
        }
    }
    wake_requests
}

fn wake_channel_is_usable(channel: &Channel, request: &WakeSettlementRequest) -> bool {
    parse_user_channel_id(&channel.user_channel_id) == Some(request.user_channel_id)
        && channel.counterparty_node_id == request.counterparty
        && channel.is_usable
}

/// Removes a channel from the active-poll set even if the task is cancelled while awaiting an
/// RPC.
struct ActiveWakePoll {
    active_polls: Arc<Mutex<HashSet<String>>>,
    key: Option<String>,
}

impl ActiveWakePoll {
    fn new(active_polls: Arc<Mutex<HashSet<String>>>, key: String) -> Self {
        Self {
            active_polls,
            key: Some(key),
        }
    }

    async fn release(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        self.active_polls.lock().await.remove(&key);
    }
}

impl Drop for ActiveWakePoll {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        if let Ok(mut active) = self.active_polls.try_lock() {
            active.remove(&key);
            return;
        }
        let active_polls = self.active_polls.clone();
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            active_polls.lock().await.remove(&key);
        });
    }
}

/// After an `lsp_to_user` push, watch the logical channel that triggered the push. Once LDK Server
/// reports that logical channel usable, run settlement for all eligible stable channels on the same
/// peer rather than waiting for another node-level push cooldown window.
async fn poll_for_wake_settlement(
    state: AppState,
    active_polls: Arc<Mutex<HashSet<String>>>,
    request: WakeSettlementRequest,
) {
    let poll_key = format!("{}:{}", request.counterparty, request.user_channel_id);
    let mut cleanup = ActiveWakePoll::new(active_polls, poll_key);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(WAKE_SETTLEMENT_POLL_TIMEOUT_SECS);
    stable_channels::audit::audit_event(
        "STABILITY_WAKE_POLL_STARTED",
        serde_json::json!({
            "user_channel_id": request.user_channel_id.to_string(),
            "channel_id": request.channel_id.as_str(),
            "node_id": request.counterparty.as_str(),
            "timeout_secs": WAKE_SETTLEMENT_POLL_TIMEOUT_SECS,
        }),
    );

    let mut became_usable = false;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let result = timeout(
            remaining,
            state.ldk_server.list_channels(ListChannelsRequest {}),
        )
        .await;
        match result {
            Ok(Ok(response)) => {
                if response.channels.iter().any(|channel| wake_channel_is_usable(channel, &request)) {
                    became_usable = true;
                    break;
                }
            }
            Ok(Err(error)) => {
                warn!(
                    "[stability_tick] wake poll list_channels failed for {}: {}",
                    request.user_channel_id, error
                );
            }
            Err(_) => {
                warn!(
                    "[stability_tick] wake poll list_channels timed out for {}",
                    request.user_channel_id
                );
                break;
            }
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(std::cmp::min(
            Duration::from_secs(WAKE_SETTLEMENT_POLL_INTERVAL_SECS),
            remaining,
        ))
        .await;
    }

    if became_usable {
        stable_channels::audit::audit_event(
            "STABILITY_WAKE_POLL_ONLINE",
            serde_json::json!({
                "user_channel_id": request.user_channel_id.to_string(),
                "channel_id": request.channel_id.as_str(),
                "node_id": request.counterparty.as_str(),
                "elapsed_ms": started.elapsed().as_millis(),
            }),
        );
        let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
        if btc_price > 0.0 {
            let plan = {
                let mut mgr = state.stable_manager.lock().await;
                mgr.run_tick_plan(
                    state.ldk_server.as_ref() as &dyn LdkServerCalls,
                    btc_price,
                    None,
                    Some(request.counterparty.as_str()),
                )
                .await
            };
            let _ = dispatch_wake_notifications(&state.push, plan).await;
        } else {
            warn!(
                "[stability_tick] wake poll found {} online but price cache is cold",
                request.user_channel_id
            );
        }
    } else {
        stable_channels::audit::audit_event(
            "STABILITY_WAKE_POLL_TIMEOUT",
            serde_json::json!({
                "user_channel_id": request.user_channel_id.to_string(),
                "channel_id": request.channel_id.as_str(),
                "node_id": request.counterparty.as_str(),
                "timeout_secs": WAKE_SETTLEMENT_POLL_TIMEOUT_SECS,
            }),
        );
    }

    cleanup.release().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_follows_logical_channel_after_physical_id_changes() {
        let request = WakeSettlementRequest {
            user_channel_id: 42,
            channel_id: "old-funding-channel".into(),
            counterparty: "peer".into(),
        };
        let mut channel = Channel {
            user_channel_id: "42".into(),
            channel_id: "new-funding-channel".into(),
            counterparty_node_id: "peer".into(),
            is_usable: true,
            ..Default::default()
        };
        assert!(wake_channel_is_usable(&channel, &request));
        channel.counterparty_node_id = "other-peer".into();
        assert!(!wake_channel_is_usable(&channel, &request));
        channel.counterparty_node_id = "peer".into();
        channel.user_channel_id = "43".into();
        assert!(!wake_channel_is_usable(&channel, &request));
        channel.user_channel_id = "42".into();
        channel.is_usable = false;
        assert!(!wake_channel_is_usable(&channel, &request));
    }

    #[tokio::test]
    async fn active_wake_poll_cleanup_removes_cancelled_channel() {
        let active = Arc::new(Mutex::new(HashSet::from(["channel".to_string()])));
        {
            let _cleanup = ActiveWakePoll::new(active.clone(), "channel".to_string());
        }
        tokio::task::yield_now().await;
        assert!(!active.lock().await.contains("channel"));
    }
}
