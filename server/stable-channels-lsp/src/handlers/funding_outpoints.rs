//! FundingOutpoints handler: a wallet restored without its channel data asks for its channels' funding outpoints to find a force close.
//! Outside the auth layer like /api/channel-exists, but the node key signs the request: an outpoint ties a private channel on chain to the node.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::lightning::util::message_signing;
use ldk_server_client::ldk_server_grpc::api::{GetNodeInfoRequest, ListChannelsRequest};
use ldk_server_client::ldk_server_grpc::types::Channel;
use serde_json::{json, Value};
use stable_channels::db::{Database, LedgerChannelRow};

use crate::handlers::channel_exists::is_valid_node_id;
use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

/// Freshness window for the signed timestamp, in seconds (the same as RegisterPush).
const SIG_WINDOW_SECS: u64 = 300;
/// A zbase32 node signature is 104 characters; a much longer one is refused before any work.
const MAX_SIGNATURE_CHARS: usize = 150;
/// Lookups per window for each node that has channels with the LSP.
const MAX_REQUESTS_PER_WINDOW: u32 = 10;
const RATE_WINDOW_SECS: u64 = 60;
/// How long one ListChannels answer is reused after its read ends, so a flood of signed requests can't load LDK Server.
const CHANNEL_CACHE: Duration = Duration::from_secs(5);
/// Gap between lookups of the LSP's own node id while it is unknown.
const NODE_ID_RETRY_SECS: u64 = 10;
/// How long a call to LDK Server may take before the request gets a 503.
const LDK_TIMEOUT: Duration = Duration::from_secs(10);
/// Newest outputs returned per channel; the one a close spends is among them unless one splice gets 10+ fee-bump candidates.
const MAX_OUTPUTS_PER_CHANNEL: usize = 10;
/// Closed channels looked up per request, newest first; only the node's own channel opens add to them.
const MAX_CLOSED_CHANNELS: usize = 20;
/// Newest channel records read per lookup; a real node has a few per channel, plus one per event-stream reconnect that changed it.
const MAX_LINK_ROWS: usize = 5_000;

/// Ledger events whose `counterparty_node_id` and `user_channel_id` name the same channel.
const LINK_EVENTS: &[&str] =
    &["CHANNEL_PENDING", "SPLICE_NEGOTIATED", "CHANNEL_RECONSTRUCTED", "CHANNEL_CLOSED", "CHANNEL_OPEN_FAILED"];
/// Ledger events that record a channel's funding outpoint, that its funding never confirmed, or its close (naming the counterparty).
const FUNDING_EVENTS: &[&str] = &[
    "CHANNEL_CLOSED",
    "CHANNEL_PENDING",
    "CHANNEL_READY",
    "CHANNEL_READY_TRACKED",
    "CHANNEL_READY_SPLICE",
    "SPLICE_NEGOTIATED",
    "CHANNEL_RECONSTRUCTED",
    "CHANNEL_OPEN_FAILED",
];

#[derive(serde::Deserialize)]
pub struct FundingOutpointsRequest {
    pub node_id: String,
    pub timestamp: u64,
    pub signature: String,
}

/// One funding output the wallet should watch; `funding_vout` is None when only the txid was recorded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FundingOutpoint {
    /// The same number for every output of one channel, counting from 0 in the order channels first appear in this reply.
    pub channel: usize,
    pub funding_txid: String,
    pub funding_vout: Option<u32>,
    pub state: &'static str,
}

/// Fixed-window request counter for one node.
#[derive(Debug, Default)]
struct RequestWindow {
    started_at: u64,
    count: u32,
}

impl RequestWindow {
    /// Whether `now` (unix seconds) falls in this window; a clock stepped back does not, so it cannot lock a node out.
    fn current(&self, now: u64) -> bool {
        now >= self.started_at && now - self.started_at < RATE_WINDOW_SECS
    }

    /// Whether this window's budget is spent at `now`, without counting a request.
    fn spent(&self, now: u64) -> bool {
        self.current(now) && self.count >= MAX_REQUESTS_PER_WINDOW
    }

    /// Count a request at `now`; false once this window's budget is spent.
    fn allow(&mut self, now: u64) -> bool {
        if !self.current(now) {
            self.started_at = now;
            self.count = 0;
        }
        if self.count >= MAX_REQUESTS_PER_WINDOW {
            return false;
        }
        self.count += 1;
        true
    }
}

/// Request windows of the nodes that have a channel record, and when stale ones were last dropped.
#[derive(Default)]
struct Budgets {
    windows: HashMap<String, RequestWindow>,
    pruned_at: u64,
}

/// LDK Server's channel list as last read, stamped when the read ended; `channels` is None when the read failed.
struct ChannelSnapshot {
    read_at: tokio::time::Instant,
    channels: Option<Arc<Vec<Channel>>>,
}

/// Shared state of the route: the LSP's node id, a short-lived channel list and per-node budgets.
#[derive(Default)]
pub struct RouteState {
    lsp_node_id: OnceLock<String>,
    next_node_id_lookup: Mutex<u64>,
    channels: Arc<tokio::sync::Mutex<Option<ChannelSnapshot>>>,
    budgets: Mutex<Budgets>,
    // Nodes with a lookup running.
    lookups: Mutex<HashSet<String>>,
    // Test hook: the next ledger read waits here, so a test can hold one in progress.
    #[cfg(test)]
    read_gate: Mutex<Option<tests::ReadGate>>,
}

/// Marks a node's lookup as running until dropped; it moves into the ledger read, so a cancelled request keeps it until the read ends.
struct Lookup {
    route: Arc<RouteState>,
    node_id: String,
}

impl Drop for Lookup {
    fn drop(&mut self) {
        self.route.lookups.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.node_id);
    }
}

impl RouteState {
    /// Keep LDK Server's node id for the signed request check; an empty id is ignored.
    pub fn remember_lsp_node_id(&self, node_id: &str) {
        if !node_id.is_empty() {
            let _ = self.lsp_node_id.set(node_id.to_ascii_lowercase());
        }
    }

    /// The LSP's node id; while unknown it is read from LDK Server at most every NODE_ID_RETRY_SECS.
    async fn lsp_node_id(&self, ldk: &dyn LdkServerCalls, now: u64) -> Option<&str> {
        if self.lsp_node_id.get().is_none() {
            let due = {
                let mut next = self.next_node_id_lookup.lock().unwrap_or_else(PoisonError::into_inner);
                // A clock stepped back past the last lookup also makes one due.
                let due = now >= *next || now.saturating_add(NODE_ID_RETRY_SECS) < *next;
                if due {
                    *next = now.saturating_add(NODE_ID_RETRY_SECS);
                }
                due
            };
            if due {
                match tokio::time::timeout(LDK_TIMEOUT, ldk.get_node_info(GetNodeInfoRequest {})).await {
                    Ok(Ok(info)) => self.remember_lsp_node_id(&info.node_id),
                    Ok(Err(e)) => tracing::error!("[funding-outpoints] get_node_info failed: {}", e),
                    Err(_) => tracing::error!("[funding-outpoints] get_node_info timed out"),
                }
            }
        }
        self.lsp_node_id.get().map(String::as_str)
    }

    /// LDK Server's channel list, reused for CHANNEL_CACHE after each read ends; a failed read is reused too.
    async fn channels(&self, ldk: Arc<dyn LdkServerCalls>) -> Option<Arc<Vec<Channel>>> {
        let mut cached = Arc::clone(&self.channels).lock_owned().await;
        // Timed after taking the lock, so requests queued behind a slow read reuse its result.
        if let Some(snapshot) = cached.as_ref().filter(|s| s.read_at.elapsed() < CHANNEL_CACHE) {
            return snapshot.channels.clone();
        }
        // The read runs in its own task, so it fills the cache even if this request is dropped.
        let read = tokio::spawn(async move {
            let channels = match tokio::time::timeout(LDK_TIMEOUT, ldk.list_channels(ListChannelsRequest {})).await {
                Ok(Ok(r)) => Some(Arc::new(r.channels)),
                Ok(Err(e)) => {
                    tracing::error!("[funding-outpoints] list_channels failed: {}", e);
                    None
                },
                Err(_) => {
                    tracing::error!("[funding-outpoints] list_channels timed out");
                    None
                },
            };
            *cached = Some(ChannelSnapshot { read_at: tokio::time::Instant::now(), channels: channels.clone() });
            channels
        });
        read.await.ok().flatten()
    }

    /// Start a lookup for a node; None while another one for the same node is running.
    fn start_lookup(self: &Arc<Self>, node_id: &str) -> Option<Lookup> {
        let started = self.lookups.lock().unwrap_or_else(PoisonError::into_inner).insert(node_id.to_string());
        started.then(|| Lookup { route: Arc::clone(self), node_id: node_id.to_string() })
    }

    /// Whether a node already spent its budget for this window; counts nothing.
    fn spent(&self, node_id: &str, now: u64) -> bool {
        let budgets = self.budgets.lock().unwrap_or_else(PoisonError::into_inner);
        budgets.windows.get(node_id).is_some_and(|w| w.spent(now))
    }

    /// Count a lookup for a node that has a channel record; false once its budget for this window is spent.
    fn allow(&self, node_id: &str, now: u64) -> bool {
        let mut budgets = self.budgets.lock().unwrap_or_else(PoisonError::into_inner);
        // Windows over a minute old are dropped at most once a minute, not on every new node.
        let due = now < budgets.pruned_at || now - budgets.pruned_at >= RATE_WINDOW_SECS;
        if due && !budgets.windows.contains_key(node_id) {
            budgets.windows.retain(|_, w| w.current(now));
            budgets.pruned_at = now;
        }
        budgets.windows.entry(node_id.to_string()).or_default().allow(now)
    }

    #[cfg(test)]
    fn wait_at_read_gate(&self) {
        let gate = self.read_gate.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(gate) = gate {
            let _ = gate.reached.send(());
            let _ = gate.open.recv_timeout(Duration::from_secs(5));
        }
    }
}

#[derive(Debug, Clone)]
struct Funding {
    txid: String,
    vout: Option<u32>,
    at_ms: i64,
}

/// Parse a `txid:vout` or bare `txid` funding outpoint.
fn parse_funding(text: &str) -> Option<(String, Option<u32>)> {
    let (txid, vout) = match text.split_once(':') {
        Some((txid, vout)) => (txid, Some(vout.parse::<u32>().ok()?)),
        None => (text, None),
    };
    (txid.len() == 64 && txid.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| (txid.to_ascii_lowercase(), vout))
}

/// Every funding output recorded for one channel, once each; a close spends one of them, older ones were spent by splices.
#[derive(Default)]
struct ChannelOutputs {
    outputs: Vec<Funding>,
    open_failed: bool,
}

impl ChannelOutputs {
    fn add(&mut self, next: Funding) {
        match self.outputs.iter_mut().find(|f| f.txid == next.txid) {
            Some(known) => {
                known.vout = known.vout.or(next.vout);
                known.at_ms = known.at_ms.min(next.at_ms);
            },
            None => self.outputs.push(next),
        }
    }
}

/// The node's channels: live ones from LDK Server, and any the ledger links to it as counterparty.
struct NodeChannels<'a> {
    node_id: &'a str,
    live: HashMap<&'a str, &'a Channel>,
    // user_channel_ids of other nodes' live channels.
    foreign: HashSet<&'a str>,
    // Whether the LSP has any channel record with the node, a failed open included.
    known: bool,
    ids: BTreeSet<String>,
}

/// Find the channels the node has or had with the LSP from its newest `max_link_rows` channel records; `node_id` is lowercase.
fn node_channels<'a>(db: &Database, live_all: &'a [Channel], node_id: &'a str, max_link_rows: usize) -> rusqlite::Result<NodeChannels<'a>> {
    let live: HashMap<&str, &Channel> = live_all
        .iter()
        .filter(|c| c.counterparty_node_id.eq_ignore_ascii_case(node_id))
        .map(|c| (c.user_channel_id.as_str(), c))
        .collect();
    // Each ledger channel's latest link row, and the channels whose open failed.
    let mut last_seen: HashMap<String, i64> = HashMap::new();
    let mut failed = HashSet::new();
    for row in db.newest_ledger_channel_rows_by_ref("node_id", node_id, LINK_EVENTS, max_link_rows)? {
        // Only rows where this node is the channel's counterparty; a forward also names the other peer.
        if !row.counterparty_node_id.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(node_id)) {
            continue;
        }
        if let Some(uid) = row.user_channel_id.as_deref().filter(|u| !u.is_empty()) {
            if row.event_type == "CHANNEL_OPEN_FAILED" {
                failed.insert(uid.to_string());
            }
            let at = last_seen.entry(uid.to_string()).or_insert(row.occurred_at_ms);
            *at = (*at).max(row.occurred_at_ms);
        }
    }
    let known = !live.is_empty() || !last_seen.is_empty();
    // A failed open that isn't live has no outpoint to return, so it is settled here without a lookup.
    let mut closed: Vec<(i64, String)> = last_seen
        .into_iter()
        .filter(|(uid, _)| !live.contains_key(uid.as_str()) && !failed.contains(uid))
        .map(|(uid, at)| (at, uid))
        .collect();
    closed.sort_unstable_by(|a, b| b.cmp(a));
    let mut ids: BTreeSet<String> = live.keys().map(|uid| uid.to_string()).collect();
    ids.extend(closed.into_iter().take(MAX_CLOSED_CHANNELS).map(|(_, uid)| uid));
    // Only needed when there is something to look up.
    let foreign = match known {
        true => live_all
            .iter()
            .filter(|c| !c.counterparty_node_id.eq_ignore_ascii_case(node_id))
            .map(|c| c.user_channel_id.as_str())
            .collect(),
        false => HashSet::new(),
    };
    Ok(NodeChannels { node_id, live, foreign, known, ids })
}

/// Whether a user_channel_id is a random 128-bit value, which no other channel shares; one below 2^64 was not drawn at random (an old LDK left inbound channels at 0).
fn drawn_at_random(uid: &str) -> bool {
    uid.parse::<u128>().is_ok_and(|id| id > u128::from(u64::MAX))
}

/// Every funding outpoint of the node's channels, open ones first, then newest first.
fn funding_outpoints_of(db: &Database, channels: &NodeChannels) -> rusqlite::Result<Vec<FundingOutpoint>> {
    let names = |row: &LedgerChannelRow| row.counterparty_node_id.as_deref().map(|n| n.eq_ignore_ascii_case(channels.node_id));
    let mut found = Vec::new();
    for (key, uid) in channels.ids.iter().enumerate() {
        let rows: Vec<LedgerChannelRow> = db
            .ledger_channel_rows_by_ref("user_channel_id", uid, FUNDING_EVENTS)?
            .into_iter()
            .filter(|row| row.user_channel_id.as_deref() == Some(uid.as_str()))
            .collect();
        // Another node's channel may use the same user_channel_id: then only rows naming this node can be attributed.
        let shared = !drawn_at_random(uid)
            || channels.foreign.contains(uid.as_str())
            || rows.iter().any(|row| names(row) == Some(false));
        let mut outputs = ChannelOutputs::default();
        for row in rows {
            match names(&row) {
                Some(false) => continue,
                None if shared => continue,
                _ => {},
            }
            if row.event_type == "CHANNEL_OPEN_FAILED" {
                outputs.open_failed = true;
                continue;
            }
            if let Some((txid, vout)) = row.funding_txo.as_deref().and_then(parse_funding) {
                outputs.add(Funding { txid, vout, at_ms: row.occurred_at_ms });
            }
        }
        let channel = channels.live.get(uid.as_str());
        // LDK Server's current output, which the ledger may have missed.
        let live_txid = channel.and_then(|c| c.funding_txo.as_ref()).and_then(|txo| {
            let (txid, _) = parse_funding(&txo.txid)?;
            outputs.add(Funding { txid: txid.clone(), vout: Some(txo.vout), at_ms: i64::MAX });
            Some(txid)
        });
        if outputs.open_failed && channel.is_none() {
            continue;
        }
        let state = if channel.is_some() { "open" } else { "closed" };
        let mut newest = outputs.outputs;
        newest.sort_by_key(|funding| std::cmp::Reverse(funding.at_ms));
        // The live output is the one a close spends while a splice is pending, so the cap never cuts it.
        let kept = newest
            .into_iter()
            .enumerate()
            .filter(|(i, funding)| *i < MAX_OUTPUTS_PER_CHANNEL || live_txid.as_ref() == Some(&funding.txid))
            .map(|(_, funding)| (key, state, funding));
        found.extend(kept);
    }
    found.sort_by_key(|(_, state, funding)| (*state != "open", std::cmp::Reverse(funding.at_ms)));
    // Number channels by first appearance, so the wallet can group the outputs and check each channel's newest first.
    let mut numbers = HashMap::new();
    Ok(found
        .into_iter()
        .map(|(key, state, f)| {
            let next = numbers.len();
            let channel = *numbers.entry(key).or_insert(next);
            FundingOutpoint { channel, funding_txid: f.txid, funding_vout: f.vout, state }
        })
        .collect())
}

fn json_reply(status: StatusCode, reply: &Value) -> Response {
    let mut response = Response::builder().status(status).header("content-type", "application/json");
    if status == StatusCode::TOO_MANY_REQUESTS {
        response = response.header("retry-after", RATE_WINDOW_SECS.to_string());
    }
    response.body(axum::body::Body::from(reply.to_string())).unwrap()
}

fn refuse(status: StatusCode, error: &str) -> (StatusCode, Value) {
    (status, json!({ "error": error }))
}

/// Validate and verify a request, then look up the node's outpoints, limiting only nodes that have channels.
pub async fn answer<L: LdkServerCalls + 'static>(
    ldk: &Arc<L>,
    db: &Database,
    route: &Arc<RouteState>,
    body: &[u8],
    now: u64,
) -> (StatusCode, Value) {
    let req: FundingOutpointsRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let Some(public_key) = is_valid_node_id(&req.node_id).then(|| PublicKey::from_str(&req.node_id).ok()).flatten()
    else {
        return refuse(StatusCode::BAD_REQUEST, "invalid node_id");
    };
    if now.abs_diff(req.timestamp) > SIG_WINDOW_SECS {
        return refuse(StatusCode::UNAUTHORIZED, "stale timestamp");
    }
    if req.signature.is_empty() || req.signature.len() > MAX_SIGNATURE_CHARS {
        return refuse(StatusCode::UNAUTHORIZED, "invalid signature");
    }
    let Some(lsp_node_id) = route.lsp_node_id(ldk.as_ref(), now).await else {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "LSP node id unavailable");
    };
    // Checked here with LDK's own message signing, so unsigned traffic costs LDK Server nothing.
    let message = crate::messages::funding_outpoints_signed_bytes(&req.node_id, lsp_node_id, req.timestamp);
    if !message_signing::verify(&message, &req.signature, &public_key) {
        tracing::debug!("[funding-outpoints] invalid signature for {}", req.node_id);
        return refuse(StatusCode::UNAUTHORIZED, "invalid signature");
    }
    // The node signs the id exactly as it sends it; lookups use the lowercase form LDK records.
    let node_id = req.node_id.to_ascii_lowercase();
    // One lookup per node at a time, held until the request is counted, so copies sent together can't each read the ledger.
    let Some(lookup) = route.start_lookup(&node_id) else {
        return refuse(StatusCode::TOO_MANY_REQUESTS, "lookup already running");
    };
    // Refused before any read, so a node past its budget costs only the signature check.
    if route.spent(&node_id, now) {
        return refuse(StatusCode::TOO_MANY_REQUESTS, "rate limited");
    }
    let Some(live) = route.channels(ldk.clone()).await else {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "LDK Server unavailable");
    };
    let (db, route, read_node_id) = (db.clone(), Arc::clone(route), node_id.clone());
    // On a blocking thread, so SQLite and the database lock never hold a runtime worker; the guard goes along until the read ends.
    let found = tokio::task::spawn_blocking(move || {
        let _lookup = lookup;
        #[cfg(test)]
        route.wait_at_read_gate();
        node_channels(&db, &live, &read_node_id, MAX_LINK_ROWS).and_then(|channels| {
            // A node with no channel record needs no budget; any record, a failed open included, gets one.
            if channels.known && !route.allow(&read_node_id, now) {
                return Ok(None);
            }
            funding_outpoints_of(&db, &channels).map(Some)
        })
    })
    .await;
    match found {
        Ok(Ok(Some(outpoints))) => {
            if !outpoints.is_empty() {
                tracing::info!("[funding-outpoints] {} outpoint(s) for {}", outpoints.len(), node_id);
            }
            (StatusCode::OK, json!({ "funding_outpoints": outpoints }))
        },
        Ok(Ok(None)) => refuse(StatusCode::TOO_MANY_REQUESTS, "rate limited"),
        Ok(Err(e)) => {
            tracing::error!("[funding-outpoints] ledger read failed: {}", e);
            refuse(StatusCode::SERVICE_UNAVAILABLE, "ledger unavailable")
        },
        Err(e) => {
            tracing::error!("[funding-outpoints] ledger read task failed: {}", e);
            refuse(StatusCode::SERVICE_UNAVAILABLE, "ledger unavailable")
        },
    }
}

/// Wallet-facing POST /api/funding-outpoints. Request and reply are JSON; nothing is written.
pub async fn funding_outpoints(State(state): State<AppState>, body: Bytes) -> Response {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (status, reply) = answer(&state.ldk_server, &state.db, &state.funding_outpoints, &body, now).await;
    json_reply(status, &reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use ldk_node::bitcoin::secp256k1::{Secp256k1, SecretKey};
    use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
    use ldk_server_client::ldk_server_grpc::api::{
        GetNodeInfoResponse, ListChannelsResponse, SignMessageRequest, SignMessageResponse,
        SpontaneousSendRequest, SpontaneousSendResponse, VerifySignatureRequest,
        VerifySignatureResponse,
    };
    use ldk_server_client::ldk_server_grpc::types::OutPoint;
    use stable_channels::ledger::LedgerEventDraft;

    const ALICE: &str = "025a29399255108f82a158c7740053fb19933e3b874c68556deed6dd574da1c0f0";
    const BOB: &str = "0380ba01cf1f8232298198ce1c6da12a7bd159656e79cbc22a5dfbea62ab1aef4b";
    const LSP: &str = "029e0070a45736c1c56c25562c667331472e86e447e64671e591ac1a48b2f6ed1c";
    const TX_SPLICE_3: &str = "c42ee3e279e207297036937b2ccb3a83100f0e71d33410010fc592501b446d7d";
    const ALICE_UID: &str = "5293213951182344713304712262222632001";
    const ALICE_UID_2: &str = "337296444192003436696790865823406081674";
    const BOB_UID: &str = "281056294790600824049085389430669369069";
    const TX_OPEN: &str = "a2a93969722ff1335da9b6b2c029a1f754cfd4ccfea0740a0bf0a41c641a3e86";
    const TX_SPLICE: &str = "cff9ccd9b1725876785a0fac7e4253e257fd48fe19e9119e73d224610bd8dad9";
    const TX_SPLICE_2: &str = "9b0c374d3dfa8f0dadb97cf333b925b40635fb5e160d57db9f15f9c066a6f1c5";
    const TX_OTHER: &str = "602967f738a40de1924475f74a0816160e1575ecb52f5c1c4d92820529de360c";
    const TX_BOB: &str = "7c1843404f0914465d857d3f1e9b3b4fbcbbdfc0b573cdadf55a9d8309fc7b95";
    const NOW: u64 = 1_761_580_800;

    fn fresh_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        (dir, db)
    }

    fn record(db: &Database, event: &str, at_ms: i64, detail: Value) {
        let mut detail = detail;
        detail["occurred_at_ms"] = at_ms.into();
        db.append_ledger_event(&LedgerEventDraft::from_audit_event(event, detail)).unwrap();
    }

    fn opened(db: &Database, node: &str, uid: &str, txo: &str, at_ms: i64) {
        record(db, "CHANNEL_PENDING", at_ms, serde_json::json!({
            "channel_id": format!("{uid}-cid"), "user_channel_id": uid,
            "counterparty_node_id": node, "funding_txo": txo,
        }));
    }

    fn live(node: &str, uid: &str, txid: &str, vout: u32) -> Channel {
        Channel {
            counterparty_node_id: node.to_string(),
            user_channel_id: uid.to_string(),
            funding_txo: Some(OutPoint { txid: txid.to_string(), vout }),
            ..Default::default()
        }
    }

    fn outpoint(channel: usize, txid: &str, vout: Option<u32>, state: &'static str) -> FundingOutpoint {
        FundingOutpoint { channel, funding_txid: txid.to_string(), funding_vout: vout, state }
    }

    fn collect_funding_outpoints(db: &Database, live: &[Channel], node_id: &str) -> rusqlite::Result<Vec<FundingOutpoint>> {
        funding_outpoints_of(db, &node_channels(db, live, node_id, MAX_LINK_ROWS)?)
    }

    #[test]
    fn a_closed_channel_is_found_from_its_open_event() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        record(&db, "CHANNEL_CLOSED", 5_000, serde_json::json!({
            "channel_id": "c", "user_channel_id": ALICE_UID, "counterparty_node_id": ALICE,
        }));
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(1), "closed")]);
    }

    fn negotiated(db: &Database, uid: &str, txo: &str, at_ms: i64) {
        record(db, "SPLICE_NEGOTIATED", at_ms, serde_json::json!({
            "user_channel_id": uid, "counterparty_node_id": ALICE, "funding_txo": txo,
        }));
    }

    fn splice_locked(db: &Database, uid: &str, txo: &str, at_ms: i64) {
        record(db, "CHANNEL_READY_SPLICE", at_ms, serde_json::json!({"user_channel_id": uid, "funding_txo": txo}));
    }

    #[test]
    fn a_random_ids_splice_known_only_from_its_lock_row_is_returned() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        // Negotiated during a stream gap, so the lock row, which names no counterparty, is its only record.
        splice_locked(&db, ALICE_UID, &format!("{TX_SPLICE}:1"), 2_000);
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_SPLICE, Some(1), "closed"), outpoint(0, TX_OPEN, Some(0), "closed")]);
    }

    #[test]
    fn every_output_of_a_spliced_channel_is_returned_newest_first() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        negotiated(&db, ALICE_UID, &format!("{TX_SPLICE}:1"), 2_000);
        splice_locked(&db, ALICE_UID, &format!("{TX_SPLICE}:1"), 2_600);
        // A second splice that has not locked yet.
        negotiated(&db, ALICE_UID, &format!("{TX_SPLICE_3}:2"), 3_000);
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [
            outpoint(0, TX_SPLICE_3, Some(2), "closed"),
            outpoint(0, TX_SPLICE, Some(1), "closed"),
            outpoint(0, TX_OPEN, Some(0), "closed"),
        ]);
    }

    #[test]
    fn late_and_out_of_order_records_drop_no_output() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        record(&db, "CHANNEL_READY_TRACKED", 1_100, serde_json::json!({"user_channel_id": ALICE_UID, "funding_txo": format!("{TX_OPEN}:0")}));
        negotiated(&db, ALICE_UID, &format!("{TX_SPLICE}:1"), 2_000);
        negotiated(&db, ALICE_UID, &format!("{TX_SPLICE_3}:2"), 3_000);
        // A reconnect snapshot shows the second splice, then the first one's deferred lock record lands.
        record(&db, "CHANNEL_RECONSTRUCTED", 3_500, serde_json::json!({
            "user_channel_id": ALICE_UID, "counterparty_node_id": ALICE, "funding_txo": TX_SPLICE_3,
        }));
        splice_locked(&db, ALICE_UID, &format!("{TX_SPLICE}:1"), 4_000);
        record(&db, "CHANNEL_CLOSED", 5_000, serde_json::json!({"user_channel_id": ALICE_UID, "counterparty_node_id": ALICE}));
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [
            outpoint(0, TX_SPLICE_3, Some(2), "closed"),
            outpoint(0, TX_SPLICE, Some(1), "closed"),
            outpoint(0, TX_OPEN, Some(0), "closed"),
        ]);
    }

    #[test]
    fn an_output_recorded_several_times_is_listed_once_with_its_vout() {
        let (_dir, db) = fresh_db();
        record(&db, "CHANNEL_RECONSTRUCTED", 1_000, serde_json::json!({
            "user_channel_id": ALICE_UID, "counterparty_node_id": ALICE, "funding_txo": TX_OPEN,
        }));
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:1"), 2_000);
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(1), "closed")]);
    }

    #[test]
    fn a_txid_only_snapshot_is_returned_without_a_vout() {
        let (_dir, db) = fresh_db();
        record(&db, "CHANNEL_RECONSTRUCTED", 1_000, serde_json::json!({
            "user_channel_id": ALICE_UID, "counterparty_node_id": ALICE, "funding_txo": TX_OPEN,
        }));
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, None, "closed")]);
    }

    #[test]
    fn every_output_of_a_live_channel_is_open() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        negotiated(&db, ALICE_UID, &format!("{TX_SPLICE}:1"), 2_000);
        let found = collect_funding_outpoints(&db, &[live(ALICE, ALICE_UID, TX_OPEN, 0)], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_SPLICE, Some(1), "open"), outpoint(0, TX_OPEN, Some(0), "open")]);
    }

    #[test]
    fn a_live_output_the_ledger_never_saw_is_listed_first() {
        let (_dir, db) = fresh_db();
        negotiated(&db, ALICE_UID, &format!("{TX_SPLICE}:1"), 2_000);
        let found = collect_funding_outpoints(&db, &[live(ALICE, ALICE_UID, TX_OPEN, 0)], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(0), "open"), outpoint(0, TX_SPLICE, Some(1), "open")]);
    }

    #[test]
    fn a_row_naming_the_node_outside_its_counterparty_field_links_no_channel() {
        let (_dir, db) = fresh_db();
        record(&db, "SPLICE_NEGOTIATED", 1_000, serde_json::json!({
            "user_channel_id": BOB_UID, "counterparty_node_id": BOB, "funding_txo": format!("{TX_BOB}:0"),
            "previous": {"node_id": ALICE},
        }));
        assert!(collect_funding_outpoints(&db, &[], ALICE).unwrap().is_empty());
    }

    #[test]
    fn a_row_without_a_counterparty_links_no_channel() {
        let (_dir, db) = fresh_db();
        record(&db, "CHANNEL_CLOSED", 2_000, serde_json::json!({"user_channel_id": BOB_UID, "previous": {"node_id": ALICE}}));
        splice_locked(&db, BOB_UID, &format!("{TX_BOB}:0"), 1_000);
        assert!(collect_funding_outpoints(&db, &[], ALICE).unwrap().is_empty());
    }

    #[test]
    fn a_row_of_another_channel_naming_this_one_adds_nothing() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        record(&db, "CHANNEL_READY_SPLICE", 2_000, serde_json::json!({
            "user_channel_id": BOB_UID, "funding_txo": format!("{TX_BOB}:0"),
            "previous": {"user_channel_id": ALICE_UID},
        }));
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(0), "closed")]);
    }

    #[test]
    fn a_funding_row_naming_another_counterparty_is_ignored() {
        let (_dir, db) = fresh_db();
        // Two channels whose user_channel_id collides, as an old LDK left inbound ones at 0.
        record(&db, "CHANNEL_CLOSED", 2_000, serde_json::json!({
            "channel_id": "a", "user_channel_id": "0", "counterparty_node_id": ALICE,
        }));
        opened(&db, BOB, "0", &format!("{TX_BOB}:0"), 1_000);
        assert!(collect_funding_outpoints(&db, &[], ALICE).unwrap().is_empty());
    }

    #[test]
    fn an_unnamed_row_of_another_channel_with_a_non_random_id_is_never_returned() {
        let (_dir, db) = fresh_db();
        // Bob's channel left only a row naming no counterparty, and an old LDK gave both channels id 0.
        record(&db, "CHANNEL_READY_TRACKED", 1_000, serde_json::json!({"user_channel_id": "0", "funding_txo": format!("{TX_BOB}:0")}));
        opened(&db, ALICE, "0", &format!("{TX_OPEN}:0"), 2_000);
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(0), "closed")]);
    }

    #[test]
    fn only_a_random_128_bit_user_channel_id_counts_as_unshared() {
        assert!(drawn_at_random(ALICE_UID));
        assert!(drawn_at_random(&u128::MAX.to_string()));
        for id in ["0", "7", &u64::MAX.to_string(), "", "abc", "-1"] {
            assert!(!drawn_at_random(id), "{id}");
        }
    }

    #[test]
    fn a_shared_user_channel_id_keeps_only_rows_that_name_this_node() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, "7", &format!("{TX_OPEN}:0"), 1_000);
        opened(&db, BOB, "7", &format!("{TX_BOB}:0"), 1_100);
        // A lock record names no counterparty, so it can't be told apart once the id is shared.
        splice_locked(&db, "7", &format!("{TX_SPLICE_2}:0"), 2_000);
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(0), "closed")]);
    }

    #[test]
    fn a_closed_row_of_another_node_marks_the_user_channel_id_shared() {
        let (_dir, db) = fresh_db();
        record(&db, "CHANNEL_READY_TRACKED", 1_000, serde_json::json!({"user_channel_id": "7", "funding_txo": format!("{TX_BOB}:0")}));
        record(&db, "CHANNEL_CLOSED", 1_500, serde_json::json!({"user_channel_id": "7", "counterparty_node_id": BOB}));
        opened(&db, ALICE, "7", &format!("{TX_OPEN}:0"), 2_000);
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(0), "closed")]);
    }

    #[test]
    fn a_live_channel_of_another_node_marks_the_user_channel_id_shared() {
        let (_dir, db) = fresh_db();
        splice_locked(&db, "7", &format!("{TX_SPLICE_2}:0"), 1_000);
        opened(&db, ALICE, "7", &format!("{TX_OPEN}:0"), 2_000);
        let found = collect_funding_outpoints(&db, &[live(BOB, "7", TX_BOB, 0)], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(0), "closed")]);
    }

    #[test]
    fn another_nodes_channel_is_never_returned() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        opened(&db, BOB, BOB_UID, &format!("{TX_BOB}:0"), 1_100);
        // A forward names both peers and both channels in one row.
        record(&db, "PAYMENT_FORWARDED", 2_000, serde_json::json!({
            "prev_node_id": ALICE, "next_node_id": BOB,
            "prev_user_channel_id": ALICE_UID, "next_user_channel_id": BOB_UID,
        }));
        let found = collect_funding_outpoints(&db, &[live(BOB, BOB_UID, TX_BOB, 0)], ALICE).unwrap();
        assert_eq!(found, [outpoint(0, TX_OPEN, Some(0), "closed")]);
    }

    #[test]
    fn a_channel_that_failed_to_open_is_left_out() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        record(&db, "CHANNEL_OPEN_FAILED", 2_000, serde_json::json!({
            "user_channel_id": ALICE_UID, "counterparty_node_id": ALICE,
        }));
        assert!(collect_funding_outpoints(&db, &[], ALICE).unwrap().is_empty());
    }

    #[test]
    fn open_channels_come_first_then_closed_ones_newest_first() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        opened(&db, ALICE, ALICE_UID_2, &format!("{TX_SPLICE_2}:0"), 9_000);
        // Live and never recorded in the ledger, e.g. opened before the ledger existed.
        let open = live(ALICE, "220608917660482373069445486228549857596", TX_OTHER, 3);
        let found = collect_funding_outpoints(&db, &[open], ALICE).unwrap();
        assert_eq!(found, [
            outpoint(0, TX_OTHER, Some(3), "open"),
            outpoint(1, TX_SPLICE_2, Some(0), "closed"),
            outpoint(2, TX_OPEN, Some(0), "closed"),
        ]);
    }

    #[test]
    fn outputs_of_one_channel_share_a_number_when_channels_interleave_in_time() {
        let (_dir, db) = fresh_db();
        // ALICE_UID sorts after ALICE_UID_2 but has the newest output, so it must be numbered 0.
        opened(&db, ALICE, ALICE_UID_2, &format!("{TX_OTHER}:0"), 1_000);
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 2_000);
        negotiated(&db, ALICE_UID_2, &format!("{TX_SPLICE_2}:1"), 3_000);
        negotiated(&db, ALICE_UID, &format!("{TX_SPLICE}:1"), 4_000);
        let found = collect_funding_outpoints(&db, &[], ALICE).unwrap();
        assert_eq!(found, [
            outpoint(0, TX_SPLICE, Some(1), "closed"),
            outpoint(1, TX_SPLICE_2, Some(1), "closed"),
            outpoint(0, TX_OPEN, Some(0), "closed"),
            outpoint(1, TX_OTHER, Some(0), "closed"),
        ]);
    }

    #[test]
    fn the_window_refuses_requests_past_its_budget_until_it_rolls_over() {
        let mut window = RequestWindow::default();
        for _ in 0..10 {
            assert!(window.allow(NOW));
        }
        assert!(!window.allow(NOW + 59), "11th request inside the same minute");
        assert!(window.allow(NOW + 60), "a new minute starts a new budget");
    }

    #[test]
    fn a_clock_stepped_back_does_not_lock_a_node_out() {
        let mut window = RequestWindow::default();
        for _ in 0..10 {
            assert!(window.allow(NOW));
        }
        assert!(window.allow(NOW - 1));
    }

    #[test]
    fn a_window_reports_a_spent_budget_without_counting() {
        let mut window = RequestWindow::default();
        for _ in 0..9 {
            assert!(window.allow(NOW));
        }
        assert!(!window.spent(NOW) && !window.spent(NOW), "checking uses no budget");
        assert!(window.allow(NOW));
        assert!(window.spent(NOW + 59));
        assert!(!window.spent(NOW + 60), "a new minute");
        assert!(!window.spent(NOW - 1), "a clock stepped back");
    }

    fn budget_keys(route: &RouteState) -> BTreeSet<String> {
        route.budgets.lock().unwrap().windows.keys().cloned().collect()
    }

    #[test]
    fn stale_budgets_are_dropped_at_most_once_a_window() {
        let route = route();
        let (carol, dave) = (wallet(0x55).1, wallet(0x66).1);
        assert!(route.allow(ALICE, NOW));
        assert!(route.allow(BOB, NOW + 30));
        // A window after the first cleanup, ALICE's window is dropped.
        assert!(route.allow(&carol, NOW + 61));
        assert_eq!(budget_keys(&route), BTreeSet::from([BOB.to_string(), carol.clone()]));
        // BOB's window ended at NOW + 90, but the next cleanup is not due before NOW + 121.
        assert!(route.allow(&dave, NOW + 95));
        assert_eq!(budget_keys(&route), BTreeSet::from([BOB.to_string(), carol.clone(), dave.clone()]));
        assert!(route.allow(ALICE, NOW + 121));
        assert_eq!(budget_keys(&route), BTreeSet::from([ALICE.to_string(), dave]));
    }

    #[test]
    fn funding_outpoints_parse_with_or_without_a_vout() {
        let upper = TX_OPEN.to_ascii_uppercase();
        assert_eq!(parse_funding(&format!("{TX_OPEN}:7")), Some((TX_OPEN.to_string(), Some(7))));
        assert_eq!(parse_funding(TX_OPEN), Some((TX_OPEN.to_string(), None)));
        assert_eq!(parse_funding(&format!("{upper}:0")), Some((TX_OPEN.to_string(), Some(0))));
        assert_eq!(parse_funding(&format!("{TX_OPEN}:x")), None);
        assert_eq!(parse_funding(&format!("{}:0", &TX_OPEN[..63])), None);
        assert_eq!(parse_funding(&format!("{}zz:0", &TX_OPEN[..62])), None);
    }

    #[test]
    fn each_channel_returns_its_ten_newest_outputs_and_none_is_crowded_out() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        for n in 1..=11u64 {
            negotiated(&db, ALICE_UID, &format!("{:064x}:1", 0xfeed_0000u64 + n), 1_000 + n as i64 * 100);
        }
        opened(&db, ALICE, ALICE_UID_2, &format!("{TX_SPLICE_2}:0"), 500);
        let busy = live(ALICE, ALICE_UID, &format!("{:064x}", 0xfeed_0000u64 + 11), 1);
        let found = collect_funding_outpoints(&db, &[busy], ALICE).unwrap();
        assert_eq!(found.len(), 11);
        assert_eq!(found[0], outpoint(0, &format!("{:064x}", 0xfeed_0000u64 + 11), Some(1), "open"));
        assert_eq!(found[9], outpoint(0, &format!("{:064x}", 0xfeed_0000u64 + 2), Some(1), "open"), "the two oldest are cut");
        assert_eq!(found[10], outpoint(1, TX_SPLICE_2, Some(0), "closed"), "the other channel is still listed");
    }

    #[test]
    fn the_twenty_newest_closed_channels_are_looked_up_and_failed_opens_do_not_count() {
        let (_dir, db) = fresh_db();
        let uid = |n: u64| (0xa11ce_u128 * 1_000_003 + u128::from(n) * 7_919).to_string();
        let txid = |n: u64| format!("{:064x}", 0xc0ff_ee00_u64 + n);
        // 21 closed channels; the oldest is past the cap.
        for n in 0..21 {
            opened(&db, ALICE, &uid(n), &format!("{}:0", txid(n)), 1_000 + n as i64);
        }
        // 30 failed opens newer than all of them.
        for n in 100..130 {
            record(&db, "CHANNEL_OPEN_FAILED", 5_000 + n as i64, serde_json::json!({
                "user_channel_id": uid(n), "counterparty_node_id": ALICE,
            }));
        }
        // A live channel older than all of them is always looked up.
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:1"), 10);
        let open = [live(ALICE, ALICE_UID, TX_OPEN, 1)];
        let channels = node_channels(&db, &open, ALICE, MAX_LINK_ROWS).unwrap();
        let expected: BTreeSet<String> = (1..21).map(uid).chain([ALICE_UID.to_string()]).collect();
        assert_eq!(channels.ids, expected);
        let found = funding_outpoints_of(&db, &channels).unwrap();
        assert_eq!(found.len(), 21);
        assert_eq!(found[0], outpoint(0, TX_OPEN, Some(1), "open"));
        assert_eq!(found[1], outpoint(1, &txid(20), Some(0), "closed"));
        assert_eq!(found[20], outpoint(20, &txid(1), Some(0), "closed"));
    }

    #[test]
    fn only_a_nodes_newest_channel_records_are_read() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        for n in 0..3u64 {
            record(&db, "CHANNEL_OPEN_FAILED", 2_000 + n as i64, serde_json::json!({
                "user_channel_id": (0xfa11_u128 * 1_000_003 + u128::from(n)).to_string(), "counterparty_node_id": ALICE,
            }));
        }
        opened(&db, ALICE, ALICE_UID_2, &format!("{TX_SPLICE_2}:1"), 9_000);
        // Recorded last, but another node's record uses none of Alice's limit.
        opened(&db, BOB, BOB_UID, &format!("{TX_BOB}:0"), 9_500);
        let ids = |cap| node_channels(&db, &[], ALICE, cap).unwrap().ids;
        assert_eq!(ids(4), BTreeSet::from([ALICE_UID_2.to_string()]), "Alice's own newer records push out only her older channel");
        assert_eq!(ids(5), BTreeSet::from([ALICE_UID.to_string(), ALICE_UID_2.to_string()]));
    }

    #[test]
    fn the_live_output_is_kept_past_the_cap_even_when_recorded_early() {
        let (_dir, db) = fresh_db();
        opened(&db, ALICE, ALICE_UID, &format!("{TX_OPEN}:0"), 1_000);
        // A splice that hasn't locked yet, with ten candidates; LDK Server still reports the pre-splice output.
        for n in 1..=10u64 {
            negotiated(&db, ALICE_UID, &format!("{:064x}:1", 0xbeef_0000u64 + n), 1_000 + n as i64 * 100);
        }
        let found = collect_funding_outpoints(&db, &[live(ALICE, ALICE_UID, TX_OPEN, 0)], ALICE).unwrap();
        assert_eq!(found.len(), 11);
        assert!(found.contains(&outpoint(0, TX_OPEN, Some(0), "open")));
        assert_eq!(found[0], outpoint(0, &format!("{:064x}", 0xbeef_0000u64 + 10), Some(1), "open"));
    }

    struct FakeLdk {
        hangs: bool,
        channels: Option<Vec<Channel>>,
        node_id: Option<String>,
        channel_reads: AtomicUsize,
        node_id_reads: AtomicUsize,
    }

    impl FakeLdk {
        fn new(channels: Vec<Channel>) -> Self {
            Self {
                hangs: false,
                channels: Some(channels),
                node_id: Some(LSP.to_string()),
                channel_reads: AtomicUsize::new(0),
                node_id_reads: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LdkServerCalls for FakeLdk {
        async fn list_channels(&self, _req: ListChannelsRequest) -> Result<ListChannelsResponse, LdkServerError> {
            self.channel_reads.fetch_add(1, Ordering::SeqCst);
            if self.hangs {
                std::future::pending::<()>().await;
            }
            match &self.channels {
                Some(channels) => Ok(ListChannelsResponse { channels: channels.clone() }),
                None => Err(LdkServerError::new(LdkServerErrorCode::InternalServerError, "down".to_string())),
            }
        }
        async fn spontaneous_send(&self, _req: SpontaneousSendRequest) -> Result<SpontaneousSendResponse, LdkServerError> {
            unreachable!("the route never pays")
        }
        async fn sign_message(&self, _req: SignMessageRequest) -> Result<SignMessageResponse, LdkServerError> {
            unreachable!("the route never signs")
        }
        async fn verify_signature(&self, _req: VerifySignatureRequest) -> Result<VerifySignatureResponse, LdkServerError> {
            unreachable!("the daemon checks signatures itself")
        }
        async fn get_node_info(&self, _req: GetNodeInfoRequest) -> Result<GetNodeInfoResponse, LdkServerError> {
            self.node_id_reads.fetch_add(1, Ordering::SeqCst);
            match &self.node_id {
                Some(node_id) => Ok(GetNodeInfoResponse { node_id: node_id.clone(), ..Default::default() }),
                None => Err(LdkServerError::new(LdkServerErrorCode::InternalServerError, "down".to_string())),
            }
        }
    }

    /// A wallet's node key and its node id.
    fn wallet(byte: u8) -> (SecretKey, String) {
        let secret = SecretKey::from_slice(&[byte; 32]).unwrap();
        (secret, PublicKey::from_secret_key(&Secp256k1::new(), &secret).to_string())
    }

    fn request(node_id: &str, timestamp: u64, signature: &str) -> Vec<u8> {
        serde_json::json!({"node_id": node_id, "timestamp": timestamp, "signature": signature}).to_string().into_bytes()
    }

    /// A request signed over the canonical bytes for `lsp`, with `node_id` exactly as given.
    fn signed(secret: &SecretKey, node_id: &str, lsp: &str, timestamp: u64) -> Vec<u8> {
        let message = crate::messages::funding_outpoints_signed_bytes(node_id, lsp, timestamp);
        request(node_id, timestamp, &message_signing::sign(&message, secret))
    }

    fn junk(node_id: &str) -> Vec<u8> {
        request(node_id, NOW, "d9o8ftxzs3ggfpzwfkqbjzb7rsh1bs5wb5oy9ji8ayahbqzr6rksi4ff4jcb94mgfujbhp3e6z86qucb9mt1iw4cdw7aqyw4bksfb5r")
    }

    fn route() -> Arc<RouteState> {
        let route = Arc::new(RouteState::default());
        route.remember_lsp_node_id(LSP);
        route
    }

    /// Holds the next ledger read at its start: `reached` fires when it gets there, and a send on `open` lets it go on.
    pub(super) struct ReadGate {
        pub(super) reached: tokio::sync::oneshot::Sender<()>,
        pub(super) open: std::sync::mpsc::Receiver<()>,
    }

    fn hold_next_read(route: &RouteState) -> (tokio::sync::oneshot::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (reached, reached_rx) = tokio::sync::oneshot::channel();
        let (open_tx, open) = std::sync::mpsc::channel();
        *route.read_gate.lock().unwrap() = Some(ReadGate { reached, open });
        (reached_rx, open_tx)
    }

    #[tokio::test]
    async fn a_ledger_read_waiting_on_the_database_does_not_hold_the_runtime() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x55);
        opened(&db, &node, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        let (reached, open) = hold_next_read(&route);
        let body = signed(&secret, &node, LSP, NOW);
        let started = std::time::Instant::now();
        // While the read is held, a timer on this single-threaded runtime still fires, then lets the read go on.
        let other_work = async {
            tokio::time::timeout(Duration::from_secs(5), reached).await.expect("the read was never reached").unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            open.send(()).unwrap();
        };
        let (reply, ()) = tokio::join!(answer(&ldk, &db, &route, &body, NOW), other_work);
        assert_eq!(reply.0, StatusCode::OK);
        assert!(started.elapsed() < Duration::from_secs(2), "the read held the runtime for {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_cancelled_request_keeps_its_node_lookup_until_the_ledger_read_ends() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x66);
        opened(&db, &node, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        let (reached, open) = hold_next_read(&route);
        let body = signed(&secret, &node, LSP, NOW);
        let request = tokio::spawn({
            let (ldk, db, route, body) = (Arc::clone(&ldk), db.clone(), Arc::clone(&route), body.clone());
            async move { answer(&ldk, &db, &route, &body, NOW).await }
        });
        tokio::time::timeout(Duration::from_secs(5), reached).await.expect("the read was never reached").unwrap();
        // The client goes away while its read is running.
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        let copy = answer(&ldk, &db, &route, &body, NOW).await;
        assert_eq!((copy.0, copy.1["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("lookup already running")));
        open.send(()).unwrap();
        // Freed once the read ends.
        for _ in 0..200 {
            if route.lookups.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(answer(&ldk, &db, &route, &body, NOW).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_request_signed_over_the_documented_bytes_gets_its_outpoints() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x11);
        opened(&db, &node, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        let ts = NOW - 299;
        // The bytes as the PR documents them for app authors, written out by hand.
        let message = format!(r#"{{"type":"FUNDING_OUTPOINTS_V1","node_id":"{node}","lsp_node_id":"{LSP}","ts":{ts}}}"#);
        let body = request(&node, ts, &message_signing::sign(message.as_bytes(), &secret));
        let (status, reply) = answer(&Arc::new(FakeLdk::new(vec![])), &db, &route(), &body, NOW).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply, serde_json::json!({"funding_outpoints": [
            {"channel": 0, "funding_txid": TX_OPEN, "funding_vout": 1, "state": "closed"},
        ]}));
    }

    #[tokio::test]
    async fn a_signature_by_another_key_or_for_another_lsp_is_refused() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x11);
        let (other_secret, _) = wallet(0x22);
        opened(&db, &node, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        let ldk = Arc::new(FakeLdk::new(vec![]));
        let wrong_key = signed(&other_secret, &node, LSP, NOW);
        let (status, reply) = answer(&ldk, &db, &route(), &wrong_key, NOW).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(reply.get("funding_outpoints").is_none());
        let other_lsp = signed(&secret, &node, BOB, NOW);
        assert_eq!(answer(&ldk, &db, &route(), &other_lsp, NOW).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(ldk.channel_reads.load(Ordering::SeqCst), 0, "nothing is read for an unsigned request");
    }

    #[tokio::test]
    async fn the_node_id_is_signed_exactly_as_sent_and_looked_up_in_lowercase() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x11);
        opened(&db, &node, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        let upper = node.to_ascii_uppercase();
        let signed_lowercase = request(&upper, NOW, &message_signing::sign(&crate::messages::funding_outpoints_signed_bytes(&node, LSP, NOW), &secret));
        assert_eq!(answer(&Arc::new(FakeLdk::new(vec![])), &db, &route(), &signed_lowercase, NOW).await.0, StatusCode::UNAUTHORIZED);
        let (status, reply) = answer(&Arc::new(FakeLdk::new(vec![])), &db, &route(), &signed(&secret, &upper, LSP, NOW), NOW).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["funding_outpoints"][0]["funding_txid"], TX_OPEN);
    }

    #[tokio::test]
    async fn a_stale_timestamp_is_refused_before_any_ldk_server_call() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x11);
        let ldk = Arc::new(FakeLdk::new(vec![]));
        let empty = Arc::new(RouteState::default());
        assert_eq!(answer(&ldk, &db, &empty, &signed(&secret, &node, LSP, NOW - 301), NOW).await.0, StatusCode::UNAUTHORIZED);
        let ahead = signed(&secret, &node, LSP, NOW + 301);
        assert_eq!(answer(&ldk, &db, &empty, &ahead, NOW).await.0, StatusCode::UNAUTHORIZED, "a timestamp too far ahead too");
        assert_eq!(ldk.node_id_reads.load(Ordering::SeqCst) + ldk.channel_reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn malformed_requests_are_refused_before_any_ldk_server_call() {
        let (_dir, db) = fresh_db();
        let ldk = Arc::new(FakeLdk::new(vec![]));
        let empty = Arc::new(RouteState::default());
        let off_curve = format!("02{}", "ff".repeat(32));
        let too_long = request(&wallet(0x11).1, NOW, &"y".repeat(151));
        assert_eq!(answer(&ldk, &db, &empty, b"{\"node_id\":", NOW).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(answer(&ldk, &db, &empty, &junk(&ALICE[..64]), NOW).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(answer(&ldk, &db, &empty, &junk(&format!("04{}", &ALICE[2..])), NOW).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(answer(&ldk, &db, &empty, &junk(&off_curve), NOW).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(answer(&ldk, &db, &empty, &too_long, NOW).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(ldk.node_id_reads.load(Ordering::SeqCst) + ldk.channel_reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn bad_signatures_cannot_use_up_a_wallets_budget() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x11);
        opened(&db, &node, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        for _ in 0..200 {
            assert_eq!(answer(&ldk, &db, &route, &junk(&node), NOW).await.0, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_node_without_channels_gets_an_empty_list_and_is_never_limited() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x33);
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        for _ in 0..30 {
            let (status, reply) = answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await;
            assert_eq!((status, reply), (StatusCode::OK, serde_json::json!({"funding_outpoints": []})));
        }
    }

    #[tokio::test]
    async fn a_node_with_channels_is_limited_on_its_own() {
        let (_dir, db) = fresh_db();
        let (alice_secret, alice) = wallet(0x11);
        let (bob_secret, bob) = wallet(0x22);
        opened(&db, &alice, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        opened(&db, &bob, BOB_UID, &format!("{TX_BOB}:0"), 1_000);
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        for _ in 0..10 {
            assert_eq!(answer(&ldk, &db, &route, &signed(&alice_secret, &alice, LSP, NOW), NOW).await.0, StatusCode::OK);
        }
        assert_eq!(answer(&ldk, &db, &route, &signed(&alice_secret, &alice, LSP, NOW), NOW).await.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(answer(&ldk, &db, &route, &signed(&bob_secret, &bob, LSP, NOW), NOW).await.0, StatusCode::OK, "another node keeps its budget");
        let later = NOW + 60;
        assert_eq!(answer(&ldk, &db, &route, &signed(&alice_secret, &alice, LSP, later), later).await.0, StatusCode::OK);
    }

    #[tokio::test(start_paused = true)]
    async fn a_node_past_its_budget_is_refused_before_the_channel_list_is_read() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x11);
        opened(&db, &node, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        for _ in 0..10 {
            assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::OK);
        }
        tokio::time::advance(CHANNEL_CACHE).await;
        assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(ldk.channel_reads.load(Ordering::SeqCst), 1, "the expired cache was not refreshed for it");
    }

    #[tokio::test]
    async fn a_node_whose_opens_all_failed_gets_nothing_and_still_has_a_budget() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x44);
        record(&db, "CHANNEL_OPEN_FAILED", 1_000, serde_json::json!({
            "user_channel_id": ALICE_UID, "counterparty_node_id": node,
        }));
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        for _ in 0..10 {
            let (status, reply) = answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await;
            assert_eq!((status, reply), (StatusCode::OK, serde_json::json!({"funding_outpoints": []})));
        }
        assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn a_node_past_its_budget_is_refused_before_any_lookup() {
        let (dir, db) = fresh_db();
        let (secret, node) = wallet(0x11);
        opened(&db, &node, ALICE_UID, &format!("{TX_OPEN}:1"), 1_000);
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        for _ in 0..10 {
            assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::OK);
        }
        // Any ledger lookup now fails with a 503.
        rusqlite::Connection::open(dir.path().join(stable_channels::db::DB_FILENAME))
            .unwrap()
            .execute_batch("DROP TABLE ledger_event_refs")
            .unwrap();
        assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(answer(&ldk, &db, &route, &junk(&node), NOW).await.0, StatusCode::UNAUTHORIZED, "an unsigned request learns nothing of the budget");
    }

    #[tokio::test(start_paused = true)]
    async fn the_channel_list_is_read_at_most_once_per_cache_window() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x33);
        let (ldk, route) = (Arc::new(FakeLdk::new(vec![])), route());
        for _ in 0..3 {
            answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await;
        }
        tokio::time::advance(Duration::from_secs(4)).await;
        answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW + 4), NOW + 4).await;
        assert_eq!(ldk.channel_reads.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW + 5), NOW + 5).await;
        assert_eq!(ldk.channel_reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_channel_list_answers_503_and_is_not_retried_within_the_window() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x33);
        let (ldk, route) = (Arc::new(FakeLdk { channels: None, ..FakeLdk::new(vec![]) }), route());
        for _ in 0..2 {
            assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::SERVICE_UNAVAILABLE);
        }
        assert_eq!(ldk.channel_reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_channel_list_answers_503_after_a_timeout() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x33);
        let (ldk, route) = (Arc::new(FakeLdk { hangs: true, ..FakeLdk::new(vec![]) }), route());
        assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_queued_behind_a_hung_read_reuses_its_result() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x33);
        let (other_secret, other) = wallet(0x44);
        let (ldk, route) = (Arc::new(FakeLdk { hangs: true, ..FakeLdk::new(vec![]) }), route());
        let (first_body, queued_body) = (signed(&secret, &node, LSP, NOW), signed(&other_secret, &other, LSP, NOW + 6));
        let first = answer(&ldk, &db, &route, &first_body, NOW);
        // Another node's request arrives 6 s in and waits until the first read times out at 10 s.
        let queued = async {
            tokio::time::sleep(Duration::from_secs(6)).await;
            answer(&ldk, &db, &route, &queued_body, NOW + 6).await
        };
        let (first, queued) = tokio::join!(first, queued);
        assert_eq!((first.0, queued.0), (StatusCode::SERVICE_UNAVAILABLE, StatusCode::SERVICE_UNAVAILABLE));
        assert_eq!(ldk.channel_reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_node_has_one_lookup_running_at_a_time() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x11);
        let (other_secret, other) = wallet(0x22);
        let (ldk, route) = (Arc::new(FakeLdk { hangs: true, ..FakeLdk::new(vec![]) }), route());
        let (body, other_body) = (signed(&secret, &node, LSP, NOW), signed(&other_secret, &other, LSP, NOW));
        // A copy of the request sent 1 s into the first one's read, and another node's request.
        let later = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tokio::join!(answer(&ldk, &db, &route, &body, NOW), answer(&ldk, &db, &route, &other_body, NOW))
        };
        let (first, (copy, other_node)) = tokio::join!(answer(&ldk, &db, &route, &body, NOW), later);
        assert_eq!(copy.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!((first.0, other_node.0), (StatusCode::SERVICE_UNAVAILABLE, StatusCode::SERVICE_UNAVAILABLE));
        // The node is free again once its lookup ends, or once its client gives up.
        tokio::time::advance(CHANNEL_CACHE).await;
        assert!(tokio::time::timeout(Duration::from_secs(1), answer(&ldk, &db, &route, &body, NOW)).await.is_err());
        assert_eq!(answer(&ldk, &db, &route, &body, NOW).await.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test(start_paused = true)]
    async fn a_read_whose_request_gave_up_still_fills_the_cache() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x33);
        let (ldk, route) = (Arc::new(FakeLdk { hangs: true, ..FakeLdk::new(vec![]) }), route());
        let body = signed(&secret, &node, LSP, NOW);
        // The client disconnects 1 s into the read.
        assert!(tokio::time::timeout(Duration::from_secs(1), answer(&ldk, &db, &route, &body, NOW)).await.is_err());
        assert_eq!(answer(&ldk, &db, &route, &body, NOW + 1).await.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(ldk.channel_reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_lsp_node_id_not_known_yet_is_read_from_ldk_server() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x33);
        let ldk = Arc::new(FakeLdk { node_id: Some(LSP.to_ascii_uppercase()), ..FakeLdk::new(vec![]) });
        let route = Arc::new(RouteState::default());
        assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW), NOW).await.0, StatusCode::OK);
        assert_eq!(route.lsp_node_id.get().unwrap(), LSP);
    }

    #[tokio::test]
    async fn an_unknown_lsp_node_id_is_looked_up_at_most_every_ten_seconds() {
        let (_dir, db) = fresh_db();
        let (secret, node) = wallet(0x33);
        let ldk = Arc::new(FakeLdk { node_id: None, ..FakeLdk::new(vec![]) });
        let route = Arc::new(RouteState::default());
        for at in [NOW, NOW + 9] {
            assert_eq!(answer(&ldk, &db, &route, &signed(&secret, &node, LSP, at), at).await.0, StatusCode::SERVICE_UNAVAILABLE);
        }
        assert_eq!(ldk.node_id_reads.load(Ordering::SeqCst), 1);
        answer(&ldk, &db, &route, &signed(&secret, &node, LSP, NOW + 10), NOW + 10).await;
        assert_eq!(ldk.node_id_reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_lsp_node_id_is_kept_lowercase_and_an_empty_one_is_ignored() {
        let route = Arc::new(RouteState::default());
        route.remember_lsp_node_id("");
        assert!(route.lsp_node_id.get().is_none());
        route.remember_lsp_node_id(&LSP.to_ascii_uppercase());
        assert_eq!(route.lsp_node_id.get().unwrap(), LSP);
    }

    #[test]
    fn a_rate_limited_reply_says_when_to_retry() {
        let limited = json_reply(StatusCode::TOO_MANY_REQUESTS, &serde_json::json!({"error": "rate limited"}));
        assert_eq!(limited.headers()["retry-after"], "60");
        let served = json_reply(StatusCode::OK, &serde_json::json!({"funding_outpoints": []}));
        assert!(served.headers().get("retry-after").is_none());
        assert_eq!(served.headers()["content-type"], "application/json");
    }
}
