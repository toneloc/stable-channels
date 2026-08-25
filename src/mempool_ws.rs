//! Real-time on-chain transaction tracking over the mempool.space websocket.
//!
//! Port of the iOS/Android mempool-websocket services (`Services/WebSocket/`,
//! `services/websocket/`): the wallet tracks its receive addresses and gets a
//! [`WsEvent::Receive`] at mempool-sighting time and a [`WsEvent::Removed`] on RBF/eviction,
//! instead of waiting for the next BDK sync. Two deliberate divergences from the mobile ports:
//!
//! - No tracked-txid/outspend branch: desktop resolves channel-close txids from ldk-node's
//!   `pending_balances_from_channel_closures`, so funding-outpoint tracking is redundant.
//! - Reconnect backoff actually grows (1s → 60s) across consecutive failures. The mobile
//!   ports reset the attempt counter inside `connect()`, which pins their backoff at ~1s.
//!
//! Everything here is synchronous std + tungstenite, matching the desktop app's thread model.

use crate::audit::audit_event;
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const ENDPOINT: &str = "wss://mempool.space/api/v1/ws";
const READ_TIMEOUT: Duration = Duration::from_secs(1);
const PING_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BACKOFF_SECS: u64 = 60;
const STABLE_CONNECTION_RESET: Duration = Duration::from_secs(30);
/// Ignore receives below this — fee-estimation noise, parity with the mobile ports.
const MIN_RECEIVE_SATS: u64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsEvent {
    /// A transaction paying a tracked address was seen (mempool or newly confirmed).
    Receive {
        address: String,
        txid: String,
        amount_sats: u64,
    },
    /// A previously seen transaction was replaced or evicted from the mempool.
    Removed { address: String, txid: String },
}

enum Cmd {
    Track(String),
    Untrack(String),
    Shutdown,
}

/// Handle to the websocket thread. Dropping it does not stop the thread; call
/// [`MempoolWs::shutdown`] for a clean stop (process exit also suffices).
pub struct MempoolWs {
    cmd_tx: Sender<Cmd>,
}

impl MempoolWs {
    /// Spawn the websocket thread. It stays idle (no connection) until the first
    /// tracked address arrives, and reconnects with growing backoff on failures.
    pub fn start(event_tx: Sender<WsEvent>) -> Self {
        let (cmd_tx, cmd_rx) = channel();
        std::thread::spawn(move || run(cmd_rx, event_tx));
        MempoolWs { cmd_tx }
    }

    pub fn track_address(&self, address: &str) {
        let normalized = address.trim();
        if !normalized.is_empty() {
            let _ = self.cmd_tx.send(Cmd::Track(normalized.to_string()));
        }
    }

    pub fn untrack_address(&self, address: &str) {
        let normalized = address.trim();
        if !normalized.is_empty() {
            let _ = self.cmd_tx.send(Cmd::Untrack(normalized.to_string()));
        }
    }

    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(Cmd::Shutdown);
    }
}

// ---------------------------------------------------------------------------
// Wire format (subset of mempool.space's websocket schema that we consume)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WsVout {
    #[serde(rename = "scriptpubkey_address")]
    scriptpubkey_address: Option<String>,
    value: Option<u64>,
}

#[derive(Deserialize)]
struct WsTx {
    txid: String,
    vout: Option<Vec<WsVout>>,
}

#[derive(Deserialize)]
struct WsAddressTxs {
    mempool: Option<Vec<WsTx>>,
    confirmed: Option<Vec<WsTx>>,
    removed: Option<Vec<WsTx>>,
}

#[derive(Deserialize)]
struct WsMessage {
    address: Option<String>,
    #[serde(rename = "address-transactions")]
    address_transactions: Option<Vec<WsTx>>,
    #[serde(rename = "block-transactions")]
    block_transactions: Option<Vec<WsTx>>,
    #[serde(rename = "multi-address-transactions")]
    multi_address_transactions: Option<HashMap<String, WsAddressTxs>>,
}

fn is_valid_txid(txid: &str) -> bool {
    txid.len() == 64 && txid.bytes().all(|b| b.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------
// Dedup store (port of ProcessedTxStore: 15 min TTL, 500 entries, evict oldest fifth)
// ---------------------------------------------------------------------------

pub struct ProcessedTxStore {
    entries: HashMap<String, Instant>,
    ttl: Duration,
    max_entries: usize,
    purge_interval: Duration,
    last_purge: Instant,
}

impl ProcessedTxStore {
    pub fn new() -> Self {
        Self::with_limits(Duration::from_secs(900), 500)
    }

    pub fn with_limits(ttl: Duration, max_entries: usize) -> Self {
        ProcessedTxStore {
            entries: HashMap::new(),
            ttl,
            max_entries,
            purge_interval: Duration::from_secs(300),
            last_purge: Instant::now(),
        }
    }

    pub fn is_recently_processed(&self, key: &str) -> bool {
        self.entries
            .get(key)
            .map(|seen| seen.elapsed() < self.ttl)
            .unwrap_or(false)
    }

    pub fn record(&mut self, key: String) {
        self.entries.insert(key, Instant::now());
        if self.entries.len() > self.max_entries {
            let evict = (self.max_entries / 5).max(1);
            let mut by_age: Vec<(String, Instant)> = self
                .entries
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            by_age.sort_by_key(|(_, at)| *at);
            for (key, _) in by_age.into_iter().take(evict) {
                self.entries.remove(&key);
            }
        }
        if self.last_purge.elapsed() >= self.purge_interval {
            let ttl = self.ttl;
            self.entries.retain(|_, seen| seen.elapsed() < ttl);
            self.last_purge = Instant::now();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ProcessedTxStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Message handling (pure — unit tested below)
// ---------------------------------------------------------------------------

fn handle_message(
    text: &str,
    tracked: &HashSet<String>,
    dedup: &mut ProcessedTxStore,
) -> Vec<WsEvent> {
    let Ok(msg) = serde_json::from_str::<WsMessage>(text) else {
        return Vec::new();
    };
    let mut events = Vec::new();

    let mut txs: Vec<&WsTx> = Vec::new();
    if let Some(list) = &msg.address_transactions {
        txs.extend(list.iter());
    }
    if let Some(list) = &msg.block_transactions {
        txs.extend(list.iter());
    }
    if let Some(groups) = &msg.multi_address_transactions {
        for group in groups.values() {
            for list in [&group.mempool, &group.confirmed].into_iter().flatten() {
                txs.extend(list.iter());
            }
        }
    }

    for tx in txs {
        if !is_valid_txid(&tx.txid) {
            continue;
        }
        let mut targets: Vec<String> = Vec::new();
        if let Some(addr) = &msg.address {
            if tracked.contains(addr) {
                targets.push(addr.clone());
            }
        }
        for vout in tx.vout.iter().flatten() {
            if let Some(addr) = &vout.scriptpubkey_address {
                if tracked.contains(addr) && !targets.iter().any(|t| t == addr) {
                    targets.push(addr.clone());
                }
            }
        }
        if let Some(groups) = &msg.multi_address_transactions {
            for (addr, group) in groups {
                if !tracked.contains(addr) || targets.iter().any(|t| t == addr) {
                    continue;
                }
                let listed = [&group.mempool, &group.confirmed, &group.removed]
                    .into_iter()
                    .flatten()
                    .any(|list| list.iter().any(|t| t.txid == tx.txid));
                if listed {
                    targets.push(addr.clone());
                }
            }
        }

        for target in targets {
            let key = format!("{}_{}", tx.txid, target);
            if dedup.is_recently_processed(&key) {
                continue;
            }
            dedup.record(key);
            let amount_sats: u64 = tx
                .vout
                .iter()
                .flatten()
                .filter(|v| v.scriptpubkey_address.as_deref() == Some(target.as_str()))
                .map(|v| v.value.unwrap_or(0))
                .sum();
            if amount_sats < MIN_RECEIVE_SATS {
                continue;
            }
            events.push(WsEvent::Receive {
                address: target,
                txid: tx.txid.clone(),
                amount_sats,
            });
        }
    }

    // Removed transactions are a separate pass with a distinct dedup namespace, so an
    // RBF replacement of an already-seen txid still fires (mobile-port parity).
    if let Some(groups) = &msg.multi_address_transactions {
        for (addr, group) in groups {
            if !tracked.contains(addr) {
                continue;
            }
            for tx in group.removed.iter().flatten() {
                if !is_valid_txid(&tx.txid) {
                    continue;
                }
                let key = format!("removed_{}_{}", tx.txid, addr);
                if dedup.is_recently_processed(&key) {
                    continue;
                }
                dedup.record(key);
                events.push(WsEvent::Removed {
                    address: addr.clone(),
                    txid: tx.txid.clone(),
                });
            }
        }
    }

    events
}

// ---------------------------------------------------------------------------
// Connection thread
// ---------------------------------------------------------------------------

type WsSocket = WebSocket<MaybeTlsStream<TcpStream>>;

fn send_tracking(socket: &mut WsSocket, tracked: &HashSet<String>) -> Result<(), String> {
    // Always send, even empty — an empty list clears server-side subscriptions after untrack.
    let list: Vec<&String> = tracked.iter().collect();
    socket
        .send(Message::Text(json!({ "track-addresses": list }).to_string()))
        .map_err(|error| error.to_string())
}

/// Apply a command to the tracked set. Returns false when the thread should exit.
fn apply_cmd(cmd: Cmd, tracked: &mut HashSet<String>) -> (bool, bool) {
    match cmd {
        Cmd::Track(addr) => (tracked.insert(addr), true),
        Cmd::Untrack(addr) => (tracked.remove(&addr), true),
        Cmd::Shutdown => (false, false),
    }
}

fn reconnect_delay(failed_attempts: u32) -> Duration {
    if failed_attempts == 0 {
        return Duration::ZERO;
    }
    let seconds = (1u64 << (failed_attempts - 1).min(6)).min(MAX_BACKOFF_SECS);
    Duration::from_secs(seconds)
}

fn run(cmd_rx: Receiver<Cmd>, event_tx: Sender<WsEvent>) {
    let mut tracked: HashSet<String> = HashSet::new();
    let mut dedup = ProcessedTxStore::new();
    let mut failed_attempts: u32 = 0;

    'reconnect: loop {
        // Idle until there is something to track.
        while tracked.is_empty() {
            match cmd_rx.recv() {
                Ok(cmd) => {
                    let (_, keep_running) = apply_cmd(cmd, &mut tracked);
                    if !keep_running {
                        return;
                    }
                }
                Err(_) => return,
            }
        }

        // Exponential backoff before retry attempts, drained in small steps so shutdown
        // and tracking changes stay responsive.
        if failed_attempts > 0 {
            let deadline = Instant::now() + reconnect_delay(failed_attempts);
            while Instant::now() < deadline {
                match cmd_rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(cmd) => {
                        let (_, keep_running) = apply_cmd(cmd, &mut tracked);
                        if !keep_running {
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            if tracked.is_empty() {
                continue 'reconnect;
            }
        }

        let mut socket = match tungstenite::connect(ENDPOINT) {
            Ok((socket, _response)) => socket,
            Err(e) => {
                failed_attempts = failed_attempts.saturating_add(1);
                audit_event(
                    "WEBSOCKET_CONNECT_FAILED",
                    json!({ "error": format!("{e}"), "attempts": failed_attempts }),
                );
                continue 'reconnect;
            }
        };
        match socket.get_mut() {
            MaybeTlsStream::Plain(stream) => {
                let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
            }
            MaybeTlsStream::NativeTls(stream) => {
                let _ = stream.get_ref().set_read_timeout(Some(READ_TIMEOUT));
            }
            _ => {}
        }

        if let Err(e) = send_tracking(&mut socket, &tracked) {
            failed_attempts = failed_attempts.saturating_add(1);
            audit_event(
                "WEBSOCKET_TRACKING_FAILED",
                json!({ "error": format!("{e}"), "attempts": failed_attempts }),
            );
            continue 'reconnect;
        }
        audit_event(
            "WEBSOCKET_CONNECTED",
            json!({ "tracked_addresses": tracked.len() }),
        );

        let mut last_ping = Instant::now();
        let connected_at = Instant::now();
        let mut stable_connection = false;
        loop {
            // A successful TLS handshake is not enough to reset the retry counter: endpoints and
            // captive portals can accept and immediately close. Only a connection that survives
            // this window earns a reset, preventing a reconnect/audit-log storm.
            if !stable_connection && connected_at.elapsed() >= STABLE_CONNECTION_RESET {
                failed_attempts = 0;
                stable_connection = true;
            }

            // Drain pending commands; re-sync tracking on any change.
            loop {
                match cmd_rx.try_recv() {
                    Ok(cmd) => {
                        let (changed, keep_running) = apply_cmd(cmd, &mut tracked);
                        if !keep_running {
                            let _ = socket.close(None);
                            return;
                        }
                        if changed {
                            if let Err(e) = send_tracking(&mut socket, &tracked) {
                                failed_attempts = failed_attempts.saturating_add(1);
                                audit_event(
                                    "WEBSOCKET_TRACKING_FAILED",
                                    json!({
                                        "error": format!("{e}"),
                                        "attempts": failed_attempts,
                                    }),
                                );
                                continue 'reconnect;
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        let _ = socket.close(None);
                        return;
                    }
                }
            }

            if last_ping.elapsed() >= PING_INTERVAL {
                if socket.send(Message::Ping(Vec::new())).is_err() {
                    break;
                }
                last_ping = Instant::now();
            }

            match socket.read() {
                Ok(Message::Text(text)) => {
                    for event in handle_message(&text, &tracked, &mut dedup) {
                        if event_tx.send(event).is_err() {
                            let _ = socket.close(None);
                            return;
                        }
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
                Err(e) => {
                    audit_event("WEBSOCKET_DISCONNECTED", json!({ "error": format!("{e}") }));
                    break;
                }
            }
        }
        failed_attempts = failed_attempts.saturating_add(1);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const TXID_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TXID_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn tracked(addrs: &[&str]) -> HashSet<String> {
        addrs.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn reconnect_delay_grows_and_caps() {
        assert_eq!(reconnect_delay(0), Duration::ZERO);
        assert_eq!(reconnect_delay(1), Duration::from_secs(1));
        assert_eq!(reconnect_delay(2), Duration::from_secs(2));
        assert_eq!(reconnect_delay(7), Duration::from_secs(60));
        assert_eq!(reconnect_delay(u32::MAX), Duration::from_secs(60));
    }

    #[test]
    fn receive_sums_all_vouts_to_the_tracked_address() {
        let text = format!(
            r#"{{"address-transactions":[{{"txid":"{TXID_A}","vout":[
                {{"scriptpubkey_address":"bc1qme","value":60000}},
                {{"scriptpubkey_address":"bc1qother","value":5000}},
                {{"scriptpubkey_address":"bc1qme","value":40000}}
            ]}}]}}"#
        );
        let mut dedup = ProcessedTxStore::new();
        let events = handle_message(&text, &tracked(&["bc1qme"]), &mut dedup);
        assert_eq!(
            events,
            vec![WsEvent::Receive {
                address: "bc1qme".into(),
                txid: TXID_A.into(),
                amount_sats: 100_000
            }]
        );
    }

    #[test]
    fn confirmed_copy_of_seen_mempool_tx_is_deduped() {
        let mempool = format!(
            r#"{{"multi-address-transactions":{{"bc1qme":{{"mempool":[{{"txid":"{TXID_A}","vout":[{{"scriptpubkey_address":"bc1qme","value":50000}}]}}]}}}}}}"#
        );
        let confirmed = format!(
            r#"{{"multi-address-transactions":{{"bc1qme":{{"confirmed":[{{"txid":"{TXID_A}","vout":[{{"scriptpubkey_address":"bc1qme","value":50000}}]}}]}}}}}}"#
        );
        let mut dedup = ProcessedTxStore::new();
        let t = tracked(&["bc1qme"]);
        assert_eq!(handle_message(&mempool, &t, &mut dedup).len(), 1);
        assert!(handle_message(&confirmed, &t, &mut dedup).is_empty());
    }

    #[test]
    fn removed_fires_even_after_receive_was_deduped_and_is_not_a_receive() {
        let mempool = format!(
            r#"{{"multi-address-transactions":{{"bc1qme":{{"mempool":[{{"txid":"{TXID_A}","vout":[{{"scriptpubkey_address":"bc1qme","value":50000}}]}}]}}}}}}"#
        );
        let removed = format!(
            r#"{{"multi-address-transactions":{{"bc1qme":{{"removed":[{{"txid":"{TXID_A}","vout":[{{"scriptpubkey_address":"bc1qme","value":50000}}]}}]}}}}}}"#
        );
        let mut dedup = ProcessedTxStore::new();
        let t = tracked(&["bc1qme"]);
        assert_eq!(handle_message(&mempool, &t, &mut dedup).len(), 1);
        let events = handle_message(&removed, &t, &mut dedup);
        assert_eq!(
            events,
            vec![WsEvent::Removed {
                address: "bc1qme".into(),
                txid: TXID_A.into()
            }],
            "a removal must produce exactly one Removed and no Receive",
        );
    }

    #[test]
    fn untracked_addresses_dust_and_invalid_txids_are_ignored() {
        let untracked = format!(
            r#"{{"address-transactions":[{{"txid":"{TXID_A}","vout":[{{"scriptpubkey_address":"bc1qother","value":50000}}]}}]}}"#
        );
        let dust = format!(
            r#"{{"address-transactions":[{{"txid":"{TXID_B}","vout":[{{"scriptpubkey_address":"bc1qme","value":999}}]}}]}}"#
        );
        let bad_txid = r#"{"address-transactions":[{"txid":"nothex","vout":[{"scriptpubkey_address":"bc1qme","value":50000}]}]}"#;
        let mut dedup = ProcessedTxStore::new();
        let t = tracked(&["bc1qme"]);
        assert!(handle_message(&untracked, &t, &mut dedup).is_empty());
        assert!(handle_message(&dust, &t, &mut dedup).is_empty());
        assert!(handle_message(bad_txid, &t, &mut dedup).is_empty());
    }

    #[test]
    fn dedup_store_expires_and_caps() {
        let mut store = ProcessedTxStore::with_limits(Duration::from_millis(0), 10);
        store.record("k".into());
        assert!(
            !store.is_recently_processed("k"),
            "zero TTL entries expire immediately"
        );

        let mut store = ProcessedTxStore::with_limits(Duration::from_secs(900), 10);
        for i in 0..12 {
            store.record(format!("k{i}"));
        }
        assert!(store.len() <= 11, "cap eviction keeps the store bounded");
        assert!(store.is_recently_processed("k11"), "newest entries survive");
    }

    #[test]
    fn top_level_address_field_matches_without_vout_entry_but_yields_no_dust_receive() {
        // A message naming a tracked address whose tx pays it nothing sums to 0 and is
        // dropped by the dust gate (mobile-port parity: the dedup key is still burned).
        let text = format!(
            r#"{{"address":"bc1qme","address-transactions":[{{"txid":"{TXID_A}","vout":[{{"scriptpubkey_address":"bc1qother","value":50000}}]}}]}}"#
        );
        let mut dedup = ProcessedTxStore::new();
        assert!(handle_message(&text, &tracked(&["bc1qme"]), &mut dedup).is_empty());
        assert!(dedup.is_recently_processed(&format!("{TXID_A}_bc1qme")));
    }
}
