//! Long-running SubscribeEvents loop: connects to LDK Server's event stream, reconnects with exponential backoff, and dispatches each EventEnvelope to its handler.

use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;

use tracing::{info, warn};

use ldk_server_client::ldk_server_grpc::events::event_envelope::Event as EventVariant;
use ldk_server_client::ldk_server_grpc::events::{ChannelState, EventEnvelope};

use crate::event_inbox::{payload_size, Entry, Inbox, Row};
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

/// The reader with the production limits over an in-memory inbox, for tests; dropping the JoinSet stops it.
#[cfg(test)]
pub(crate) fn buffer_events(source: impl EventSource) -> (tokio::task::JoinSet<()>, EventBuffer) {
    let (reader, events, _) = buffer_events_with_health(source, Arc::new(Inbox::in_memory()), LIMITS);
    (reader, events)
}

/// A reader counting what its inbox already holds, for tests.
#[cfg(test)]
fn buffer_events_with_health(
    source: impl EventSource,
    inbox: Arc<Inbox>,
    limits: Limits,
) -> (tokio::task::JoinSet<()>, EventBuffer, Arc<AtomicBool>) {
    let saved = count_saved(&inbox);
    start_reader(source, inbox, saved, limits)
}

/// The events and payload bytes already saved; a failed read counts nothing, so the inbox limit counts only new ones.
fn count_saved(inbox: &Inbox) -> (usize, usize) {
    inbox.held().unwrap_or_else(|error| {
        warn!("[event_loop] couldn't count the saved events, so the inbox limit counts only new ones: {}", error);
        (0, 0)
    })
}

/// Most events, and encoded bytes, kept for handling, saved or not; the reader can't wait instead, since LDK Server drops events for a reader that falls behind.
const INBOX_LIMIT: BufferLimit = BufferLimit { events: 1_000_000, bytes: 1024 * 1024 * 1024 };
/// Most events, and encoded bytes, waiting in memory to be saved, so a failing disk can't fill memory instead.
const HANDOFF_LIMIT: BufferLimit = BufferLimit { events: 100_000, bytes: 64 * 1024 * 1024 };
/// Most entries saved in one transaction.
const SAVE_BATCH: usize = 1_000;

#[derive(Debug, Clone, Copy)]
struct Limits {
    inbox: BufferLimit,
    handoff: BufferLimit,
}

const LIMITS: Limits = Limits { inbox: INBOX_LIMIT, handoff: HANDOFF_LIMIT };

impl Limits {
    fn half(self) -> Self {
        Self { inbox: self.inbox.half(), handoff: self.handoff.half() }
    }
}

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

/// Events and their encoded bytes held against a limit.
#[derive(Default)]
struct Count {
    events: AtomicUsize,
    bytes: AtomicUsize,
}

impl Count {
    fn add(&self, events: usize, bytes: usize) {
        self.events.fetch_add(events, Ordering::SeqCst);
        self.bytes.fetch_add(bytes, Ordering::SeqCst);
    }

    fn sub(&self, events: usize, bytes: usize) {
        let _ = self.events.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |held| Some(held.saturating_sub(events)));
        let _ = self.bytes.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |held| Some(held.saturating_sub(bytes)));
    }

    /// An empty count takes any one event, so an oversized event can't stop the stream for good.
    fn has_room(&self, bytes: usize, limit: BufferLimit) -> bool {
        let events = self.events.load(Ordering::SeqCst);
        events == 0 || (events < limit.events && self.bytes.load(Ordering::SeqCst).saturating_add(bytes) <= limit.bytes)
    }
}

/// What the reader and dispatch share.
#[derive(Default)]
struct Shared {
    /// Events saved or waiting to be saved, against the inbox limit.
    held: Count,
    /// Events waiting in memory to be saved, against the hand-off limit.
    handoff: Count,
    /// Entries read and not saved yet, plus a hole being dropped; the checkpoint can't pass them.
    unsaved: AtomicUsize,
    /// Wakes a reader holding a hole when dispatch frees room, so the hole is saved even if no event follows it.
    freed: tokio::sync::Notify,
    /// Wakes dispatch when rows are saved or the reader ends.
    saved: tokio::sync::Notify,
    /// The reader has ended, after saving everything it read.
    finished: AtomicBool,
    /// The subscription delivered at least one event.
    read_an_event: AtomicBool,
}

impl Shared {
    fn has_room(&self, bytes: usize, limits: Limits) -> bool {
        self.held.has_room(bytes, limits.inbox) && self.handoff.has_room(bytes, limits.handoff)
    }
}

/// Saved events waiting to be handled, read oldest first.
pub(crate) struct EventBuffer {
    inbox: Arc<Inbox>,
    shared: Arc<Shared>,
}

impl EventBuffer {
    /// The oldest saved row, left in place until `finish`; None once the reader has ended, even with rows left, so a reconnect doesn't wait for them.
    pub(crate) async fn recv(&mut self) -> Option<Row> {
        let mut backoff = Duration::from_secs(1);
        loop {
            if self.shared.finished.load(Ordering::SeqCst) {
                return None;
            }
            match self.inbox.oldest() {
                Ok(Some(row)) => return Some(row),
                Ok(None) => self.shared.saved.notified().await,
                Err(error) => {
                    warn!("[event_loop] couldn't read the event inbox: {}; trying again in {:?}", error, backoff);
                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(backoff * 2, MAX_RETRY_WAIT);
                },
            }
        }
    }

    /// Delete a handled row, trying until it is gone, since a row left behind would be handled again.
    pub(crate) async fn finish(&self, id: i64) {
        let mut backoff = Duration::from_secs(1);
        loop {
            match self.inbox.finish(id) {
                Ok((events, bytes)) => {
                    self.shared.held.sub(events, bytes);
                    self.shared.freed.notify_one();
                    return;
                },
                Err(error) => inbox_write_failed("finish_event", Some(id), &error),
            }
            tokio::time::sleep(backoff).await;
            backoff = std::cmp::min(backoff * 2, MAX_RETRY_WAIT);
        }
    }

    /// No row saved or waiting to be saved, and no hole being dropped; a failed read counts as not empty.
    pub(crate) fn is_empty(&self) -> bool {
        // Rows are counted as saved only once committed, so reading the count first can't miss one.
        self.shared.unsaved.load(Ordering::SeqCst) == 0 && self.inbox.is_empty().unwrap_or(false)
    }

    pub(crate) fn inbox(&self) -> &Inbox {
        &self.inbox
    }

    fn read_an_event(&self) -> bool {
        self.shared.read_an_event.load(Ordering::SeqCst)
    }

    /// Events held, saved or waiting to be saved.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.shared.held.events.load(Ordering::SeqCst)
    }

    /// A second handle on the same rows, for a test to watch while serve holds this one.
    #[cfg(test)]
    pub(crate) fn watcher(&self) -> EventBuffer {
        EventBuffer { inbox: Arc::clone(&self.inbox), shared: Arc::clone(&self.shared) }
    }

    #[cfg(test)]
    pub(crate) async fn take(&mut self) -> Option<Entry> {
        let row = self.recv().await?;
        self.finish(row.id).await;
        Some(row.entry)
    }
}

fn inbox_write_failed(op: &str, row_id: Option<i64>, error: &rusqlite::Error) {
    warn!("[event_loop] {} failed: {}", op, error);
    stable_channels::audit::audit_event(
        "DB_WRITE_FAILED",
        serde_json::json!({ "op": op, "row_id": row_id, "error": error.to_string(), "scope": "event_inbox" }),
    );
}

/// Drop events rather than hold more; the hole's reconcile settles payment outcomes and channels, but trades and forward deductions among them are not replayed.
fn buffer_full(shared: &Shared, limits: Limits) {
    warn!("[event_loop] event inbox full; dropping events until fewer than half are held");
    stable_channels::audit::audit_event(
        "EVENT_BUFFER_FULL",
        serde_json::json!({
            "events": shared.held.events.load(Ordering::SeqCst),
            "bytes": shared.held.bytes.load(Ordering::SeqCst),
            "max_events": limits.inbox.events,
            "max_bytes": limits.inbox.bytes,
            "unsaved_events": shared.handoff.events.load(Ordering::SeqCst),
            "unsaved_bytes": shared.handoff.bytes.load(Ordering::SeqCst),
            "max_unsaved_events": limits.handoff.events,
            "max_unsaved_bytes": limits.handoff.bytes,
        }),
    );
}

type SaveResult = Result<(Vec<(Entry, i64)>, rusqlite::Result<()>), tokio::task::JoinError>;

/// Saves what the reader hands it in batches, off the runtime, keeping a batch that fails.
struct Saver {
    inbox: Arc<Inbox>,
    shared: Arc<Shared>,
    waiting: std::collections::VecDeque<(Entry, i64)>,
    saving: Option<tokio::task::JoinHandle<(Vec<(Entry, i64)>, rusqlite::Result<()>)>>,
    retry_at: Option<tokio::time::Instant>,
    backoff: Duration,
}

impl Saver {
    /// Queue an entry whose place in `unsaved` is already taken.
    fn queue(&mut self, entry: Entry) {
        if matches!(entry, Entry::Event(_)) {
            let bytes = payload_size(&entry);
            self.shared.held.add(1, bytes);
            self.shared.handoff.add(1, bytes);
        }
        self.waiting.push_back((entry, now_millis() as i64));
    }

    fn pending(&self) -> bool {
        self.saving.is_some() || !self.waiting.is_empty()
    }

    /// Checked on every turn, since the retry timer's branch isn't reached while reads keep coming.
    fn retry_if_due(&mut self) {
        if self.retry_at.is_some_and(|at| at <= tokio::time::Instant::now()) {
            self.retry_at = None;
        }
    }

    fn start(&mut self) {
        if self.saving.is_some() || self.retry_at.is_some() || self.waiting.is_empty() {
            return;
        }
        let batch: Vec<_> = self.waiting.drain(..self.waiting.len().min(SAVE_BATCH)).collect();
        let (inbox, shared) = (Arc::clone(&self.inbox), Arc::clone(&self.shared));
        self.saving = Some(tokio::task::spawn_blocking(move || {
            let result = inbox.save_and_then(&batch, || {
                let events: Vec<usize> = batch.iter().filter(|(entry, _)| matches!(entry, Entry::Event(_))).map(|(entry, _)| payload_size(entry)).collect();
                shared.handoff.sub(events.len(), events.iter().sum());
                shared.unsaved.fetch_sub(batch.len(), Ordering::SeqCst);
            });
            (batch, result)
        }));
    }

    fn done(&mut self, joined: SaveResult) {
        self.saving = None;
        match joined {
            Ok((_, Ok(()))) => {
                self.backoff = Duration::from_secs(1);
                self.shared.saved.notify_one();
            },
            Ok((batch, Err(error))) => {
                inbox_write_failed("save_events", None, &error);
                for entry in batch.into_iter().rev() {
                    self.waiting.push_front(entry);
                }
                self.retry_at = Some(tokio::time::Instant::now() + self.backoff);
                self.backoff = std::cmp::min(self.backoff * 2, MAX_RETRY_WAIT);
            },
            // Only a panic gets here; its entries stay counted as unsaved, so no checkpoint passes them.
            Err(error) => warn!("[event_loop] saving events stopped: {}", error),
        }
    }
}

async fn save_finished(saving: &mut Option<tokio::task::JoinHandle<(Vec<(Entry, i64)>, rusqlite::Result<()>)>>) -> SaveResult {
    match saving {
        Some(handle) => handle.await,
        None => std::future::pending().await,
    }
}

async fn retry_due(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Save a hole's marker in the place it has held since its first drop.
fn queue_hole(saver: &mut Saver, dropped: &mut u64) {
    stable_channels::audit::audit_event("EVENT_BUFFER_RESUMED", serde_json::json!({ "dropped": *dropped }));
    saver.queue(Entry::Dropped(*dropped));
    *dropped = 0;
}

struct ReaderActivity(Arc<AtomicBool>);

impl Drop for ReaderActivity {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Marks the reader ended however its task stops, so dispatch stops waiting for it.
struct ReaderFinished(Arc<Shared>);

impl Drop for ReaderFinished {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::SeqCst);
        self.0.saved.notify_one();
    }
}

/// Start reading a subscription into the inbox; `saved` is what the inbox already holds, counted before subscribing so reading starts at once.
fn start_reader(
    mut source: impl EventSource,
    inbox: Arc<Inbox>,
    saved: (usize, usize),
    limits: Limits,
) -> (tokio::task::JoinSet<()>, EventBuffer, Arc<AtomicBool>) {
    let shared = Arc::new(Shared::default());
    shared.held.add(saved.0, saved.1);
    let active = Arc::new(AtomicBool::new(false));
    let activity = ReaderActivity(Arc::clone(&active));
    let finished = ReaderFinished(Arc::clone(&shared));
    let mut saver = Saver {
        inbox: Arc::clone(&inbox),
        shared: Arc::clone(&shared),
        waiting: Default::default(),
        saving: None,
        retry_at: None,
        backoff: Duration::from_secs(1),
    };
    // The subscription's start goes ahead of its first event, so its reconcile runs there.
    shared.unsaved.fetch_add(1, Ordering::SeqCst);
    saver.queue(Entry::Connected);
    let mut reader = tokio::task::JoinSet::new();
    reader.spawn(async move {
        let _finished = finished;
        let activity = activity;
        activity.0.store(true, Ordering::Release);
        let shared = Arc::clone(&saver.shared);
        let mut dropped: u64 = 0;
        loop {
            // The same read stays pending across wake-ups, so nothing is cancelled.
            let mut next = source.next_event();
            let item = loop {
                saver.retry_if_due();
                saver.start();
                tokio::select! {
                    biased;
                    joined = save_finished(&mut saver.saving) => {
                        saver.done(joined);
                        if dropped > 0 && shared.has_room(0, limits.half()) {
                            queue_hole(&mut saver, &mut dropped);
                        }
                    },
                    item = &mut next => break item,
                    () = retry_due(saver.retry_at) => saver.retry_at = None,
                    _ = shared.freed.notified(), if dropped > 0 => {
                        if shared.has_room(0, limits.half()) {
                            queue_hole(&mut saver, &mut dropped);
                        }
                    },
                }
            };
            let envelope = match item {
                Some(Ok(envelope)) => {
                    shared.read_an_event.store(true, Ordering::SeqCst);
                    envelope
                },
                Some(Err(error)) => {
                    warn!("[event_loop] event stream failed: {}", error);
                    break;
                },
                None => break,
            };
            // Take the event's place before deciding, so no checkpoint passes it while it is decided or dropped.
            shared.unsaved.fetch_add(1, Ordering::SeqCst);
            let bytes = prost::Message::encoded_len(&envelope);
            // Once full, take events again only when fewer than half are held, so each hole costs one reconcile per half an inbox handled.
            let room = if dropped == 0 { limits } else { limits.half() };
            if shared.has_room(bytes, room) {
                if dropped > 0 {
                    queue_hole(&mut saver, &mut dropped);
                }
                saver.queue(Entry::Event(envelope));
                continue;
            }
            if dropped == 0 {
                // The first dropped event's place is kept for the hole's marker.
                buffer_full(&shared, limits);
            } else {
                shared.unsaved.fetch_sub(1, Ordering::SeqCst);
            }
            dropped += 1;
        }
        // A hole still being dropped keeps its place and is left to the reconnect's gap reconcile.
        activity.0.store(false, Ordering::Release);
        while saver.pending() {
            saver.retry_if_due();
            saver.start();
            tokio::select! {
                joined = save_finished(&mut saver.saving) => saver.done(joined),
                () = retry_due(saver.retry_at) => saver.retry_at = None,
            }
        }
    });
    (reader, EventBuffer { inbox, shared }, active)
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
    /// The inbox couldn't record that the handler is starting: run nothing and try again after a pause.
    NotMarked,
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
const PINNED_LDK_SERVER_REV: &str = "aa2d60f5";

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

/// Open the inbox and report the rows a stop interrupted, before anything is handled.
async fn start_inbox(dir: &std::path::Path) -> Arc<Inbox> {
    let inbox = open_inbox_with_retry(dir).await;
    report_interrupted(&inbox).await;
    Arc::new(inbox)
}

async fn open_inbox_with_retry(dir: &std::path::Path) -> Inbox {
    let mut backoff = Duration::from_secs(1);
    loop {
        match Inbox::open(dir) {
            Ok(inbox) => return inbox,
            Err(error) => warn!("[event_loop] couldn't open the event inbox: {}; trying again in {:?}", error, backoff),
        }
        tokio::time::sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, MAX_RETRY_WAIT);
    }
}

/// A row still marked started was being handled when the daemon stopped; its effects may be partly applied, so it is recorded and dropped, not replayed.
async fn report_interrupted(inbox: &Inbox) {
    let mut backoff = Duration::from_secs(1);
    let rows = loop {
        match inbox.take_interrupted() {
            Ok(rows) => break rows,
            Err(error) => inbox_write_failed("take_interrupted_events", None, &error),
        }
        tokio::time::sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, MAX_RETRY_WAIT);
    };
    for row in &rows {
        record_interrupted(row);
    }
}

/// Record a row whose handler started and may have partly applied, which is not handled again.
fn record_interrupted(row: &Row) {
    let (event, payment_id) = match &row.entry {
        Entry::Event(envelope) => (event_name(envelope), payment_of(envelope)),
        Entry::Connected => ("connected", None),
        Entry::Dropped(_) => ("dropped", None),
        Entry::Unreadable => ("unreadable", None),
    };
    warn!("[event_loop] saved event {} ({}) was being handled when it stopped; not handling it again", row.id, event);
    stable_channels::audit::audit_event(
        "EVENT_INTERRUPTED",
        serde_json::json!({ "row_id": row.id, "event": event, "payment_id": payment_id, "received_ms": row.received_ms }),
    );
}

/// Longest the first rows wait at startup for a price, so a trade saved before a restart isn't rejected for want of one.
const STARTUP_PRICE_WAIT: Duration = Duration::from_secs(60);

/// Wait until `price` gives a price, up to `limit`; false when none came.
async fn wait_for_price(price: impl Fn() -> f64, limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if price() > 0.0 {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            warn!("[event_loop] no BTC price after {:?}; handling saved events without one", limit);
            return false;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// A subscription that ended at once without an event, as while LDK Server shuts down, waits like a failed subscribe before the next.
fn resubscribe_delay(read_an_event: bool, lived: Duration, backoff: &mut Duration) -> Option<Duration> {
    if read_an_event || lived >= STREAM_HEALTH_INTERVAL {
        *backoff = Duration::from_secs(1);
        return None;
    }
    let delay = *backoff;
    *backoff = std::cmp::min(*backoff * 2, MAX_RETRY_WAIT);
    Some(delay)
}

async fn run(state: AppState) {
    let mut backoff = Duration::from_secs(1);
    let inbox = start_inbox(&state.data_dir).await;
    // Reload the durable boundary, including unresolved gaps surviving earlier processes. The
    // compatibility AppState timestamp is never used as a substitute for a failed database read.
    let mut health = StreamHealth::new(begin_gap_with_retry(state.db.as_ref()).await);
    let mut first = true;
    loop {
        // Nothing writes the inbox between subscriptions, so counting it first lets the reader start the moment the stream opens.
        let saved = count_saved(&inbox);
        let stream = match state.ldk_server.subscribe_events().await {
            // The backoff starts over only once a subscription delivers or lasts.
            Ok(s) => s,
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
        let subscribed_at = tokio::time::Instant::now();
        let (_reader, mut events, active) = start_reader(stream, Arc::clone(&inbox), saved, LIMITS);
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
        if std::mem::take(&mut first) {
            // The price task starts with the daemon; the reader keeps saving meanwhile.
            wait_for_price(stable_channels::price_feeds::get_fresh_cached_price_no_fetch, STARTUP_PRICE_WAIT).await;
        }
        serve(&mut events, &active, &state.stable_manager, state.db.as_ref(), state.ldk_server.as_ref(), &mut health).await;
        warn!("[event_loop] stream ended; reconnecting");
        // Persist before announcing the transition or attempting another subscription. If this
        // write fails, the prior durable health still bounds a crash/restart conservatively.
        health = StreamHealth::new(begin_gap_with_retry(state.db.as_ref()).await);
        stable_channels::audit::audit_event(
            "EVENT_STREAM_DISCONNECTED",
            serde_json::json!({ "correlation_id": health.gap.as_ref().map(|gap| gap.1.as_str()) }),
        );
        if let Some(delay) = resubscribe_delay(events.read_an_event(), subscribed_at.elapsed(), &mut backoff) {
            warn!("[event_loop] the subscription ended at once without an event; subscribing again in {:?}", delay);
            tokio::time::sleep(delay).await;
        }
    }
}

/// Longest wait between reconciliation passes while terminal settlement outcomes can't be applied, and between tries of an event LDK Server didn't answer for.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

/// Handle saved rows oldest first, each deleted once handled, until the reader ends; rows left then wait for the next subscription.
async fn serve(
    events: &mut EventBuffer,
    active: &AtomicBool,
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    health: &mut StreamHealth,
) {
    let mut heartbeat = health_interval(STREAM_HEALTH_INTERVAL);
    loop {
        tokio::select! {
            row = events.recv() => {
                let Some(row) = row else { return };
                // Within one run only a delete that didn't take effect leaves a row marked, and its handler already ran.
                if row.started {
                    record_interrupted(&row);
                    events.finish(row.id).await;
                    continue;
                }
                let connected = row.entry == Entry::Connected;
                match row.entry {
                    Entry::Event(envelope) => handle(row.id, envelope, events.inbox(), manager, db, ldk, health).await,
                    // Rows saved before this subscription started are handled first, then its gap is reconciled; a start right behind it reconciles the same open gap later.
                    Entry::Connected if events.inbox().next_is_connected(row.id).unwrap_or(false) => {},
                    Entry::Connected => reconcile_until_safe(manager, db, ldk, health, "stream_connected").await,
                    Entry::Dropped(_) => reconcile_in_place(manager, db, ldk, health, "events_dropped").await,
                    Entry::Unreadable => {
                        warn!("[event_loop] saved event {} can't be read; skipping it", row.id);
                        stable_channels::audit::audit_event(
                            "EVENT_UNREADABLE",
                            serde_json::json!({ "row_id": row.id, "received_ms": row.received_ms }),
                        );
                    },
                }
                events.finish(row.id).await;
                if connected {
                    checkpoint_if_ready(health, db, active, events);
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

/// Handle one saved event, trying again while LDK Server doesn't answer for it; its row is marked started only while its handler can change something.
async fn handle(
    id: i64,
    envelope: EventEnvelope,
    inbox: &Inbox,
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    health: &mut StreamHealth,
) {
    let started = tokio::time::Instant::now();
    let mut retry = Duration::from_secs(1);
    let mut unmarked = Duration::from_secs(1);
    loop {
        match dispatch(envelope.clone(), manager, db, ldk, || inbox.set_started(id, true)).await {
            DispatchOutcome::Continue => return,
            DispatchOutcome::NotMarked => {
                warn!("[event_loop] couldn't mark saved event {} started; trying again in {:?}", id, unmarked);
                tokio::time::sleep(unmarked).await;
                unmarked = std::cmp::min(unmarked * 2, MAX_RETRY_WAIT);
            },
            DispatchOutcome::Reconcile => {
                reconcile_in_place(manager, db, ldk, health, "dispatch_write_failed").await;
                return;
            },
            DispatchOutcome::Retry if started.elapsed() < MAX_EVENT_RETRY => {
                // Nothing of it was applied, so a crash while it waits replays it.
                if let Err(error) = inbox.set_started(id, false) {
                    inbox_write_failed("clear_event_started", Some(id), &error);
                }
                warn!("[event_loop] LDK Server didn't answer for an event; trying it again in {:?}", retry);
                tokio::time::sleep(retry).await;
                retry = std::cmp::min(retry * 2, MAX_RETRY_WAIT);
            },
            DispatchOutcome::Retry => {
                warn!("[event_loop] LDK Server didn't answer for an event within {:?}; moving on", MAX_EVENT_RETRY);
                stable_channels::audit::audit_event(
                    "EVENT_RETRY_ABANDONED",
                    serde_json::json!({ "payment_id": payment_of(&envelope), "waited_secs": started.elapsed().as_secs() }),
                );
                return;
            },
        }
    }
}

/// The kind of event, for records about a saved row.
fn event_name(envelope: &EventEnvelope) -> &'static str {
    match envelope.event {
        Some(EventVariant::PaymentReceived(_)) => "payment_received",
        Some(EventVariant::PaymentForwarded(_)) => "payment_forwarded",
        Some(EventVariant::PaymentSuccessful(_)) => "payment_successful",
        Some(EventVariant::PaymentFailed(_)) => "payment_failed",
        Some(EventVariant::PaymentClaimable(_)) => "payment_claimable",
        Some(EventVariant::ChannelStateChanged(_)) => "channel_state_changed",
        Some(EventVariant::SpliceNegotiated(_)) => "splice_negotiated",
        Some(EventVariant::SpliceNegotiationFailed(_)) => "splice_negotiation_failed",
        None => "none",
    }
}

/// The payment an event reports; forwards and channel events have none.
fn payment_of(envelope: &EventEnvelope) -> Option<&str> {
    let payment = match envelope.event.as_ref()? {
        EventVariant::PaymentReceived(event) => event.payment.as_ref(),
        EventVariant::PaymentSuccessful(event) => event.payment.as_ref(),
        EventVariant::PaymentFailed(event) => event.payment.as_ref(),
        EventVariant::PaymentClaimable(event) => event.payment.as_ref(),
        _ => None,
    };
    payment.map(|payment| payment.payment_id.as_str())
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

/// Run one event's handler under the manager lock, recording that it started only once the lock is held.
async fn dispatch(
    envelope: EventEnvelope,
    manager: &tokio::sync::Mutex<StableChannelManager>,
    db: &stable_channels::db::Database,
    ldk: &dyn LdkServerCalls,
    mark_started: impl FnOnce() -> rusqlite::Result<()>,
) -> DispatchOutcome {
    // Waiting here applies nothing, so the row isn't marked yet and a crash meanwhile replays it.
    let mut mgr = StableChannelManager::lock_for_event_with_top_ups_resolved(manager, ldk).await;
    // A handler run on an unmarked row would be replayed after a crash, and forwards deduct each time they're handled.
    if let Err(error) = mark_started() {
        inbox_write_failed("mark_event_started", None, &error);
        drop(mgr);
        return DispatchOutcome::NotMarked;
    }
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
        let (mut reader, mut events, active) = buffer_events_with_health(TestSource(receiver), Arc::new(Inbox::in_memory()), LIMITS);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !active.load(Ordering::Acquire) { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(!events.is_empty(), "the subscription's connected row comes first");
        assert_eq!(events.take().await, Some(Entry::Connected));
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

    fn payment_id_of(entry: Option<Entry>) -> String {
        match entry {
            Some(Entry::Event(EventEnvelope { event: Some(EventVariant::PaymentClaimable(event)) })) => event.payment.unwrap().payment_id,
            other => panic!("unexpected entry {other:?}"),
        }
    }

    /// The inbox and hand-off limits a test sets, with the production hand-off unless it says otherwise.
    fn inbox_limit(events: usize, bytes: usize) -> Limits {
        Limits { inbox: BufferLimit { events, bytes }, handoff: HANDOFF_LIMIT }
    }

    /// Every save of an event fails, as on a full disk.
    const FAIL_EVENT_SAVES: &str = "CREATE TRIGGER fail_save BEFORE INSERT ON events WHEN NEW.kind = 'event'
         BEGIN SELECT RAISE(FAIL, 'disk full'); END;";

    /// An inbox on disk, and a second connection to it for injecting failures.
    fn inbox_on_disk() -> (tempfile::TempDir, Arc<Inbox>, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Arc::new(Inbox::open(dir.path()).unwrap());
        let conn = rusqlite::Connection::open(dir.path().join(crate::event_inbox::INBOX_FILENAME)).unwrap();
        (dir, inbox, conn)
    }

    async fn saved(inbox: &Inbox, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while inbox.held().unwrap().0 != count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the reader saves what it was sent");
    }

    /// A subscription with LDK Server's bounded per-subscriber queue.
    struct BoundedSource(tokio::sync::mpsc::Receiver<EventItem>);

    #[async_trait::async_trait]
    impl EventSource for BoundedSource {
        async fn next_event(&mut self) -> Option<EventItem> {
            self.0.recv().await
        }
    }

    #[tokio::test]
    async fn a_subscription_is_saved_in_order_after_its_connected_row() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (_reader, mut events) = buffer_events(TestSource(receiver));
        assert!(!events.is_empty(), "the connected row counts from the start, before the reader first runs");
        for n in 0..3 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        assert_eq!(events.take().await, Some(Entry::Connected));
        for n in 0..3 {
            assert_eq!(payment_id_of(events.take().await), format!("pay-{n}"));
        }
        assert_eq!(events.inbox().held().unwrap(), (0, 0));
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn saved_events_outlive_the_reader() {
        let inbox = Arc::new(Inbox::in_memory());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (reader, events, _active) = buffer_events_with_health(TestSource(receiver), Arc::clone(&inbox), LIMITS);
        sender.send(Ok(claimable("pay-0"))).unwrap();
        sender.send(Ok(claimable("pay-1"))).unwrap();
        saved(&inbox, 2).await;
        drop((reader, events, sender));
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (_reader, mut events, _active) = buffer_events_with_health(TestSource(receiver), Arc::clone(&inbox), LIMITS);
        assert_eq!(events.len(), 2, "counted from what was saved");
        assert_eq!(events.take().await, Some(Entry::Connected));
        assert_eq!(payment_id_of(events.take().await), "pay-0");
        assert_eq!(payment_id_of(events.take().await), "pay-1");
        assert_eq!(events.take().await, Some(Entry::Connected), "the new subscription's row goes behind them");
        drop(sender);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_save_keeps_the_batch_and_retries() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_dir, inbox, conn) = inbox_on_disk();
        conn.execute_batch(FAIL_EVENT_SAVES).unwrap();
        stable_channels::audit::enable_test_capture();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (_reader, mut events, _active) = buffer_events_with_health(TestSource(receiver), Arc::clone(&inbox), LIMITS);
        sender.send(Ok(claimable("pay-0"))).unwrap();
        sender.send(Ok(claimable("pay-1"))).unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let rows = stable_channels::audit::drain_test_capture();
        assert!(rows.iter().any(|(name, data)| name == "DB_WRITE_FAILED" && data["op"] == "save_events"), "{rows:?}");
        assert_eq!(inbox.held().unwrap(), (0, 0), "nothing was saved");
        assert_eq!(events.len(), 2, "both are kept");
        conn.execute_batch("DROP TRIGGER fail_save").unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
        stable_channels::audit::disable_test_capture();
        let mut taken = vec![];
        for _ in 0..3 {
            taken.push(tokio::time::timeout(Duration::from_secs(120), events.take()).await.expect("saved once writes work"));
        }
        assert_eq!(taken[0], Some(Entry::Connected));
        assert_eq!(payment_id_of(taken[1].clone()), "pay-0");
        assert_eq!(payment_id_of(taken[2].clone()), "pay-1");
        assert_eq!(inbox.oldest().unwrap(), None, "each was saved once");
        drop(sender);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_saves_drop_past_the_handoff_limit_and_save_the_hole_later() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_dir, inbox, conn) = inbox_on_disk();
        conn.execute_batch(FAIL_EVENT_SAVES).unwrap();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let limits = Limits { inbox: INBOX_LIMIT, handoff: BufferLimit { events: 2, bytes: usize::MAX } };
        let (_reader, mut events, _active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), limits);
        for n in 0..5 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        dealt_with(&asks, 5).await;
        assert_eq!(events.len(), 2, "two wait in memory and the rest are dropped");
        conn.execute_batch("DROP TRIGGER fail_save").unwrap();
        assert_eq!(events.take().await, Some(Entry::Connected));
        assert_eq!(payment_id_of(events.take().await), "pay-0");
        assert_eq!(payment_id_of(events.take().await), "pay-1");
        assert_eq!(events.take().await, Some(Entry::Dropped(3)), "the hole is saved once the hand-off drains");
        sender.send(Ok(claimable("pay-5"))).unwrap();
        assert_eq!(payment_id_of(events.take().await), "pay-5");
    }

    #[tokio::test]
    async fn the_reader_keeps_reading_while_a_commit_is_blocked() {
        let inbox = Arc::new(Inbox::in_memory());
        let (sender, source) = tokio::sync::mpsc::channel(64);
        let (_reader, mut events, _active) = buffer_events_with_health(BoundedSource(source), Arc::clone(&inbox), LIMITS);
        let blocked = inbox.lock_for_test();
        tokio::time::timeout(Duration::from_secs(5), async {
            for n in 0..2048 {
                sender.send(Ok(claimable(&format!("pay-{n}")))).await.unwrap();
            }
            while events.len() != 2048 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the reader keeps reading while a commit waits");
        drop(blocked);
        assert_eq!(events.take().await, Some(Entry::Connected));
        for n in 0..2048 {
            assert_eq!(payment_id_of(events.take().await), format!("pay-{n}"));
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
            Arc::new(Inbox::in_memory()),
            inbox_limit(4, usize::MAX),
        );
        for n in 0..6 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        dealt_with(&asks, 6).await;
        assert_eq!(events.len(), 4);
        assert_eq!(events.take().await, Some(Entry::Connected), "markers don't count toward the limit");
        for n in 0..2 {
            assert_eq!(payment_id_of(events.take().await), format!("pay-{n}"));
        }
        // Two of four held is not fewer than half yet.
        sender.send(Ok(claimable("pay-6"))).unwrap();
        dealt_with(&asks, 7).await;
        for n in 2..4 {
            assert_eq!(payment_id_of(events.take().await), format!("pay-{n}"));
        }
        let hole = tokio::time::timeout(Duration::from_secs(5), events.take()).await;
        assert!(
            matches!(hole, Ok(Some(Entry::Dropped(3)))),
            "pay-4, pay-5 and pay-6 were dropped, and the hole is queued once half is free with no event after it: {hole:?}"
        );
        assert!(events.is_empty());
        sender.send(Ok(claimable("pay-7"))).unwrap();
        dealt_with(&asks, 8).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert_eq!(payment_id_of(events.take().await), "pay-7");
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
            Arc::new(Inbox::in_memory()),
            inbox_limit(100, 2 * size),
        );
        for n in 0..3 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        dealt_with(&asks, 3).await;
        assert_eq!(events.len(), 2, "a third event is over the byte limit");
        saved(events.inbox(), 2).await;
        sender.send(Err(ldk_server_client::error::LdkServerError::new(
            ldk_server_client::error::LdkServerErrorCode::InternalError, "stream failed",
        ))).unwrap();
        reader_ended(&mut reader).await;
        assert!(events.recv().await.is_none(), "an ended reader hands out nothing more");
        let mut left = vec![];
        while let Some(row) = events.inbox().oldest().unwrap() {
            events.finish(row.id).await;
            left.push(row.entry);
        }
        assert_eq!(left, [Entry::Connected, Entry::Event(claimable("pay-0")), Entry::Event(claimable("pay-1"))], "a stream that fails while dropping leaves the hole to the reconnect");
        assert!(!events.is_empty(), "the hole still holds its place, so no checkpoint passes it");
        // One event larger than the whole limit still goes through when nothing else waits.
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(Ok(claimable("pay-big"))).unwrap();
        let (mut reader, mut events, active) =
            buffer_events_with_health(TestSource(receiver), Arc::new(Inbox::in_memory()), inbox_limit(100, 1));
        assert_eq!(events.take().await, Some(Entry::Connected));
        assert_eq!(payment_id_of(events.take().await), "pay-big");
        assert!(active.load(Ordering::Acquire));
        drop(sender);
        reader_ended(&mut reader).await;
        assert_eq!(events.inbox().oldest().unwrap(), None, "nothing was dropped");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn the_production_limits_are_a_million_events_or_1_gib_and_100_000_or_64_mib_in_memory() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!((LIMITS.inbox.events, LIMITS.inbox.bytes), (1_000_000, 1 << 30));
        assert_eq!((LIMITS.handoff.events, LIMITS.handoff.bytes), (100_000, 64 << 20));
        // An inbox one short of the limit takes one more event and drops the next.
        let (_dir, inbox, conn) = inbox_on_disk();
        conn.execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 999999)
             INSERT INTO events (kind, payload, received_ms) SELECT 'event', x'', 0 FROM n;",
        ).unwrap();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (reader, events, _active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        assert_eq!(events.len(), 999_999);
        sender.send(Ok(EventEnvelope::default())).unwrap();
        sender.send(Ok(EventEnvelope::default())).unwrap();
        dealt_with(&asks, 2).await;
        assert_eq!(events.len(), 1_000_000, "the next event is over the inbox limit");
        drop((reader, events, sender));
        // While saving waits, memory takes 100,000 events and drops the next.
        let inbox = Arc::new(Inbox::in_memory());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (reader, events, _active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        let blocked = inbox.lock_for_test();
        for _ in 0..100_001 {
            sender.send(Ok(EventEnvelope::default())).unwrap();
        }
        dealt_with(&asks, 100_001).await;
        assert_eq!(events.len(), 100_000, "the next event is over the hand-off's event limit");
        drop((reader, events, sender, blocked));
        // Events of exactly 1 MiB encoded: 64 wait in memory and the next is over the hand-off's byte limit.
        let overhead = prost::Message::encoded_len(&claimable(&"x".repeat(1 << 20))) - (1 << 20);
        let id = "x".repeat((1 << 20) - overhead);
        assert_eq!(prost::Message::encoded_len(&claimable(&id)), 1 << 20);
        let inbox = Arc::new(Inbox::in_memory());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, events, _active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        let blocked = inbox.lock_for_test();
        for _ in 0..65 {
            sender.send(Ok(claimable(&id))).unwrap();
        }
        dealt_with(&asks, 65).await;
        assert_eq!(events.len(), 64);
        drop(blocked);
    }

    #[tokio::test]
    async fn a_hole_is_queued_ahead_of_the_event_that_ends_it() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, _active) = buffer_events_with_health(
            CountingSource(receiver, Arc::clone(&asks)),
            Arc::new(Inbox::in_memory()),
            inbox_limit(2, usize::MAX),
        );
        for n in 0..3 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        dealt_with(&asks, 3).await;
        assert_eq!(events.take().await, Some(Entry::Connected));
        assert_eq!(payment_id_of(events.take().await), "pay-0");
        // Free the last held event and send the next before the reader runs, so it finds both at once.
        assert_eq!(payment_id_of(events.take().await), "pay-1");
        sender.send(Ok(claimable("pay-3"))).unwrap();
        assert_eq!(events.take().await, Some(Entry::Dropped(1)), "pay-2 was dropped");
        assert_eq!(payment_id_of(events.take().await), "pay-3");
    }

    #[tokio::test]
    async fn a_handled_event_frees_its_room() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (mut reader, mut events, active) =
            buffer_events_with_health(TestSource(receiver), Arc::new(Inbox::in_memory()), inbox_limit(2, usize::MAX));
        sender.send(Ok(claimable("pay-0"))).unwrap();
        sender.send(Ok(claimable("pay-1"))).unwrap();
        holds(&events, 2).await;
        assert_eq!(events.take().await, Some(Entry::Connected));
        assert_eq!(payment_id_of(events.take().await), "pay-0");
        sender.send(Ok(claimable("pay-2"))).unwrap();
        holds(&events, 2).await;
        assert!(active.load(Ordering::Acquire), "the room pay-0 freed took pay-2");
        assert_eq!(payment_id_of(events.take().await), "pay-1");
        assert_eq!(payment_id_of(events.take().await), "pay-2");
        drop(sender);
        reader_ended(&mut reader).await;
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
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::new(Inbox::in_memory()), LIMITS);
        stable_channels::audit::enable_test_capture();
        serve_until_handled(&mut events, &active, &asks, 3, sender, &manager, &db, &ldk, &mut health).await;
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
        let (_reader, mut events, active) = buffer_events_with_health(TestSource(receiver), Arc::new(Inbox::in_memory()), LIMITS);
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
            buffer_events_with_health(TestSource(receiver), Arc::new(Inbox::in_memory()), inbox_limit(3, usize::MAX));
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

    /// Serve a subscription that ends once the reader has read `count` events and every row is handled; returns when that was.
    #[allow(clippy::too_many_arguments)]
    async fn serve_until_handled(
        events: &mut EventBuffer,
        active: &AtomicBool,
        asks: &AtomicUsize,
        count: usize,
        sender: tokio::sync::mpsc::UnboundedSender<EventItem>,
        manager: &tokio::sync::Mutex<StableChannelManager>,
        db: &stable_channels::db::Database,
        ldk: &dyn LdkServerCalls,
        health: &mut StreamHealth,
    ) -> tokio::time::Instant {
        let watch = events.watcher();
        let end = async move {
            while asks.load(Ordering::SeqCst) <= count || !watch.is_empty() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            drop(sender);
            tokio::time::Instant::now()
        };
        let (served, handled) = tokio::join!(tokio::time::timeout(Duration::from_secs(3_600), serve(events, active, manager, db, ldk, health)), end);
        served.expect("serve returns once the subscription ends");
        handled
    }

    /// Save one event into an empty inbox and return its row id.
    fn save_only(inbox: &Inbox, envelope: &EventEnvelope) -> i64 {
        inbox.save(&[(Entry::Event(envelope.clone()), 0)]).unwrap();
        inbox.oldest().unwrap().unwrap().id
    }

    fn claimed(rows: &[(String, serde_json::Value)]) -> Vec<String> {
        rows.iter()
            .filter(|(name, _)| name == "PAYMENT_CLAIMABLE")
            .filter_map(|(_, data)| data["payment_id"].as_str().map(str::to_owned))
            .collect()
    }

    #[tokio::test]
    async fn rows_saved_before_a_reconnect_are_handled_before_its_reconcile() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let inbox = Arc::new(Inbox::in_memory());
        inbox.save(&[(Entry::Event(claimable("old")), 0)]).unwrap();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(Ok(claimable("new"))).unwrap();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        stable_channels::audit::enable_test_capture();
        serve_until_handled(&mut events, &active, &asks, 1, sender, &manager, &db, &ldk, &mut health).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        let at = |payment_id: &str| rows.iter().position(|(name, data)| name == "PAYMENT_CLAIMABLE" && data["payment_id"] == payment_id).unwrap();
        let connect = rows
            .iter()
            .position(|(name, data)| name == "RECONCILIATION_STARTED" && data["trigger"] == "stream_connected")
            .expect("the connect reconcile ran");
        assert!(at("old") < connect && connect < at("new"), "{names:?}");
    }

    #[tokio::test]
    async fn serve_returns_when_the_reader_ends_and_leaves_the_rest_saved() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let inbox = Arc::new(Inbox::in_memory());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        for n in 0..3 {
            sender.send(Ok(claimable(&format!("pay-{n}")))).unwrap();
        }
        let (mut reader, mut events, active) = buffer_events_with_health(TestSource(receiver), Arc::clone(&inbox), LIMITS);
        stable_channels::audit::enable_test_capture();
        let held = manager.lock().await;
        let serving = serve(&mut events, &active, &manager, &db, &ldk, &mut health);
        tokio::pin!(serving);
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut serving).await.is_err(), "handling waits for the manager");
        saved(&inbox, 3).await;
        drop(sender);
        reader_ended(&mut reader).await;
        drop(held);
        tokio::time::timeout(Duration::from_secs(5), serving).await.expect("serve returns once the reader has ended");
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert!(claimed(&rows).len() <= 1, "at most the row it was handling");
        assert_eq!(inbox.held().unwrap().0, 3 - claimed(&rows).len(), "the rest stay saved for the next subscription");
    }

    #[tokio::test]
    async fn back_to_back_connected_rows_reconcile_once_and_then_checkpoint() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = complete_history();
        let gap_from = now_millis() as i64 - 2_000;
        let mut health = StreamHealth::new(db.begin_event_stream_gap(gap_from).unwrap());
        // Subscriptions that ended at once, before their connected rows were handled, as while LDK Server shuts down.
        let inbox = Arc::new(Inbox::in_memory());
        inbox.save(&[(Entry::Connected, 0), (Entry::Connected, 1), (Entry::Connected, 2)]).unwrap();
        let last = inbox.oldest().unwrap().unwrap().id + 2;
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        // A start still waiting to be saved isn't seen yet, which costs at most one more reconcile.
        tokio::time::timeout(Duration::from_secs(5), async {
            while !inbox.next_is_connected(last).unwrap() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the new subscription's start is saved");
        stable_channels::audit::enable_test_capture();
        serve_until_handled(&mut events, &active, &asks, 0, sender, &manager, &db, &ldk, &mut health).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        let connects = rows.iter().filter(|(name, data)| name == "RECONCILIATION_STARTED" && data["trigger"] == "stream_connected").count();
        assert_eq!(connects, 1, "the last of them covers the same open gap: {names:?}");
        assert!(health.gap.is_none(), "the gap closes once they are handled: {names:?}");
        assert!(db.begin_event_stream_gap(now_millis() as i64).unwrap().0 > gap_from);
        assert_eq!(inbox.oldest().unwrap(), None);
    }

    #[tokio::test]
    async fn a_connected_row_with_events_after_it_still_reconciles() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let inbox = Arc::new(Inbox::in_memory());
        inbox.save(&[(Entry::Connected, 0), (Entry::Event(claimable("old")), 1)]).unwrap();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        stable_channels::audit::enable_test_capture();
        serve_until_handled(&mut events, &active, &asks, 0, sender, &manager, &db, &ldk, &mut health).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        let connects: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, (name, data))| name == "RECONCILIATION_STARTED" && data["trigger"] == "stream_connected")
            .map(|(at, _)| at)
            .collect();
        let old = rows.iter().position(|(name, data)| name == "PAYMENT_CLAIMABLE" && data["payment_id"] == "old").unwrap();
        assert_eq!(connects.len(), 2, "an event between them keeps both: {names:?}");
        assert!(connects[0] < old && old < connects[1], "{names:?}");
    }

    #[test]
    fn a_subscription_that_ends_at_once_without_an_event_backs_off() {
        let at_once = Duration::from_millis(5);
        let mut backoff = Duration::from_secs(1);
        let waits: Vec<_> = (0..8).map(|_| resubscribe_delay(false, at_once, &mut backoff)).collect();
        let secs: Vec<_> = waits.iter().map(|wait| wait.map(|wait| wait.as_secs())).collect();
        assert_eq!(secs, [Some(1), Some(2), Some(4), Some(8), Some(16), Some(32), Some(60), Some(60)], "like a failed subscribe");
        assert_eq!(resubscribe_delay(true, at_once, &mut backoff), None, "one that delivered an event resubscribes at once");
        assert_eq!(resubscribe_delay(false, at_once, &mut backoff), Some(Duration::from_secs(1)), "and starts the backoff over");
        assert_eq!(resubscribe_delay(false, STREAM_HEALTH_INTERVAL, &mut backoff), None, "as does one that lasted");
        assert_eq!(backoff, Duration::from_secs(1));
    }

    #[tokio::test]
    async fn an_event_is_marked_started_only_once_the_manager_lock_is_held() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let inbox = Inbox::in_memory();
        let id = save_only(&inbox, &claimable("pay-0"));
        stable_channels::audit::enable_test_capture();
        let held = manager.lock().await;
        let handling = handle(id, claimable("pay-0"), &inbox, &manager, &db, &ldk, &mut health);
        tokio::pin!(handling);
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut handling).await.is_err(), "it waits for the manager");
        assert!(!inbox.oldest().unwrap().unwrap().started, "not marked while it waits");
        assert!(inbox.take_interrupted().unwrap().is_empty(), "a crash now would replay it");
        drop(held);
        handling.await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert_eq!(claimed(&rows), ["pay-0"]);
        assert!(inbox.oldest().unwrap().unwrap().started, "marked while its handler ran, until serve finishes it");
    }

    #[tokio::test(start_paused = true)]
    async fn a_retrying_event_is_not_marked_started_while_it_sleeps() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let (ldk, request, fee) = crate::stable_manager::tests::trade_request(&mut *manager.lock().await);
        ldk.list_channels_fails.store(true, Ordering::SeqCst);
        let payment_id = "c".repeat(64);
        let envelope = paid_request(&payment_id, &request, fee);
        let inbox = Inbox::in_memory();
        let id = save_only(&inbox, &envelope);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let check = async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(!inbox.oldest().unwrap().unwrap().started, "not marked while it waits to try again");
            ldk.list_channels_fails.store(false, Ordering::SeqCst);
        };
        tokio::join!(handle(id, envelope.clone(), &inbox, &manager, &db, &ldk, &mut health), check);
        assert!(db.trade_decision_by_payment(&payment_id).unwrap().is_some(), "decided once LDK Server answered");
        StableChannelManager::retry_pending_trade_responses(&db, &ldk).await;
        assert_eq!(trade_responses(&ldk), 1, "and answered once");
    }

    #[tokio::test(start_paused = true)]
    async fn an_event_whose_start_cannot_be_recorded_is_not_handled() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let (_dir, inbox, conn) = inbox_on_disk();
        let id = save_only(&inbox, &claimable("pay-0"));
        conn.execute_batch("CREATE TRIGGER fail_start BEFORE UPDATE OF started ON events BEGIN SELECT RAISE(FAIL, 'disk full'); END;").unwrap();
        stable_channels::audit::enable_test_capture();
        let fix = async {
            tokio::time::sleep(Duration::from_millis(2_500)).await;
            let rows = stable_channels::audit::drain_test_capture();
            assert!(claimed(&rows).is_empty(), "nothing is handled before its start is recorded");
            assert!(rows.iter().any(|(name, data)| name == "DB_WRITE_FAILED" && data["op"] == "mark_event_started"), "{rows:?}");
            conn.execute_batch("DROP TRIGGER fail_start").unwrap();
        };
        tokio::join!(handle(id, claimable("pay-0"), &inbox, &manager, &db, &ldk, &mut health), fix);
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert_eq!(claimed(&rows), ["pay-0"], "handled once, after its start was recorded");
    }

    #[tokio::test]
    async fn an_unreadable_row_is_recorded_and_skipped() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let (_dir, inbox, conn) = inbox_on_disk();
        inbox.save(&[(Entry::Event(claimable("bad")), 5), (Entry::Event(claimable("good")), 6)]).unwrap();
        let bad = inbox.oldest().unwrap().unwrap().id;
        conn.execute("UPDATE events SET payload = x'ff' WHERE id = ?1", [bad]).unwrap();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        stable_channels::audit::enable_test_capture();
        serve_until_handled(&mut events, &active, &asks, 0, sender, &manager, &db, &ldk, &mut health).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let unreadable: Vec<_> = rows.iter().filter(|(name, _)| name == "EVENT_UNREADABLE").collect();
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0].1["row_id"], bad);
        assert_eq!(unreadable[0].1["received_ms"], 5);
        assert_eq!(claimed(&rows), ["good"], "the queue keeps moving");
        assert_eq!(inbox.oldest().unwrap(), None);
    }

    #[tokio::test]
    async fn the_checkpoint_waits_for_every_row() {
        let dir = tempfile::tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        let gap = db.begin_event_stream_gap(1_000).unwrap();
        let mut health = StreamHealth::new(gap.clone());
        health.reconciliation_finished(&complete_counts(), completed_result(), true);
        let inbox = Arc::new(Inbox::in_memory());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (_reader, mut events, active) = buffer_events_with_health(TestSource(receiver), Arc::clone(&inbox), LIMITS);
        assert_eq!(events.take().await, Some(Entry::Connected));
        let id = save_only(&inbox, &claimable("pay-0"));
        checkpoint_if_ready(&mut health, &db, &active, &events);
        assert_eq!(db.begin_event_stream_gap(2_000).unwrap(), gap, "a saved row holds the gap open");
        inbox.set_started(id, true).unwrap();
        checkpoint_if_ready(&mut health, &db, &active, &events);
        assert_eq!(db.begin_event_stream_gap(3_000).unwrap(), gap, "so does one being handled");
        events.finish(id).await;
        checkpoint_if_ready(&mut health, &db, &active, &events);
        assert!(health.gap.is_none(), "it closes once every row is handled");
        drop(sender);
    }

    /// Trade decisions sent to the wallet, leaving out SYNCs.
    fn trade_responses(ldk: &crate::stable_manager::tests::FakeLdkServer) -> usize {
        ldk.sends.lock().unwrap().iter().filter(|send| send.custom_tlvs.iter().any(|tlv| String::from_utf8_lossy(&tlv.value).contains("TRADE_"))).count()
    }

    #[tokio::test]
    async fn a_row_still_marked_started_is_not_handled_again() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let inbox = Arc::new(Inbox::in_memory());
        // Handled already, but its delete didn't take effect.
        let id = save_only(&inbox, &claimable("pay-0"));
        inbox.set_started(id, true).unwrap();
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        stable_channels::audit::enable_test_capture();
        serve_until_handled(&mut events, &active, &asks, 0, sender, &manager, &db, &ldk, &mut health).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert!(claimed(&rows).is_empty(), "not handled a second time");
        let interrupted: Vec<_> = rows.iter().filter(|(name, _)| name == "EVENT_INTERRUPTED").collect();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].1["row_id"], id);
        assert_eq!(inbox.oldest().unwrap(), None);
    }

    #[tokio::test]
    async fn building_a_reader_does_not_wait_on_the_inbox() {
        let inbox = Arc::new(Inbox::in_memory());
        let (_sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let blocked = inbox.lock_for_test();
        let building = {
            let inbox = Arc::clone(&inbox);
            tokio::task::spawn_blocking(move || drop(start_reader(TestSource(receiver), inbox, (0, 0), LIMITS)))
        };
        let built = tokio::time::timeout(Duration::from_secs(2), building).await;
        drop(blocked);
        assert!(built.is_ok(), "the subscription is read from as soon as it is open, so nothing may wait on the inbox first");
    }

    #[tokio::test(start_paused = true)]
    async fn startup_waits_a_while_for_a_price_to_decide_trades_with() {
        let started = tokio::time::Instant::now();
        let asked = AtomicUsize::new(0);
        let priced = wait_for_price(|| if asked.fetch_add(1, Ordering::SeqCst) < 3 { 0.0 } else { 100_000.0 }, Duration::from_secs(60)).await;
        assert!(priced);
        assert_eq!(tokio::time::Instant::now() - started, Duration::from_secs(3), "handling starts as soon as a price is there");
        let started = tokio::time::Instant::now();
        assert!(!wait_for_price(|| 0.0, Duration::from_secs(60)).await);
        assert_eq!(tokio::time::Instant::now() - started, Duration::from_secs(60), "and no later than the limit without one");
    }

    /// One event, then a gate; once opened, a burst of events that are all ready at once, each taking a moment to read.
    struct BurstSource {
        first: bool,
        gate: Option<tokio::sync::oneshot::Receiver<()>>,
        read: usize,
        inbox: Arc<Inbox>,
        saved_during_burst: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl EventSource for BurstSource {
        async fn next_event(&mut self) -> Option<EventItem> {
            if std::mem::take(&mut self.first) {
                return Some(Ok(claimable("pay-0")));
            }
            if let Some(gate) = self.gate.take() {
                let _ = gate.await;
            }
            if self.read == 6_000 {
                std::future::pending::<()>().await;
            }
            self.read += 1;
            std::thread::sleep(Duration::from_micros(200));
            if self.read == 5_000 {
                self.saved_during_burst.store(self.inbox.held().unwrap().0 > 0, Ordering::SeqCst);
            }
            Some(Ok(claimable(&format!("burst-{}", self.read))))
        }
    }

    #[tokio::test]
    async fn a_failed_save_is_retried_during_a_burst() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_dir, inbox, conn) = inbox_on_disk();
        conn.execute_batch(FAIL_EVENT_SAVES).unwrap();
        let (open, gate) = tokio::sync::oneshot::channel();
        let saved_during_burst = Arc::new(AtomicBool::new(false));
        let source = BurstSource { first: true, gate: Some(gate), read: 0, inbox: Arc::clone(&inbox), saved_during_burst: Arc::clone(&saved_during_burst) };
        let (_reader, _events, _active) = buffer_events_with_health(source, Arc::clone(&inbox), LIMITS);
        // The first save fails and is due again 1 s later, half a second into a 1.2 s burst.
        tokio::time::sleep(Duration::from_millis(200)).await;
        conn.execute_batch("DROP TRIGGER fail_save").unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        open.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(2_000)).await;
        assert!(saved_during_burst.load(Ordering::SeqCst), "the retry ran when due, not after the burst");
    }

    fn forwarded() -> EventEnvelope {
        use ldk_server_client::ldk_server_grpc::events::PaymentForwarded;
        EventEnvelope { event: Some(EventVariant::PaymentForwarded(PaymentForwarded { outbound_amount_forwarded_msat: 5_000, ..Default::default() })) }
    }

    #[tokio::test]
    async fn an_event_started_before_a_crash_is_recorded_and_not_replayed() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        inbox.save(&[(Entry::Event(forwarded()), 41), (Entry::Event(claimable("pay-0")), 42)]).unwrap();
        let forward = inbox.oldest().unwrap().unwrap().id;
        inbox.set_started(forward, true).unwrap();
        // The daemon stops while the forward's handler runs.
        drop(inbox);
        stable_channels::audit::enable_test_capture();
        let inbox = start_inbox(dir.path()).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let interrupted: Vec<_> = rows.iter().filter(|(name, _)| name == "EVENT_INTERRUPTED").collect();
        assert_eq!(interrupted.len(), 1, "{rows:?}");
        assert_eq!(interrupted[0].1["row_id"], forward);
        assert_eq!(interrupted[0].1["event"], "payment_forwarded");
        assert_eq!(interrupted[0].1["received_ms"], 41);
        assert!(interrupted[0].1["payment_id"].is_null());
        assert_eq!(inbox.oldest().unwrap().unwrap().entry, Entry::Event(claimable("pay-0")), "the forward isn't replayed, the next row is");
    }

    #[tokio::test(start_paused = true)]
    async fn a_trade_request_interrupted_while_retrying_is_decided_once_after_a_restart() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let (ldk, request, fee) = crate::stable_manager::tests::trade_request(&mut *manager.lock().await);
        ldk.list_channels_fails.store(true, Ordering::SeqCst);
        let payment_id = "c".repeat(64);
        let envelope = paid_request(&payment_id, &request, fee);
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        let id = save_only(&inbox, &envelope);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        // The daemon stops 2 s in, while the request waits to be tried again.
        let crashed = tokio::time::timeout(Duration::from_secs(2), handle(id, envelope, &inbox, &manager, &db, &ldk, &mut health)).await;
        assert!(crashed.is_err(), "still waiting for LDK Server");
        drop(inbox);
        stable_channels::audit::enable_test_capture();
        let inbox = start_inbox(dir.path()).await;
        ldk.list_channels_fails.store(false, Ordering::SeqCst);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        serve_until_handled(&mut events, &active, &asks, 0, sender, &manager, &db, &ldk, &mut health).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert!(!rows.iter().any(|(name, _)| name == "EVENT_INTERRUPTED"), "a request only waiting wasn't started");
        assert!(db.trade_decision_by_payment(&payment_id).unwrap().is_some(), "decided after the restart");
        StableChannelManager::retry_pending_trade_responses(&db, &ldk).await;
        assert_eq!(trade_responses(&ldk), 1, "and answered once");
    }

    #[tokio::test]
    async fn events_saved_before_a_restart_come_before_the_new_subscription() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(dir.path()).unwrap();
        inbox.save(&[(Entry::Event(claimable("pay-0")), 0), (Entry::Event(claimable("pay-1")), 1)]).unwrap();
        drop(inbox);
        let inbox = start_inbox(dir.path()).await;
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(Ok(claimable("pay-2"))).unwrap();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::clone(&inbox), LIMITS);
        stable_channels::audit::enable_test_capture();
        serve_until_handled(&mut events, &active, &asks, 1, sender, &manager, &db, &ldk, &mut health).await;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(claimed(&rows), ["pay-0", "pay-1", "pay-2"]);
        let at = |payment_id: &str| rows.iter().position(|(name, data)| name == "PAYMENT_CLAIMABLE" && data["payment_id"] == payment_id).unwrap();
        let connect = rows.iter().position(|(name, data)| name == "RECONCILIATION_STARTED" && data["trigger"] == "stream_connected").unwrap();
        assert!(at("pay-1") < connect && connect < at("pay-2"), "{names:?}");
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
        let envelope = paid_request(&payment_id, &request, fee);
        let inbox = Inbox::in_memory();
        let id = save_only(&inbox, &envelope);
        tokio::join!(handle(id, envelope, &inbox, &manager, &db, &ldk, &mut health), recover);
        let waited = tokio::time::Instant::now() - started;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
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
        let envelope = paid_request(&payment_id, &request, fee);
        let inbox = Inbox::in_memory();
        let id = save_only(&inbox, &envelope);
        let started = tokio::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(3_600), handle(id, envelope, &inbox, &manager, &db, &ldk, &mut health))
            .await
            .expect("handling gives up");
        let waited = tokio::time::Instant::now() - started;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        assert!(waited >= MAX_EVENT_RETRY && waited < MAX_EVENT_RETRY + MAX_RETRY_WAIT, "{waited:?}");
        let abandoned: Vec<_> = rows.iter().filter(|(name, _)| name == "EVENT_RETRY_ABANDONED").collect();
        assert_eq!(abandoned.len(), 1);
        assert_eq!(abandoned[0].1["payment_id"], payment_id.as_str());
        assert!(db.trade_decision_by_payment(&payment_id).unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn unsafe_settlement_outcomes_are_retried_in_place_backing_off_to_a_minute() {
        let _guard = crate::stable_manager::tests::AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (manager, db) = serving_fixture();
        db.record_settlement("sync-2", "sync").unwrap();
        let ldk = crate::stable_manager::tests::FakeLdkServer::new(vec![]);
        ldk.payment_lookup_failures.store(usize::MAX, Ordering::SeqCst);
        let mut health = StreamHealth::new(db.begin_event_stream_gap(1_000).unwrap());
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(Ok(claimable("buffered"))).unwrap();
        let asks = Arc::new(AtomicUsize::new(0));
        let (_reader, mut events, active) = buffer_events_with_health(CountingSource(receiver, Arc::clone(&asks)), Arc::new(Inbox::in_memory()), LIMITS);
        stable_channels::audit::enable_test_capture();
        let started = tokio::time::Instant::now();
        // Lookups work again after 150 s; passes run at 0, 1, 3, 7, 15, 31, 63, 123 and 183 s.
        let recover = async {
            tokio::time::sleep(Duration::from_secs(150)).await;
            ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        };
        let (handled, ()) = tokio::join!(serve_until_handled(&mut events, &active, &asks, 1, sender, &manager, &db, &ldk, &mut health), recover);
        let waited = handled - started;
        let rows = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        let passes = Duration::from_secs(1 + 2 + 4 + 8 + 16 + 32 + 60 + 60);
        assert!(waited >= passes && waited < passes + Duration::from_secs(1), "{waited:?} {names:?}");
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
        let (mut reader, mut events, active) = buffer_events_with_health(TestSource(receiver), Arc::new(Inbox::in_memory()), LIMITS);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !active.load(Ordering::Acquire) { tokio::task::yield_now().await; }
        }).await.unwrap();
        reader.abort_all();
        assert!(reader.join_next().await.unwrap().unwrap_err().is_cancelled());
        assert!(!active.load(Ordering::Acquire));
        assert!(events.recv().await.is_none(), "an aborted reader counts as ended");
    }

    #[tokio::test]
    async fn a_stream_error_marks_the_reader_unhealthy_and_leaves_its_events_saved() {
        use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (mut reader, mut events, active) = buffer_events_with_health(TestSource(receiver), Arc::new(Inbox::in_memory()), LIMITS);
        sender.send(Ok(EventEnvelope::default())).unwrap();
        sender.send(Err(LdkServerError::new(
            LdkServerErrorCode::InternalServerError, "stream failed during backfill",
        ))).unwrap();
        reader.join_next().await.unwrap().unwrap();
        assert!(!active.load(Ordering::Acquire));
        assert_eq!(events.inbox().held().unwrap(), (1, 0), "the event read before the error is saved");
        assert!(events.recv().await.is_none(), "the error itself isn't saved, and the ended reader hands out nothing");
        assert!(!events.is_empty());
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
        let d = connected_audit_data(Some("event-stream-gap-1"), Some("0.1.0 (aa2d60f51ebacc7edd5dbdfb90b53a9c73ee0bd3)"));
        assert_eq!(d["correlation_id"], "event-stream-gap-1");
        assert_eq!(d["ldk_server_version"], "0.1.0 (aa2d60f51ebacc7edd5dbdfb90b53a9c73ee0bd3)");
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
