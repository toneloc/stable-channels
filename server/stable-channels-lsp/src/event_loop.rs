//! Long-running SubscribeEvents loop: connects to LDK Server's event stream, reconnects with exponential backoff, and dispatches each EventEnvelope to its handler.

use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
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

/// The reader with the production limit, for tests; dropping the JoinSet stops it.
#[cfg(test)]
pub(crate) fn buffer_events(source: impl EventSource) -> (tokio::task::JoinSet<()>, EventBuffer) {
    let (reader, events, _) = buffer_events_with_health(source, BUFFER_LIMIT);
    (reader, events)
}

/// Most events, and encoded bytes, held while dispatch is delayed; the reader can't wait instead, since LDK Server drops events for a reader that falls behind.
const BUFFER_LIMIT: BufferLimit = BufferLimit { events: 100_000, bytes: 64 * 1024 * 1024 };

#[derive(Debug, Clone, Copy)]
struct BufferLimit {
    events: usize,
    bytes: usize,
}

impl BufferLimit {
    fn half(self) -> Self {
        Self { events: self.events / 2, bytes: self.bytes / 2 }
    }
}

/// What the buffered events hold against the limit; a hole being dropped holds one event slot for its marker.
#[derive(Default)]
struct Held {
    events: AtomicUsize,
    bytes: AtomicUsize,
    /// Wakes a reader holding a hole when dispatch frees room, so the hole is queued even if no event follows it.
    freed: tokio::sync::Notify,
}

impl Held {
    /// An empty buffer takes any one event, so an oversized event can't stop the stream for good.
    fn has_room(&self, events: usize, bytes: usize, limit: BufferLimit) -> bool {
        events == 0
            || (events < limit.events && self.bytes.load(Ordering::Acquire).saturating_add(bytes) <= limit.bytes)
    }
}

/// What the reader queues for dispatch.
#[derive(Debug)]
pub(crate) enum Buffered {
    Event(EventItem),
    /// The buffer was full, so this many events were dropped here.
    Dropped(u64),
}

#[cfg(test)]
impl Buffered {
    pub(crate) fn event(self) -> EventItem {
        match self {
            Buffered::Event(item) => item,
            Buffered::Dropped(count) => panic!("expected an event, found {count} dropped"),
        }
    }
}

/// Events read off the subscription and not dispatched yet.
pub(crate) struct EventBuffer {
    receiver: tokio::sync::mpsc::UnboundedReceiver<(usize, Buffered)>,
    held: Arc<Held>,
}

impl EventBuffer {
    pub(crate) async fn recv(&mut self) -> Option<Buffered> {
        let (bytes, item) = self.receiver.recv().await?;
        self.held.events.fetch_sub(1, Ordering::SeqCst);
        self.held.bytes.fetch_sub(bytes, Ordering::AcqRel);
        self.held.freed.notify_one();
        Some(item)
    }

    /// Counts a hole still being dropped, so no checkpoint passes it before its reconcile.
    pub(crate) fn is_empty(&self) -> bool {
        self.held.events.load(Ordering::SeqCst) == 0
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.receiver.len()
    }
}

/// Drop events rather than hold more; the hole's reconcile settles payment outcomes and channels, but trades and forward deductions among them are not replayed.
fn buffer_full(events: usize, bytes: usize, limit: BufferLimit) {
    warn!("[event_loop] event buffer full ({} events, {} bytes); dropping events until fewer than half are held", events, bytes);
    stable_channels::audit::audit_event(
        "EVENT_BUFFER_FULL",
        serde_json::json!({ "events": events, "bytes": bytes, "max_events": limit.events, "max_bytes": limit.bytes }),
    );
}

/// Queue a hole's marker in the slot it has held since its first drop; false once dispatch is gone.
fn queue_hole(sender: &tokio::sync::mpsc::UnboundedSender<(usize, Buffered)>, dropped: &mut u64) -> bool {
    stable_channels::audit::audit_event("EVENT_BUFFER_RESUMED", serde_json::json!({ "dropped": *dropped }));
    let queued = sender.send((0, Buffered::Dropped(*dropped))).is_ok();
    *dropped = 0;
    queued
}

struct ReaderActivity(Arc<AtomicBool>);

impl Drop for ReaderActivity {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn buffer_events_with_health(
    mut source: impl EventSource,
    limit: BufferLimit,
) -> (tokio::task::JoinSet<()>, EventBuffer, Arc<AtomicBool>) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let held = Arc::new(Held::default());
    let counted = Arc::clone(&held);
    let active = Arc::new(AtomicBool::new(false));
    let activity = ReaderActivity(Arc::clone(&active));
    let mut reader = tokio::task::JoinSet::new();
    reader.spawn(async move {
        let activity = activity;
        activity.0.store(true, Ordering::Release);
        let mut dropped: u64 = 0;
        loop {
            // The same read stays pending across wake-ups, so nothing is cancelled.
            let mut next = source.next_event();
            let item = loop {
                tokio::select! {
                    // An event that ends a hole queues the hole ahead of itself, so an event comes first when both are ready.
                    biased;
                    item = &mut next => break item,
                    _ = counted.freed.notified(), if dropped > 0 => {
                        if counted.has_room(counted.events.load(Ordering::SeqCst) - 1, 0, limit.half()) && !queue_hole(&sender, &mut dropped) {
                            return;
                        }
                    },
                }
            };
            let Some(item) = item else { break };
            let (bytes, item) = match item {
                Ok(envelope) => {
                    let bytes = prost::Message::encoded_len(&envelope);
                    // Once full, take events again only when fewer than half are held, so each hole costs one reconcile per half a buffer handled.
                    let room = if dropped == 0 { limit } else { limit.half() };
                    let fits = loop {
                        let seen = counted.events.load(Ordering::SeqCst);
                        if counted.has_room(seen - usize::from(dropped > 0), bytes, room) {
                            break true;
                        }
                        // Hold the marker's slot from the first drop, unless an event was handled since `seen`, so no checkpoint can pass the hole.
                        if dropped > 0 || counted.events.compare_exchange(seen, seen + 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                            break false;
                        }
                    };
                    if !fits {
                        if dropped == 0 {
                            buffer_full(counted.events.load(Ordering::SeqCst) - 1, counted.bytes.load(Ordering::Acquire), limit);
                        }
                        dropped += 1;
                        continue;
                    }
                    (bytes, Ok(envelope))
                },
                Err(error) => (0, Err(error)),
            };
            // A stream error leaves the hole, still holding its slot, to the reconnect's gap reconcile.
            if dropped > 0 && item.is_ok() && !queue_hole(&sender, &mut dropped) {
                break;
            }
            let failed = item.is_err();
            if failed {
                activity.0.store(false, Ordering::Release);
            }
            counted.events.fetch_add(1, Ordering::SeqCst);
            counted.bytes.fetch_add(bytes, Ordering::AcqRel);
            if sender.send((bytes, Buffered::Event(item))).is_err() || failed {
                break;
            }
        }
    });
    (reader, EventBuffer { receiver, held }, active)
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
    events: &EventBuffer,
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
    /// The stream failed: subscribe again.
    Reconnect,
    /// A write for this event failed: reconcile from LDK Server's records before the next event, keeping the stream.
    Reconcile,
    /// LDK Server didn't answer a read this event needs: handle it again after a pause, keeping the stream.
    Retry,
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
const PINNED_LDK_SERVER_REV: &str = "d7e7a2e7";

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
        let (_reader, mut events, active) = buffer_events_with_health(stream, BUFFER_LIMIT);
        info!("[event_loop] subscribed");
        let node_info = state
            .ldk_server
            .get_node_info(ldk_server_client::ldk_server_grpc::api::GetNodeInfoRequest {})
            .await;
        if let Ok(info) = &node_info {
            state.funding_outpoints.remember_lsp_node_id(&info.node_id);
        }
        let ldk_server_version = match node_info {
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
        serve(&mut events, &active, &state.stable_manager, state.db.as_ref(), state.ldk_server.as_ref(), &mut health).await;
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

/// Longest wait between reconciliation passes while terminal settlement outcomes can't be applied, and between tries of an event LDK Server didn't answer for.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

/// Reconcile the gap, then dispatch buffered events until the subscription ends or fails.
async fn serve(
    events: &mut EventBuffer,
    active: &AtomicBool,
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    health: &mut StreamHealth,
) {
    reconcile_until_safe(manager, db, ldk, health, "stream_connected").await;
    checkpoint_if_ready(health, db, active, events);
    let mut heartbeat = health_interval(STREAM_HEALTH_INTERVAL);
    loop {
        tokio::select! {
            item = events.recv() => {
                match item {
                    None => return,
                    Some(Buffered::Dropped(_)) => reconcile_in_place(manager, db, ldk, health, "events_dropped").await,
                    Some(Buffered::Event(item)) => {
                        if !handle(item, manager, db, ldk, health).await {
                            return;
                        }
                    },
                }
            },
            _ = heartbeat.tick() => {
                checkpoint_if_ready(health, db, active, events);
            },
        }
    }
}

/// Longest an event waits for LDK Server: as long as a wallet waits for a trade's result.
const MAX_EVENT_RETRY: Duration = Duration::from_secs(stable_channels::constants::TRADE_RESULT_TIMEOUT_SECS);

/// Dispatch one event, trying again while LDK Server doesn't answer for it; false when the stream failed and must be subscribed again.
async fn handle(
    item: EventItem,
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    health: &mut StreamHealth,
) -> bool {
    let started = tokio::time::Instant::now();
    let mut retry = Duration::from_secs(1);
    loop {
        match dispatch(item.clone(), manager, db, ldk).await {
            DispatchOutcome::Continue => return true,
            DispatchOutcome::Reconnect => return false,
            DispatchOutcome::Reconcile => {
                reconcile_in_place(manager, db, ldk, health, "dispatch_write_failed").await;
                return true;
            },
            DispatchOutcome::Retry if started.elapsed() < MAX_EVENT_RETRY => {
                warn!("[event_loop] LDK Server didn't answer for an event; trying it again in {:?}", retry);
                tokio::time::sleep(retry).await;
                retry = std::cmp::min(retry * 2, MAX_RETRY_WAIT);
            },
            DispatchOutcome::Retry => {
                warn!("[event_loop] LDK Server didn't answer for an event within {:?}; moving on", MAX_EVENT_RETRY);
                stable_channels::audit::audit_event(
                    "EVENT_RETRY_ABANDONED",
                    serde_json::json!({ "payment_id": payment_of(&item), "waited_secs": started.elapsed().as_secs() }),
                );
                return true;
            },
        }
    }
}

/// The received payment an event reports, the only kind that is tried again.
fn payment_of(item: &EventItem) -> Option<&str> {
    match item.as_ref().ok()?.event.as_ref()? {
        EventVariant::PaymentReceived(event) => event.payment.as_ref().map(|payment| payment.payment_id.as_str()),
        _ => None,
    }
}

/// Reopen the gap from the last checkpoint and reconcile it, keeping the stream; persisted first, so a crash during recovery still bounds the gap.
async fn reconcile_in_place(
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    health: &mut StreamHealth,
    trigger: &str,
) {
    *health = StreamHealth::new(begin_gap_with_retry(db).await);
    reconcile_until_safe(manager, db, ldk, health, trigger).await;
}

/// Reconcile until every terminal settlement outcome is applied, waiting up to a minute between passes; the subscription keeps reading meanwhile.
async fn reconcile_until_safe(
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    health: &mut StreamHealth,
    trigger: &str,
) {
    let mut retry = Duration::from_secs(1);
    while !reconcile(manager, db, ldk, health, trigger).await {
        warn!(
            "[event_loop] terminal settlement reconciliation incomplete; retrying in {:?} before live dispatch",
            retry
        );
        tokio::time::sleep(retry).await;
        retry = std::cmp::min(retry * 2, MAX_RETRY_WAIT);
    }
}

/// One pass over LDK Server's records for the current gap; true when every terminal settlement outcome was applied.
async fn reconcile(
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    health: &mut StreamHealth,
    trigger: &str,
) -> bool {
    stable_channels::audit::audit_event(
        "RECONCILIATION_STARTED",
        serde_json::json!({
            "correlation_id": health.gap.as_ref().map(|gap| gap.1.as_str()),
            "scopes": ["channels", "payments", "forwards", "peers", "sweeps"],
            "trigger": trigger,
        }),
    );
    // Finish failed accounting writes before hydration or backfill can change the same books; the stream keeps reading meanwhile.
    let mut mgr = StableChannelManager::lock_for_event_with_top_ups_resolved(manager, ldk).await;
    // The books need no price, so the list is reloaded whatever the price cache holds.
    let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
    mgr.reconcile_from_grpc(ldk, btc_price).await;
    drop(mgr);
    let counts = crate::backfill::reconcile_event_history(ldk, db, health.gap.as_ref().map(|gap| gap.0)).await;
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
    let result_recorded = match stable_channels::audit::record_event("RECONCILIATION_RESULT", reconciliation.clone()) {
        Ok(outcome) => outcome.event_id > 0,
        Err(error) => {
            checkpoint_failed("record_reconciliation_result", &error);
            false
        },
    };
    health.reconciliation_finished(&counts, reconciliation, result_recorded);
    counts.settlement_outcomes_safe
}

async fn dispatch(
    item: Result<EventEnvelope, ldk_server_client::error::LdkServerError>,
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
) -> DispatchOutcome {
    let envelope = match item {
        Ok(e) => e,
        Err(e) => {
            warn!("[event_loop] item error: {}", e);
            return DispatchOutcome::Reconnect;
        },
    };
    // Keep this envelope on the stack while an earlier correction is unsaved. Returning or
    // reconnecting here would lose the event because LDK Server does not replay its stream.
    let mut mgr = StableChannelManager::lock_for_event_with_top_ups_resolved(manager, ldk).await;
    let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
    let outcome = dispatch_event(envelope.event, &mut mgr, db, ldk, btc_price).await;
    drop(mgr);
    if outcome == DispatchOutcome::Continue {
        // The last event may itself have queued a failed accounting correction. Finish that
        // existing retry barrier before idle health can pass it, even if no next event arrives.
        drop(StableChannelManager::lock_for_event(manager, ldk).await);
    }
    outcome
}

/// Whether a record of a channel that hasn't become usable may be written for its peer today.
fn allow_channel_record(mgr: &mut StableChannelManager, peer: Option<&str>) -> bool {
    mgr.record_limits.allow(peer.unwrap_or_default(), crate::record_limits::RecordKind::NeverUsable, crate::revenue::now_secs())
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
                crate::queued_splice::on_channel_closed(db, &e.user_channel_id, crate::revenue::now_secs());
            } else if e.state == ChannelState::Pending as i32 {
                // Whether the channel will become usable isn't known yet, so every opening counts toward the peer's daily limit.
                if allow_channel_record(mgr, e.counterparty_node_id.as_deref()) {
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
                }
            } else if e.state == ChannelState::OpenFailed as i32 {
                if allow_channel_record(mgr, e.counterparty_node_id.as_deref()) {
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
                }
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
            let handled = mgr.handle_payment_received(e.custom_records, payment_id, amount_msat, ldk, btc_price).await;
            if handled == crate::stable_manager::Handled::Retry {
                return DispatchOutcome::Retry;
            }
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
                        return DispatchOutcome::Reconcile;
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
                        return DispatchOutcome::Reconcile;
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
                        return DispatchOutcome::Reconcile;
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
                    return DispatchOutcome::Reconcile;
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
                    return DispatchOutcome::Reconcile;
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
                    return DispatchOutcome::Reconcile;
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
            crate::queued_splice::on_splice_negotiated(db, &e.user_channel_id, &e.new_funding_txo, crate::revenue::now_secs());
        },
        Some(EventVariant::SpliceNegotiationFailed(e)) => {
            stable_channels::audit::audit_event(
                "SPLICE_NEGOTIATION_FAILED",
                crate::channel_audit::splice_failed_audit_data(&e),
            );
            crate::queued_splice::on_splice_negotiation_failed(db, &e.user_channel_id, crate::revenue::now_secs());
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
        let (mut reader, events, active) = buffer_events_with_health(TestSource(receiver), BUFFER_LIMIT);
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

    fn claimable(payment_id: &str) -> EventEnvelope {
        use ldk_server_client::ldk_server_grpc::events::PaymentClaimable;
        use ldk_server_client::ldk_server_grpc::types::Payment;
        EventEnvelope {
            event: Some(EventVariant::PaymentClaimable(PaymentClaimable {
                payment: Some(Payment { payment_id: payment_id.to_owned(), ..Default::default() }),
                ..Default::default()
            })),
        }
    }

    fn payment_id_of(item: EventItem) -> String {
        match item.unwrap().event {
            Some(EventVariant::PaymentClaimable(event)) => event.payment.unwrap().payment_id,
            other => panic!("unexpected event {other:?}"),
        }
    }

    async fn reader_ended(reader: &mut tokio::task::JoinSet<()>) {
        tokio::time::timeout(Duration::from_secs(5), reader.join_next())
            .await
            .expect("the reader stops by itself")
            .unwrap()
            .unwrap();
    }

    /// A subscription that counts how often the reader asked it for an event.
    struct CountingSource(tokio::sync::mpsc::UnboundedReceiver<EventItem>, Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl EventSource for CountingSource {
        async fn next_event(&mut self) -> Option<EventItem> {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.recv().await
        }
    }

    /// Wait until the reader has dealt with `count` events and asks for the next.
    async fn dealt_with(asks: &AtomicUsize, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while asks.load(Ordering::SeqCst) <= count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the reader deals with what it was sent");
    }

    async fn holds(events: &EventBuffer, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while events.len() != count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the reader buffers what it was sent");
    }

    #[tokio::test]
    async fn a_full_buffer_drops_events_only_until_half_of_it_is_handled() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        stable_channels::audit::enable_test_capture();
        let (_reader, mut events, active) = buffer_events_with_health(
            CountingSource(receiver, Arc::clone(&asks)),
            BufferLimit { events: 4, bytes: usize::MAX },
        );
        for n in 0..6 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        dealt_with(&asks, 6).await;
        assert_eq!(events.len(), 4);
        for n in 0..2 {
            assert_eq!(payment_id_of(events.recv().await.unwrap().event()), format!("pay-{n}"));
        }
        // Two of four held is not fewer than half yet.
        sender.send(Ok(claimable("pay-6"))).unwrap();
        dealt_with(&asks, 7).await;
        for n in 2..4 {
            assert_eq!(payment_id_of(events.recv().await.unwrap().event()), format!("pay-{n}"));
        }
        let hole = tokio::time::timeout(Duration::from_secs(5), events.recv()).await;
        assert!(
            matches!(hole, Ok(Some(Buffered::Dropped(3)))),
            "pay-4, pay-5 and pay-6 were dropped, and the hole is queued once half is free with no event after it: {hole:?}"
        );
        assert!(events.is_empty());
        sender.send(Ok(claimable("pay-7"))).unwrap();
        dealt_with(&asks, 8).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-7");
        assert!(active.load(Ordering::Acquire) && !sender.is_closed(), "the subscription stays open");
        let full: Vec<_> = rows.iter().filter(|(name, _)| name == "EVENT_BUFFER_FULL").collect();
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].1["events"], 4);
        let resumed: Vec<_> = rows.iter().filter(|(name, _)| name == "EVENT_BUFFER_RESUMED").collect();
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].1["dropped"], 3);
    }

    #[tokio::test]
    async fn the_byte_limit_counts_encoded_size_and_an_empty_buffer_takes_any_one_event() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap();
        let size = prost::Message::encoded_len(&claimable("pay-0"));
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (mut reader, mut events, _active) = buffer_events_with_health(
            CountingSource(receiver, Arc::clone(&asks)),
            BufferLimit { events: 100, bytes: 2 * size },
        );
        for n in 0..3 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        dealt_with(&asks, 3).await;
        assert_eq!(events.len(), 2, "a third event is over the byte limit");
        sender.send(Err(ldk_server_client::error::LdkServerError::new(
            ldk_server_client::error::LdkServerErrorCode::InternalError, "stream failed",
        ))).unwrap();
        reader_ended(&mut reader).await;
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-0");
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-1");
        assert!(events.recv().await.unwrap().event().is_err(), "a stream that fails while dropping leaves the hole to the reconnect");
        assert!(!events.is_empty(), "the hole still holds its slot, so no checkpoint passes it");
        assert!(events.recv().await.is_none());
        // One event larger than the whole limit still goes through when nothing else waits.
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(Ok(claimable("pay-big"))).unwrap();
        let (mut reader, mut events, active) =
            buffer_events_with_health(TestSource(receiver), BufferLimit { events: 100, bytes: 1 });
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-big");
        assert!(active.load(Ordering::Acquire));
        drop(sender);
        reader_ended(&mut reader).await;
        assert!(events.recv().await.is_none(), "nothing was dropped");
    }

    #[tokio::test]
    async fn the_production_buffer_holds_a_hundred_thousand_events_or_64_mib() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (reader, events, _active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), BUFFER_LIMIT);
        for _ in 0..100_001 {
            sender.send(Ok(EventEnvelope::default())).unwrap();
        }
        dealt_with(&asks, 100_001).await;
        assert_eq!(events.len(), 100_000, "the next event is over the event limit");
        drop((reader, events, sender));
        // Events of exactly 1 MiB encoded: 64 fit and the next is over the byte limit.
        let overhead = prost::Message::encoded_len(&claimable(&"x".repeat(1 << 20))) - (1 << 20);
        let id = "x".repeat((1 << 20) - overhead);
        assert_eq!(prost::Message::encoded_len(&claimable(&id)), 1 << 20);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, events, _active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), BUFFER_LIMIT);
        for _ in 0..65 {
            sender.send(Ok(claimable(&id))).unwrap();
        }
        dealt_with(&asks, 65).await;
        assert_eq!(events.len(), 64);
    }

    #[tokio::test]
    async fn a_hole_is_queued_ahead_of_the_event_that_ends_it() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, _active) = buffer_events_with_health(
            CountingSource(receiver, Arc::clone(&asks)),
            BufferLimit { events: 2, bytes: usize::MAX },
        );
        for n in 0..3 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        dealt_with(&asks, 3).await;
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-0");
        // Free the last held event and send the next before the reader runs, so it finds both at once.
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-1");
        sender.send(Ok(claimable("pay-3"))).unwrap();
        assert!(matches!(events.recv().await.unwrap(), Buffered::Dropped(1)), "pay-2 was dropped");
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-3");
    }

    #[tokio::test]
    async fn a_handled_event_frees_its_room() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (mut reader, mut events, active) =
            buffer_events_with_health(TestSource(receiver), BufferLimit { events: 2, bytes: usize::MAX });
        sender.send(Ok(claimable("pay-0"))).unwrap();
        sender.send(Ok(claimable("pay-1"))).unwrap();
        holds(&events, 2).await;
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-0");
        sender.send(Ok(claimable("pay-2"))).unwrap();
        holds(&events, 2).await;
        assert!(active.load(Ordering::Acquire), "the room pay-0 freed took pay-2");
        drop(sender);
        reader_ended(&mut reader).await;
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-1");
        assert_eq!(payment_id_of(events.recv().await.unwrap().event()), "pay-2");
        assert!(events.recv().await.is_none());
    }

    /// A manager and a second handle on its database, which audit rows go to as well.
    fn serving_fixture() -> (tokio::sync::Mutex<StableChannelManager>, stable_channels::db::Database) {
        let mgr = crate::stable_manager::tests::make_manager();
        let db = stable_channels::db::Database::open(mgr.data_dir()).unwrap();
        stable_channels::audit::set_audit_ledger(db.clone());
        (tokio::sync::Mutex::new(mgr), db)
    }

    /// Every settlement outcome write fails, so a failed payment's event can't be booked.
    async fn fail_settlement_outcomes(manager: &tokio::sync::Mutex<StableChannelManager>) {
        let path = manager.lock().await.data_dir().join(stable_channels::db::DB_FILENAME);
        rusqlite::Connection::open(path).unwrap().execute_batch(
            "CREATE TRIGGER fail_sync_outcome BEFORE UPDATE OF outcome ON settlement_payments
             BEGIN SELECT RAISE(FAIL, 'injected outcome failure'); END;",
        ).unwrap();
    }

    fn failed_payment(payment_id: &str) -> EventEnvelope {
        use ldk_server_client::ldk_server_grpc::events::PaymentFailed;
        use ldk_server_client::ldk_server_grpc::types::Payment;
        EventEnvelope {
            event: Some(EventVariant::PaymentFailed(PaymentFailed {
                payment: Some(Payment { payment_id: payment_id.to_owned(), ..Default::default() }),
                ..Default::default()
            })),
        }
    }

    /// LDK Server keeps every forward, so a recent gap can reconcile completely.
    fn complete_history() -> crate::stable_manager::tests::FakeLdkServer {
        use ldk_server_client::ldk_server_grpc::types::ForwardedPaymentTrackingMode;
        crate::stable_manager::tests::FakeLdkServer::new(vec![]).with_tracking_mode(ForwardedPaymentTrackingMode::Detailed)
    }

    #[tokio::test]
    async fn a_failed_write_reconciles_in_place_and_keeps_the_events_behind_it() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap();
        let (manager, db) = serving_fixture();
        db.record_settlement("sync-1", "sync").unwrap();
        fail_settlement_outcomes(&manager).await;
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(Ok(failed_payment("sync-1"))).unwrap();
        sender.send(Ok(claimable("after-1"))).unwrap();
        sender.send(Ok(claimable("after-2"))).unwrap();
        drop(sender);
        let (_reader, mut events, active) = buffer_events_with_health(TestSource(receiver), BUFFER_LIMIT);
        stable_channels::audit::enable_test_capture();
        tokio::time::timeout(Duration::from_secs(30), serve(&mut events, &active, &manager, &db, &ldk, &mut health))
            .await
            .expect("serve returns once the subscription ends");
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        let failed_write = rows
            .iter()
            .position(|(name, data)| name == "DB_WRITE_FAILED" && data["op"] == "mark_sync_payment_failed")
            .expect("the write failed");
        let in_place = rows
            .iter()
            .position(|(name, data)| name == "RECONCILIATION_STARTED" && data["trigger"] == "dispatch_write_failed")
            .expect("reconciled in place");
        let handled: Vec<usize> = rows.iter().enumerate().filter(|(_, (name, _))| name == "PAYMENT_CLAIMABLE").map(|(at, _)| at).collect();
        assert_eq!(handled.len(), 2, "the events behind the failed write are still handled: {names:?}");
        assert!(failed_write < in_place && in_place < handled[0], "{names:?}");
        assert_eq!(names.iter().filter(|name| **name == "RECONCILIATION_STARTED").count(), 2, "{names:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_write_on_a_healthy_stream_reopens_the_gap_from_its_last_checkpoint() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap();
        let (manager, db) = serving_fixture();
        db.record_settlement("sync-3", "sync").unwrap();
        fail_settlement_outcomes(&manager).await;
        let ldk = complete_history();
        // The stream was healthy: its last gap was reconciled and closed at the last checkpoint.
        let healthy_at = now_millis() as i64 - 1_000;
        let mut health = StreamHealth::new(db.begin_event_stream_gap(healthy_at - 1_000).unwrap());
        health.reconciliation_finished(&complete_counts(), completed_result(), true);
        assert!(health.checkpoint(&db, healthy_at, true, true).unwrap());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(Ok(failed_payment("sync-3"))).unwrap();
        sender.send(Ok(claimable("after"))).unwrap();
        let (_reader, mut events, active) = buffer_events_with_health(TestSource(receiver), BUFFER_LIMIT);
        stable_channels::audit::enable_test_capture();
        // The subscription stays open past a heartbeat, then ends.
        let end = async move {
            tokio::time::sleep(STREAM_HEALTH_INTERVAL + Duration::from_secs(5)).await;
            drop(sender);
        };
        let (served, ()) = tokio::join!(
            tokio::time::timeout(Duration::from_secs(3_600), serve(&mut events, &active, &manager, &db, &ldk, &mut health)),
            end,
        );
        served.expect("serve returns once the subscription ends");
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        let in_place = rows
            .iter()
            .position(|(name, data)| name == "RECONCILIATION_STARTED" && data["trigger"] == "dispatch_write_failed")
            .expect("reconciled in place");
        let result = rows[in_place..]
            .iter()
            .find(|(name, _)| name == "RECONCILIATION_RESULT")
            .expect("the in-place pass recorded its result");
        assert_eq!(result.1["gap_started_ms"], healthy_at, "the gap is persisted from the last checkpoint before reconciling: {names:?}");
        assert_eq!(result.1["status"], "completed", "{names:?}");
        assert!(rows.iter().any(|(name, data)| name == "PAYMENT_CLAIMABLE" && data["payment_id"] == "after"));
        let next = db.begin_event_stream_gap(now_millis() as i64).unwrap();
        assert!(next.0 > healthy_at, "the heartbeat closed the reopened gap once the events behind it were handled");
    }

    #[tokio::test(start_paused = true)]
    async fn an_overflow_reconciles_in_place_from_the_last_checkpoint_and_keeps_the_stream() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = complete_history();
        let healthy_at = now_millis() as i64 - 1_000;
        let mut health = StreamHealth::new(db.begin_event_stream_gap(healthy_at - 1_000).unwrap());
        health.reconciliation_finished(&complete_counts(), completed_result(), true);
        assert!(health.checkpoint(&db, healthy_at, true, true).unwrap());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        for n in 0..5 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        let (_reader, mut events, active) =
            buffer_events_with_health(TestSource(receiver), BufferLimit { events: 3, bytes: usize::MAX });
        stable_channels::audit::enable_test_capture();
        // One more event once the held ones are handled; the subscription then stays open past a heartbeat and ends.
        let feed = async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            sender.send(Ok(claimable("pay-5"))).unwrap();
            tokio::time::sleep(STREAM_HEALTH_INTERVAL + Duration::from_secs(5)).await;
            drop(sender);
        };
        let (served, ()) = tokio::join!(
            tokio::time::timeout(Duration::from_secs(3_600), serve(&mut events, &active, &manager, &db, &ldk, &mut health)),
            feed,
        );
        served.expect("serve returns once the subscription ends");
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        let handled: Vec<&str> = rows
            .iter()
            .filter(|(name, _)| name == "PAYMENT_CLAIMABLE")
            .filter_map(|(_, data)| data["payment_id"].as_str())
            .collect();
        assert_eq!(handled, ["pay-0", "pay-1", "pay-2", "pay-5"], "{names:?}");
        let resumed = rows.iter().find(|(name, _)| name == "EVENT_BUFFER_RESUMED").expect("reading resumed");
        assert_eq!(resumed.1["dropped"], 2);
        let in_place = rows
            .iter()
            .position(|(name, data)| name == "RECONCILIATION_STARTED" && data["trigger"] == "events_dropped")
            .expect("reconciled in place");
        let result = rows[in_place..].iter().find(|(name, _)| name == "RECONCILIATION_RESULT").expect("the pass recorded its result");
        assert_eq!(result.1["gap_started_ms"], healthy_at, "the gap runs from the last checkpoint, before the dropped events: {names:?}");
        let after = rows.iter().position(|(name, data)| name == "PAYMENT_CLAIMABLE" && data["payment_id"] == "pay-5").unwrap();
        assert!(in_place < after, "{names:?}");
        let next = db.begin_event_stream_gap(now_millis() as i64).unwrap();
        assert!(next.0 > healthy_at, "the heartbeat closed the gap once the hole was reconciled");
    }

    /// A paid request as LDK Server reports it, carrying a wallet's signed message.
    fn paid_request(payment_id: &str, request: &str, amount_msat: u64) -> EventEnvelope {
        use ldk_server_client::ldk_server_grpc::events::PaymentReceived;
        use ldk_server_client::ldk_server_grpc::types::{CustomTlvRecord, Payment};
        EventEnvelope {
            event: Some(EventVariant::PaymentReceived(PaymentReceived {
                payment: Some(Payment { payment_id: payment_id.to_owned(), amount_msat: Some(amount_msat), ..Default::default() }),
                custom_records: vec![CustomTlvRecord {
                    type_num: stable_channels::constants::STABLE_CHANNEL_TLV_TYPE,
                    value: request.as_bytes().to_vec().into(),
                }],
                ..Default::default()
            })),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_event_ldk_server_does_not_answer_for_is_handled_again_backing_off() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let (ldk, request, fee) = crate::stable_manager::tests::trade_request(&mut *manager.lock().await);
        ldk.list_channels_fails.store(true, Ordering::SeqCst);
        let payment_id = "c".repeat(64);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        stable_channels::audit::enable_test_capture();
        let started = tokio::time::Instant::now();
        // LDK Server answers again after 10 s; attempts run at 0, 1, 3, 7 and 15 s.
        let recover = async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            assert!(manager.try_lock().is_ok(), "the lock is free between tries, so the stability tick still runs");
            ldk.list_channels_fails.store(false, Ordering::SeqCst);
        };
        let (kept, ()) = tokio::join!(handle(Ok(paid_request(&payment_id, &request, fee)), &manager, &db, &ldk, &mut health), recover);
        let waited = tokio::time::Instant::now() - started;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert!(kept, "the subscription is kept");
        assert_eq!(waited, Duration::from_secs(15));
        let unanswered = rows.iter().filter(|(name, data)| name == "LDK_CALL_FAILED" && data["op"] == "list_channels").count();
        assert_eq!(unanswered, 4);
        assert!(db.trade_decision_by_payment(&payment_id).unwrap().is_some(), "decided once LDK Server answered");
    }

    #[tokio::test(start_paused = true)]
    async fn an_event_ldk_server_never_answers_for_is_given_up_after_fifteen_minutes() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(MAX_EVENT_RETRY, Duration::from_secs(stable_channels::constants::TRADE_RESULT_TIMEOUT_SECS));
        let (manager, db) = serving_fixture();
        let (ldk, request, fee) = crate::stable_manager::tests::trade_request(&mut *manager.lock().await);
        ldk.list_channels_fails.store(true, Ordering::SeqCst);
        let payment_id = "c".repeat(64);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        stable_channels::audit::enable_test_capture();
        let started = tokio::time::Instant::now();
        let kept = tokio::time::timeout(
            Duration::from_secs(3_600),
            handle(Ok(paid_request(&payment_id, &request, fee)), &manager, &db, &ldk, &mut health),
        )
        .await
        .expect("handling gives up");
        let waited = tokio::time::Instant::now() - started;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert!(kept);
        assert!(waited >= MAX_EVENT_RETRY && waited < MAX_EVENT_RETRY + MAX_RETRY_WAIT, "{waited:?}");
        let abandoned: Vec<_> = rows.iter().filter(|(name, _)| name == "EVENT_RETRY_ABANDONED").collect();
        assert_eq!(abandoned.len(), 1);
        assert_eq!(abandoned[0].1["payment_id"], payment_id.as_str());
        assert!(db.trade_decision_by_payment(&payment_id).unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn unsafe_settlement_outcomes_are_retried_in_place_backing_off_to_a_minute() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap();
        let (manager, db) = serving_fixture();
        db.record_settlement("sync-2", "sync").unwrap();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        ldk.payment_lookup_failures.store(usize::MAX, Ordering::SeqCst);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(Ok(claimable("buffered"))).unwrap();
        drop(sender);
        let (_reader, mut events, active) = buffer_events_with_health(TestSource(receiver), BUFFER_LIMIT);
        stable_channels::audit::enable_test_capture();
        let started = tokio::time::Instant::now();
        // Lookups work again after 150 s; passes run at 0, 1, 3, 7, 15, 31, 63, 123 and 183 s.
        let recover = async {
            tokio::time::sleep(Duration::from_secs(150)).await;
            ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        };
        let (served, ()) = tokio::join!(
            tokio::time::timeout(Duration::from_secs(3_600), serve(&mut events, &active, &manager, &db, &ldk, &mut health)),
            recover,
        );
        served.expect("serve returns once the subscription ends");
        let waited = tokio::time::Instant::now() - started;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(waited, Duration::from_secs(1 + 2 + 4 + 8 + 16 + 32 + 60 + 60), "{names:?}");
        assert_eq!(names.iter().filter(|name| **name == "RECONCILIATION_RESULT").count(), 9);
        let named = rows
            .iter()
            .filter(|(name, data)| name == "RECONCILIATION_SCOPE_FAILED" && data["error"].as_str().is_some_and(|error| error.contains("sync-2")))
            .count();
        assert_eq!(named, 8, "each failed pass names the payment it could not settle");
        let last_pass = rows.iter().rposition(|(name, _)| name == "RECONCILIATION_RESULT").unwrap();
        let handled = rows
            .iter()
            .position(|(name, data)| name == "PAYMENT_CLAIMABLE" && data["payment_id"] == "buffered")
            .expect("the event buffered meanwhile is still handled");
        assert!(last_pass < handled);
    }

    #[tokio::test]
    async fn channel_openings_and_failed_opens_share_the_peers_daily_limit() {
        use crate::record_limits::NEVER_USABLE_RECORDS_PER_PEER;
        use crate::stable_manager::tests::{make_manager, FakeLdkServer, AUDIT_TEST_GUARD, COUNTERPARTY_HEX};
        use ldk_server_client::ldk_server_grpc::events::ChannelStateChanged;
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let mut mgr = make_manager();
        let dir = tempfile::tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let ldk = FakeLdkServer::new(vec![]);
        stable_channels::audit::enable_test_capture();
        for n in 0..NEVER_USABLE_RECORDS_PER_PEER + 1 {
            let state = if n % 2 == 0 { ChannelState::Pending } else { ChannelState::OpenFailed };
            let event = ChannelStateChanged {
                channel_id: format!("{n:064x}"),
                user_channel_id: (u128::from(n) * 1_000_003 + 274_877_906_944).to_string(),
                counterparty_node_id: Some(COUNTERPARTY_HEX.to_string()),
                state: state as i32,
                funding_txo: Some(format!("{}:0", "ab".repeat(32))),
                ..Default::default()
            };
            dispatch_event(Some(EventVariant::ChannelStateChanged(event)), &mut mgr, &db, &ldk, 100_000.0).await;
        }
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let recorded = events.iter().filter(|(name, _)| name == "CHANNEL_PENDING" || name == "CHANNEL_OPEN_FAILED").count();
        assert_eq!(recorded, NEVER_USABLE_RECORDS_PER_PEER as usize);
    }

    #[tokio::test]
    async fn aborted_reader_is_not_reported_healthy() {
        let (_sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (mut reader, _events, active) = buffer_events_with_health(TestSource(receiver), BUFFER_LIMIT);
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
        let (mut reader, mut events, active) = buffer_events_with_health(TestSource(receiver), BUFFER_LIMIT);
        sender.send(Ok(EventEnvelope::default())).unwrap();
        sender.send(Err(LdkServerError::new(
            LdkServerErrorCode::InternalServerError, "stream failed during backfill",
        ))).unwrap();
        reader.join_next().await.unwrap().unwrap();
        assert!(!active.load(Ordering::Acquire));
        assert!(events.recv().await.unwrap().event().is_ok());
        assert!(!active.load(Ordering::Acquire), "an earlier buffered success cannot restore health");
        assert!(events.recv().await.unwrap().event().is_err());
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
        let d = connected_audit_data(Some("event-stream-gap-1"), Some("0.1.0 (d7e7a2e708664af922c53c786ad6a5df622f5dd8)"));
        assert_eq!(d["correlation_id"], "event-stream-gap-1");
        assert_eq!(d["ldk_server_version"], "0.1.0 (d7e7a2e708664af922c53c786ad6a5df622f5dd8)");
        assert_eq!(d["expected_ldk_server_rev"], PINNED_LDK_SERVER_REV);
        assert_eq!(d["ldk_server_matches_pin"], true);
        assert_eq!(connected_audit_data(None, Some("0.1.0 (aa2d60f51ebacc7edd5dbdfb90b53a9c73ee0bd3)"))["ldk_server_matches_pin"], false);
        let unreported = connected_audit_data(None, Some(""));
        assert!(unreported["ldk_server_version"].is_null());
        assert!(unreported["ldk_server_matches_pin"].is_null());
    }

    #[test]
    fn every_ldk_server_pin_names_the_same_rev() {
        // The hex rev that follows `marker` on the first line starting with `line_start`, or "" if there is none.
        let rev_in = |text: &str, line_start: &str, marker: &str| -> String {
            text.lines()
                .map(str::trim_start)
                .find(|line| line.starts_with(line_start))
                .and_then(|line| line.split(marker).nth(1))
                .map(|rest| rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect())
                .unwrap_or_default()
        };
        let client = rev_in(include_str!("../Cargo.toml"), "ldk-server-client", "rev = \"");
        assert!(client.starts_with(PINNED_LDK_SERVER_REV), "update PINNED_LDK_SERVER_REV with the ldk-server-client pin");
        let grpc = rev_in(include_str!("../../sc-rest-client/Cargo.toml"), "ldk-server-grpc", "rev = \"");
        assert_eq!(grpc, client, "sc-rest-client's ldk-server-grpc must be pinned to the ldk-server-client rev");
        let image = rev_in(include_str!("../../../umbrel/docker/Dockerfile.ldk-server"), "&& git checkout", "git checkout ");
        assert_eq!(image, client, "Dockerfile.ldk-server must build the ldk-server-client rev");
    }

    #[test]
    fn claimable_reflects_custom_records_and_omits_absent_id() {
        let d = claimable_audit_data(None, None, true);
        assert_eq!(d["has_custom_records"], true);
        assert!(d.get("payment_id").is_none());
    }
}
