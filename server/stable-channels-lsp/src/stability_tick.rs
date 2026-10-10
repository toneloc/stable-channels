//! Periodic stability tick: every STABILITY_CHECK_INTERVAL_SECS, run_tick_plan detects USD drift and either sends a stability payment or wakes the offline peer.
//! A peer woken for an `lsp_to_user` top-up is then watched so the payment goes out as soon as it reconnects.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ldk_server_client::ldk_server_grpc::api::ListChannelsRequest;
use ldk_server_client::ldk_server_grpc::types::Channel;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinSet;
use tokio::time::{timeout, MissedTickBehavior};
use tracing::{info, warn};

use crate::push::PushService;
use crate::stable_manager::{
    parse_user_channel_id, LdkServerCalls, TickScope, WakeNotificationRequest,
    WakeSettlementRequest,
};
use crate::state::AppState;

// Allow time for FCM/APNs delivery, mobile service startup, LDK startup, and channel
// reconnection before giving up on the wake-triggered settlement attempt.
const WAKE_WATCH_TIMEOUT_SECS: u64 = 60;
const WAKE_WATCH_INTERVAL_SECS: u64 = 1;
/// Bound on one list_channels poll, so a stalled call costs one attempt and not the whole watch.
const WAKE_WATCH_RPC_TIMEOUT_SECS: u64 = 5;

pub fn spawn(state: AppState) {
    let (wakes, plans) = mpsc::unbounded_channel();
    tokio::spawn(watch_wakes(state.clone(), plans));
    tokio::spawn(run(state, wakes));
}

async fn run(state: AppState, wakes: mpsc::UnboundedSender<Vec<WakeNotificationRequest>>) {
    let interval_secs = stable_channels::constants::STABILITY_CHECK_INTERVAL_SECS;
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    info!("[stability_tick] running every {}s", interval_secs);
    loop {
        ticker.tick().await;
        let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
        if btc_price <= 0.0 {
            warn!("[stability_tick] price cache cold; skipping");
            // Top-up outcomes need no price, so a cold cache must not leave a claimed one unbooked.
            state
                .stable_manager
                .lock()
                .await
                .resolve_top_ups_in_flight(state.ldk_server.as_ref() as &dyn LdkServerCalls)
                .await;
            continue;
        }
        let plan = {
            let mut mgr = state.stable_manager.lock().await;
            mgr.reconcile_if_empty(state.ldk_server.as_ref() as &dyn LdkServerCalls, btc_price)
                .await;
            mgr.run_tick_plan(
                state.ldk_server.as_ref() as &dyn LdkServerCalls,
                btc_price,
                TickScope::All,
            )
            .await
        };
        // Pushes go out on the watcher task so a slow provider never delays the next tick.
        if !plan.notifications.is_empty() && wakes.send(plan.notifications).is_err() {
            warn!("[stability_tick] wake watcher stopped; notifications dropped");
        }
    }
}

/// One peer's wake for a tick: a single push, plus the channels owed an `lsp_to_user` top-up.
#[derive(Debug, PartialEq)]
pub(crate) struct PeerWake {
    pub node_id: String,
    pub direction: String,
    pub channels: Vec<WakeSettlementRequest>,
}

/// Collapse a tick's notifications to one wake per peer. The push cooldown is node-scoped, so a
/// wallet-driven `user_to_lsp` wake takes the single push and `lsp_to_user` siblings ride on it.
fn group_by_peer(notifications: Vec<WakeNotificationRequest>) -> Vec<PeerWake> {
    let mut wakes: Vec<PeerWake> = Vec::new();
    for notification in notifications {
        let position = wakes.iter().position(|wake| wake.node_id == notification.node_id);
        let wake = match position {
            Some(index) => &mut wakes[index],
            None => {
                wakes.push(PeerWake {
                    node_id: notification.node_id,
                    direction: notification.direction.clone(),
                    channels: Vec::new(),
                });
                wakes.last_mut().expect("just pushed")
            }
        };
        if notification.direction == "user_to_lsp" {
            wake.direction = notification.direction;
        }
        wake.channels.extend(notification.settlement);
    }
    wakes
}

/// Push the peer. Returns the wake to watch only when the provider accepted this push and a
/// channel is owed a top-up; a peer that was not woken now is left to the regular tick.
async fn dispatch_peer_wake(push: &Arc<Mutex<PushService>>, wake: PeerWake) -> Option<PeerWake> {
    let sent = crate::push::notify(push, &wake.node_id, &wake.direction).await;
    (sent && !wake.channels.is_empty()).then_some(wake)
}

/// Push every peer in a tick plan and return the wakes a watcher would follow.
#[cfg(test)]
pub(crate) async fn dispatch_wakes(
    push: &Arc<Mutex<PushService>>,
    notifications: Vec<WakeNotificationRequest>,
) -> Vec<PeerWake> {
    let mut watched = Vec::new();
    for wake in group_by_peer(notifications) {
        watched.extend(dispatch_peer_wake(push, wake).await);
    }
    watched
}

struct WakeWatch {
    started: Instant,
    deadline: Instant,
    channels: Vec<WakeSettlementRequest>,
}

enum WakeOutcome {
    Online(WakeWatch),
    TimedOut(WakeWatch),
}

/// Peers with a push in progress and peers being watched for a reconnect, keyed by node id.
#[derive(Default)]
struct WakeWatcher {
    dispatching: HashSet<String>,
    watching: HashMap<String, WakeWatch>,
}

impl WakeWatcher {
    /// The wakes to push now. A peer already being pushed or watched is skipped.
    fn accept(&mut self, notifications: Vec<WakeNotificationRequest>) -> Vec<PeerWake> {
        let mut accepted = Vec::new();
        for wake in group_by_peer(notifications) {
            if self.watching.contains_key(&wake.node_id) {
                // No wake is lost, whatever its direction: the push that started the watch holds the peer's push cooldown.
                info!("[stability_tick] {} is already being watched; wake skipped", wake.node_id);
                continue;
            }
            if self.dispatching.insert(wake.node_id.clone()) {
                accepted.push(wake);
            }
        }
        accepted
    }

    /// A push finished. Start watching the peer when there is a top-up to deliver.
    fn dispatched(&mut self, node_id: &str, watch: Option<PeerWake>, now: Instant) {
        self.dispatching.remove(node_id);
        let Some(wake) = watch else {
            return;
        };
        for request in &wake.channels {
            stable_channels::audit::audit_event("STABILITY_WAKE_POLL_STARTED", wake_detail(request, "timeout_secs", WAKE_WATCH_TIMEOUT_SECS));
        }
        self.watching.insert(
            wake.node_id,
            WakeWatch {
                started: now,
                deadline: now + Duration::from_secs(WAKE_WATCH_TIMEOUT_SECS),
                channels: wake.channels,
            },
        );
    }

    /// Stop watching every peer that has a watched channel usable or has run out of time.
    fn poll(&mut self, channels: &[Channel], now: Instant) -> Vec<WakeOutcome> {
        let finished: Vec<(String, bool)> = self
            .watching
            .iter()
            .filter_map(|(node_id, watch)| {
                let online = watch
                    .channels
                    .iter()
                    .any(|request| channels.iter().any(|c| wake_channel_is_usable(c, request)));
                (online || now >= watch.deadline).then(|| (node_id.clone(), online))
            })
            .collect();
        finished
            .into_iter()
            .filter_map(|(node_id, online)| {
                let watch = self.watching.remove(&node_id)?;
                Some(if online { WakeOutcome::Online(watch) } else { WakeOutcome::TimedOut(watch) })
            })
            .collect()
    }
}

/// Match on the logical channel: a splice can change the physical channel id while a peer is away.
fn wake_channel_is_usable(channel: &Channel, request: &WakeSettlementRequest) -> bool {
    parse_user_channel_id(&channel.user_channel_id) == Some(request.user_channel_id)
        && channel.counterparty_node_id == request.counterparty
        && channel.is_usable
}

fn wake_detail(request: &WakeSettlementRequest, field: &str, value: u64) -> serde_json::Value {
    serde_json::json!({
        "user_channel_id": request.user_channel_id.to_string(),
        "channel_id": request.channel_id.as_str(),
        "node_id": request.counterparty.as_str(),
        field: value,
    })
}

/// Owns every wake: sends the pushes off the tick's task, then polls LDK Server once a second,
/// only while a woken peer is outstanding, and settles each peer the moment it is usable.
async fn watch_wakes(
    state: AppState,
    mut plans: mpsc::UnboundedReceiver<Vec<WakeNotificationRequest>>,
) {
    let mut watcher = WakeWatcher::default();
    let mut pushes: JoinSet<(String, Option<PeerWake>)> = JoinSet::new();
    let mut pushing: HashMap<tokio::task::Id, String> = HashMap::new();
    let mut poll = tokio::time::interval(Duration::from_secs(WAKE_WATCH_INTERVAL_SECS));
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let notifications = tokio::select! {
            received = plans.recv() => match received {
                Some(notifications) => notifications,
                None => return,
            },
            Some(joined) = pushes.join_next_with_id(), if !pushes.is_empty() => {
                match joined {
                    Ok((id, (node_id, watch))) => {
                        pushing.remove(&id);
                        watcher.dispatched(&node_id, watch, Instant::now());
                    }
                    Err(error) => {
                        warn!("[stability_tick] wake push task failed: {}", error);
                        if let Some(node_id) = pushing.remove(&error.id()) {
                            watcher.dispatched(&node_id, None, Instant::now());
                        }
                    }
                }
                continue;
            }
            _ = poll.tick(), if !watcher.watching.is_empty() => {
                settle_reconnected(&state, &mut watcher).await;
                continue;
            }
        };
        for wake in watcher.accept(notifications) {
            let push = state.push.clone();
            let node_id = wake.node_id.clone();
            let task = pushes.spawn(async move {
                let node_id = wake.node_id.clone();
                (node_id, dispatch_peer_wake(&push, wake).await)
            });
            pushing.insert(task.id(), node_id);
        }
    }
}

/// One poll of the watched peers. Each peer found usable gets one settlement pass over its channels.
async fn settle_reconnected(state: &AppState, watcher: &mut WakeWatcher) {
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    let rpc_timeout = Duration::from_secs(WAKE_WATCH_RPC_TIMEOUT_SECS);
    let channels = match timeout(rpc_timeout, ldk.list_channels(ListChannelsRequest {})).await {
        Ok(Ok(response)) => response.channels,
        Ok(Err(error)) => {
            warn!("[stability_tick] wake watch list_channels failed: {}", error);
            Vec::new()
        }
        Err(_) => {
            warn!("[stability_tick] wake watch list_channels timed out");
            Vec::new()
        }
    };
    for outcome in watcher.poll(&channels, Instant::now()) {
        let watch = match outcome {
            WakeOutcome::TimedOut(watch) => {
                for request in &watch.channels {
                    stable_channels::audit::audit_event("STABILITY_WAKE_POLL_TIMEOUT", wake_detail(request, "timeout_secs", WAKE_WATCH_TIMEOUT_SECS));
                }
                continue;
            }
            WakeOutcome::Online(watch) => watch,
        };
        let elapsed_ms = watch.started.elapsed().as_millis() as u64;
        for request in &watch.channels {
            stable_channels::audit::audit_event("STABILITY_WAKE_POLL_ONLINE", wake_detail(request, "elapsed_ms", elapsed_ms));
        }
        let Some(peer) = watch.channels.first().map(|request| request.counterparty.as_str()) else {
            continue;
        };
        let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
        if btc_price <= 0.0 {
            warn!("[stability_tick] {} reconnected but the price cache is cold", peer);
            continue;
        }
        let mut mgr = state.stable_manager.lock().await;
        // Wakes this pass asks for are dropped: the push that started the watch holds the peer's cooldown, and the regular tick asks again.
        mgr.run_tick_plan(ldk, btc_price, TickScope::WokenPeer(peer)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::testing::{service, FakeSender};

    fn request(user_channel_id: u128, peer: &str) -> WakeSettlementRequest {
        WakeSettlementRequest {
            user_channel_id,
            channel_id: format!("channel-{user_channel_id}"),
            counterparty: peer.into(),
        }
    }

    fn lsp_to_user(user_channel_id: u128, peer: &str) -> WakeNotificationRequest {
        WakeNotificationRequest {
            node_id: peer.into(),
            direction: "lsp_to_user".into(),
            settlement: Some(request(user_channel_id, peer)),
        }
    }

    fn user_to_lsp(peer: &str) -> WakeNotificationRequest {
        WakeNotificationRequest {
            node_id: peer.into(),
            direction: "user_to_lsp".into(),
            settlement: None,
        }
    }

    fn channel(user_channel_id: u128, peer: &str, usable: bool) -> Channel {
        Channel {
            user_channel_id: user_channel_id.to_string(),
            channel_id: "current-funding-channel".into(),
            counterparty_node_id: peer.into(),
            is_usable: usable,
            ..Default::default()
        }
    }

    fn watching(watcher: &mut WakeWatcher, notifications: Vec<WakeNotificationRequest>, now: Instant) {
        for wake in watcher.accept(notifications) {
            let node_id = wake.node_id.clone();
            watcher.dispatched(&node_id, Some(wake), now);
        }
    }

    #[test]
    fn wake_follows_logical_channel_after_physical_id_changes() {
        let request = request(42, "peer");
        assert!(wake_channel_is_usable(&channel(42, "peer", true), &request));
        assert!(!wake_channel_is_usable(&channel(42, "other-peer", true), &request));
        assert!(!wake_channel_is_usable(&channel(43, "peer", true), &request));
        assert!(!wake_channel_is_usable(&channel(42, "peer", false), &request));
    }

    #[test]
    fn one_wake_per_peer_prefers_the_wallet_direction_in_either_order() {
        for notifications in [
            vec![lsp_to_user(2, "peer"), user_to_lsp("peer"), lsp_to_user(3, "other")],
            vec![user_to_lsp("peer"), lsp_to_user(3, "other"), lsp_to_user(2, "peer")],
        ] {
            let mut wakes = group_by_peer(notifications);
            wakes.sort_by(|a, b| b.node_id.cmp(&a.node_id));
            assert_eq!(
                wakes,
                vec![
                    PeerWake {
                        node_id: "peer".into(),
                        direction: "user_to_lsp".into(),
                        channels: vec![request(2, "peer")],
                    },
                    PeerWake {
                        node_id: "other".into(),
                        direction: "lsp_to_user".into(),
                        channels: vec![request(3, "other")],
                    },
                ]
            );
        }
    }

    #[tokio::test]
    async fn only_a_push_accepted_now_starts_a_watch() {
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &["peer"]);

        let first = dispatch_wakes(&push, vec![lsp_to_user(1, "peer")]).await;
        assert_eq!(first.len(), 1);
        // The next tick finds the peer still offline: the cooldown suppresses the push, and a
        // push sent minutes ago is no reason to start polling again.
        let second = dispatch_wakes(&push, vec![lsp_to_user(1, "peer")]).await;
        assert!(second.is_empty());
        // A wallet-driven wake has nothing for the LSP to deliver on reconnect.
        let wallet = dispatch_wakes(&push, vec![user_to_lsp("unregistered")]).await;
        assert!(wallet.is_empty());
        assert_eq!(sender.sent(), vec![("peer".to_string(), "lsp_to_user".to_string())]);
    }

    #[tokio::test]
    async fn rejected_push_starts_no_watch() {
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(false), dir.path(), &["peer"]);

        assert!(dispatch_wakes(&push, vec![lsp_to_user(1, "peer")]).await.is_empty());
    }

    #[test]
    fn watcher_skips_a_peer_already_being_pushed_or_watched() {
        let mut watcher = WakeWatcher::default();
        let now = Instant::now();

        assert_eq!(watcher.accept(vec![lsp_to_user(1, "peer")]).len(), 1);
        assert!(watcher.accept(vec![lsp_to_user(1, "peer")]).is_empty(), "push in progress");

        watcher.dispatched("peer", Some(group_by_peer(vec![lsp_to_user(1, "peer")]).remove(0)), now);
        assert!(watcher.accept(vec![user_to_lsp("peer")]).is_empty(), "already watched");

        // A push that was not accepted frees the peer for the next tick.
        assert_eq!(watcher.accept(vec![lsp_to_user(2, "other")]).len(), 1);
        watcher.dispatched("other", None, now);
        assert!(!watcher.watching.contains_key("other"));
        assert_eq!(watcher.accept(vec![lsp_to_user(2, "other")]).len(), 1);
    }

    #[test]
    fn reconnect_ends_the_watch_once_for_the_whole_peer() {
        let mut watcher = WakeWatcher::default();
        let now = Instant::now();
        watching(&mut watcher, vec![lsp_to_user(1, "peer"), lsp_to_user(2, "peer")], now);
        watching(&mut watcher, vec![lsp_to_user(3, "other")], now);

        let offline = [channel(1, "peer", false), channel(2, "peer", false), channel(3, "other", false)];
        assert!(watcher.poll(&offline, now).is_empty());

        let online = [channel(1, "peer", false), channel(2, "peer", true), channel(3, "other", false)];
        let outcomes = watcher.poll(&online, now);
        assert_eq!(outcomes.len(), 1, "two watched channels of one peer are one settlement pass");
        assert!(matches!(&outcomes[0], WakeOutcome::Online(watch) if watch.channels.len() == 2));
        assert!(!watcher.watching.contains_key("peer"));
        assert!(watcher.watching.contains_key("other"));
        assert!(watcher.poll(&online, now).is_empty());
    }

    #[test]
    fn watch_times_out_and_frees_the_peer() {
        let mut watcher = WakeWatcher::default();
        let now = Instant::now();
        watching(&mut watcher, vec![lsp_to_user(1, "peer")], now);
        let offline = [channel(1, "peer", false)];

        let before = now + Duration::from_secs(WAKE_WATCH_TIMEOUT_SECS - 1);
        assert!(watcher.poll(&offline, before).is_empty());
        // A failed list_channels poll is an empty snapshot; the deadline still applies.
        let after = now + Duration::from_secs(WAKE_WATCH_TIMEOUT_SECS);
        let outcomes = watcher.poll(&[], after);
        assert!(matches!(outcomes.as_slice(), [WakeOutcome::TimedOut(_)]));
        assert!(watcher.watching.is_empty());
        assert_eq!(watcher.accept(vec![lsp_to_user(1, "peer")]).len(), 1);
    }
}
