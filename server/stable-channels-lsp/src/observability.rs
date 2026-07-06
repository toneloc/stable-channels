//! Periodic observability poll: synthesizes SWEEP_PROGRESS and PEER_CONNECTED/PEER_DISCONNECTED audit events.

use std::collections::HashMap;
use std::time::Duration;

use ldk_server_client::ldk_server_grpc::api::GetBalancesRequest;
use ldk_server_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;
use tokio::time::interval;
use tracing::warn;

use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

const POLL_SECS: u64 = 30;

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    let mut tick = interval(Duration::from_secs(POLL_SECS));
    let mut sweep_prev: HashMap<String, String> = HashMap::new();
    let mut peer_prev: HashMap<String, bool> = HashMap::new();
    let mut peer_first_run = true;
    loop {
        tick.tick().await;
        poll_sweeps(&state, &mut sweep_prev).await;
        poll_peers(&state, &mut peer_prev, &mut peer_first_run).await;
    }
}

/// Pure: which channels changed sweep-state (or left the pending set → "Swept").
pub fn sweep_transitions(
    prev: &HashMap<String, String>,
    current: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (cid, state) in current {
        if prev.get(cid) != Some(state) {
            out.push((cid.clone(), state.clone()));
        }
    }
    for cid in prev.keys() {
        if !current.contains_key(cid) {
            out.push((cid.clone(), "Swept".to_string()));
        }
    }
    out
}

fn sweep_state_name(bt: &BalanceType) -> (&'static str, String, u64) {
    match bt {
        BalanceType::PendingBroadcast(x) => ("PendingBroadcast", x.channel_id.clone().unwrap_or_default(), x.amount_satoshis),
        BalanceType::BroadcastAwaitingConfirmation(x) => ("BroadcastAwaitingConfirmation", x.channel_id.clone().unwrap_or_default(), x.amount_satoshis),
        BalanceType::AwaitingThresholdConfirmations(x) => ("AwaitingThresholdConfirmations", x.channel_id.clone().unwrap_or_default(), x.amount_satoshis),
    }
}

async fn poll_sweeps(state: &AppState, prev: &mut HashMap<String, String>) {
    let ldk: &dyn LdkServerCalls = state.ldk_server.as_ref();
    let resp = match ldk.get_balances(GetBalancesRequest {}).await {
        Ok(r) => r,
        Err(e) => {
            warn!("[observability] get_balances failed: {}", e);
            return;
        }
    };
    let mut current: HashMap<String, String> = HashMap::new();
    let mut amounts: HashMap<String, u64> = HashMap::new();
    for b in &resp.pending_balances_from_channel_closures {
        if let Some(bt) = &b.balance_type {
            let (name, cid, amt) = sweep_state_name(bt);
            if !cid.is_empty() {
                current.insert(cid.clone(), name.to_string());
                amounts.insert(cid, amt);
            }
        }
    }
    for (cid, state_name) in sweep_transitions(prev, &current) {
        let uid = state.db.get_user_channel_id_by_channel_id(&cid).ok().flatten();
        stable_channels::audit::audit_event(
            "SWEEP_PROGRESS",
            serde_json::json!({
                "channel_id": cid,
                "user_channel_id": uid,
                "sweep_state": state_name,
                "amount_sats": amounts.get(&cid),
            }),
        );
    }
    *prev = current;
}

/// Pure: connect/disconnect transitions for counterparty peers. First run only establishes a baseline.
pub fn peer_transitions(
    prev: &HashMap<String, bool>,
    current: &HashMap<String, bool>,
    first_run: bool,
) -> Vec<(String, bool)> {
    if first_run {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (node, connected) in current {
        if prev.get(node) != Some(connected) {
            out.push((node.clone(), *connected));
        }
    }
    out
}

async fn poll_peers(state: &AppState, prev: &mut HashMap<String, bool>, first_run: &mut bool) {
    use ldk_server_client::ldk_server_grpc::api::{ListChannelsRequest, ListPeersRequest};
    let ldk: &dyn LdkServerCalls = state.ldk_server.as_ref();
    let peers = match ldk.list_peers(ListPeersRequest {}).await {
        Ok(r) => r.peers,
        Err(e) => { warn!("[observability] list_peers failed: {}", e); return; }
    };
    let channels = match ldk.list_channels(ListChannelsRequest {}).await {
        Ok(r) => r.channels,
        Err(e) => { warn!("[observability] list_channels failed: {}", e); return; }
    };
    // counterparty node_id -> its user_channel_ids
    let mut cp_uids: HashMap<String, Vec<String>> = HashMap::new();
    for c in &channels {
        cp_uids.entry(c.counterparty_node_id.clone()).or_default().push(c.user_channel_id.clone());
    }
    // current connection state, counterparties only
    let mut current: HashMap<String, bool> = HashMap::new();
    let mut address: HashMap<String, String> = HashMap::new();
    for p in &peers {
        if cp_uids.contains_key(&p.node_id) {
            current.insert(p.node_id.clone(), p.is_connected);
            address.insert(p.node_id.clone(), p.address.clone());
        }
    }
    for (node, connected) in peer_transitions(prev, &current, *first_run) {
        stable_channels::audit::audit_event(
            if connected { "PEER_CONNECTED" } else { "PEER_DISCONNECTED" },
            serde_json::json!({
                "counterparty_node_id": node,
                "user_channel_ids": cp_uids.get(&node),
                "address": address.get(&node),
            }),
        );
    }
    *prev = current;
    *first_run = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn sweep_transitions_detects_enter_advance_and_swept() {
        let prev: HashMap<String, String> = HashMap::new();
        let mut cur = HashMap::new();
        cur.insert("c1".to_string(), "PendingBroadcast".to_string());
        let t = sweep_transitions(&prev, &cur);
        assert_eq!(t, vec![("c1".to_string(), "PendingBroadcast".to_string())]); // first-seen

        let mut cur2 = HashMap::new();
        cur2.insert("c1".to_string(), "BroadcastAwaitingConfirmation".to_string());
        assert_eq!(sweep_transitions(&cur, &cur2), vec![("c1".to_string(), "BroadcastAwaitingConfirmation".to_string())]);

        let empty = HashMap::new();
        assert_eq!(sweep_transitions(&cur2, &empty), vec![("c1".to_string(), "Swept".to_string())]); // left the set
        assert!(sweep_transitions(&cur2, &cur2).is_empty()); // no change
    }

    #[test]
    fn peer_transitions_baseline_then_changes() {
        let mut cur = HashMap::new();
        cur.insert("02aa".to_string(), true);
        assert!(peer_transitions(&HashMap::new(), &cur, true).is_empty()); // first run = baseline, no emit

        let mut cur2 = HashMap::new();
        cur2.insert("02aa".to_string(), false);
        assert_eq!(peer_transitions(&cur, &cur2, false), vec![("02aa".to_string(), false)]); // flipped to disconnected

        assert!(peer_transitions(&cur2, &cur2, false).is_empty()); // no change
    }
}
