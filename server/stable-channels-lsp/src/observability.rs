//! Periodic observability poll: synthesizes SWEEP_PROGRESS, PEER_CONNECTED/PEER_DISCONNECTED, CHANNEL_SHUTDOWN_STATE_CHANGED and CHANNEL_ONCHAIN_TX audit events.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ldk_server_client::ldk_server_grpc::api::{
    GetBalancesRequest, GetPaymentDetailsRequest, ListChannelsRequest, ListPaymentsRequest,
    ListPeersRequest,
};
use ldk_server_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;
use ldk_server_client::ldk_server_grpc::types::{Channel, Payment};
use serde_json::Value;
use stable_channels::db::Database;
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
    let mut shutdown_prev: HashMap<String, String> = HashMap::new();
    let mut onchain_pending = pending_onchain_payment_ids(&state.db);
    loop {
        tick.tick().await;
        poll_sweeps(&state, &mut sweep_prev).await;
        let ldk: &dyn LdkServerCalls = state.ldk_server.as_ref();
        let channels = match ldk.list_channels(ListChannelsRequest {}).await {
            Ok(r) => r.channels,
            Err(e) => { warn!("[observability] list_channels failed: {}", e); continue; }
        };
        poll_peers(&state, &channels, &mut peer_prev, &mut peer_first_run).await;
        shutdown_prev = record_shutdown_stages(&shutdown_prev, &channels);
        record_onchain_channel_txs(ldk, &state.db, &channels, &mut onchain_pending).await;
    }
}

/// Resolve a channel_id to its user_channel_id from the live channel list, falling back to the daemon DB for closed channels.
pub(crate) fn user_channel_id_for<'a>(
    channels: &'a [Channel],
    db: &'a Database,
) -> impl Fn(&str) -> Option<String> + 'a {
    move |channel_id: &str| {
        channels
            .iter()
            .find(|c| c.channel_id == channel_id && !c.user_channel_id.is_empty())
            .map(|c| c.user_channel_id.clone())
            .or_else(|| db.get_user_channel_id_by_channel_id(channel_id).ok().flatten())
    }
}

/// Payment ids of channel transactions the ledger last saw unconfirmed, so a restarted daemon keeps following them.
pub(crate) fn pending_onchain_payment_ids(db: &Database) -> HashSet<String> {
    let query = stable_channels::ledger::LedgerQuery {
        category: Some("channel".to_owned()),
        status: Some("pending".to_owned()),
        limit: 200,
        ..Default::default()
    };
    db.list_ledger_events(&query)
        .map(|page| {
            page.events
                .into_iter()
                .filter(|event| event.event_type == "CHANNEL_ONCHAIN_TX")
                .filter_map(|event| event.detail.get("payment_id").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Audit channel-related on-chain transactions from the newest payment page, following unconfirmed ones by id until they settle.
pub(crate) async fn record_onchain_channel_txs(
    ldk: &dyn LdkServerCalls,
    db: &Database,
    channels: &[Channel],
    pending: &mut HashSet<String>,
) {
    let resolve = user_channel_id_for(channels, db);
    let newest = match ldk.list_payments(ListPaymentsRequest { page_token: None }).await {
        Ok(r) => r.payments,
        Err(e) => { warn!("[observability] list_payments failed: {}", e); return; }
    };
    let mut seen = HashSet::new();
    for payment in &newest {
        seen.insert(payment.payment_id.clone());
        record_onchain_payment(payment, &resolve, pending);
    }
    // LDK lists newest-created first, so an older transaction confirming does not return to the first page.
    let off_page: Vec<String> = pending.iter().filter(|id| !seen.contains(*id)).cloned().collect();
    for payment_id in off_page {
        match ldk.get_payment_details(GetPaymentDetailsRequest { payment_id: payment_id.clone() }).await {
            Ok(r) => match r.payment {
                Some(payment) => record_onchain_payment(&payment, &resolve, pending),
                None => { pending.remove(&payment_id); }
            },
            Err(e) => warn!("[observability] get_payment_details({}) failed: {}", payment_id, e),
        }
    }
}

fn record_onchain_payment(
    payment: &Payment,
    resolve: &dyn Fn(&str) -> Option<String>,
    pending: &mut HashSet<String>,
) {
    let Some(mut data) = crate::channel_audit::onchain_channel_tx_audit_data(payment, resolve) else {
        return;
    };
    if data["status"] == "pending" {
        pending.insert(payment.payment_id.clone());
    } else {
        pending.remove(&payment.payment_id);
    }
    data["source"] = serde_json::json!("onchain_poll");
    stable_channels::audit::audit_event("CHANNEL_ONCHAIN_TX", data);
}

/// Audit every shutdown-stage change and return the next in-memory baseline; the per-stage dedup key keeps restarts from repeating a stage.
pub(crate) fn record_shutdown_stages(
    prev: &HashMap<String, String>,
    channels: &[Channel],
) -> HashMap<String, String> {
    let (events, next) = shutdown_audit_data(prev, channels);
    for data in events {
        stable_channels::audit::audit_event("CHANNEL_SHUTDOWN_STATE_CHANGED", data);
    }
    next
}

/// Pure: rows for channels whose shutdown stage changed since `prev` (an unseen channel emits only if already shutting down) plus the next baseline.
pub fn shutdown_audit_data(
    prev: &HashMap<String, String>,
    channels: &[Channel],
) -> (Vec<Value>, HashMap<String, String>) {
    let mut events = Vec::new();
    let mut current = HashMap::new();
    for c in channels {
        // An LDK Server older than a56d5a9 does not report the stage.
        let Some(state) = c.channel_shutdown_state else { continue };
        let name = crate::channel_audit::shutdown_state_name(state);
        let previous = prev.get(&c.channel_id);
        let changed = match previous {
            Some(previous) => previous != &name,
            None => name != "NOT_SHUTTING_DOWN",
        };
        if changed {
            events.push(serde_json::json!({
                "channel_id": c.channel_id,
                "user_channel_id": c.user_channel_id,
                "counterparty_node_id": c.counterparty_node_id,
                "previous_shutdown_state": previous,
                "shutdown_state": name,
                // Each stage is recorded the first time it is reached; a fall-back (e.g. to RESOLVING_HTLCS after a disconnect) is not re-recorded.
                "dedup_key": format!("lsp:channel-shutdown-stage:{}:{}", c.channel_id, name),
            }));
        }
        current.insert(c.channel_id.clone(), name);
    }
    (events, current)
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

async fn poll_peers(
    state: &AppState,
    channels: &[Channel],
    prev: &mut HashMap<String, bool>,
    first_run: &mut bool,
) {
    let ldk: &dyn LdkServerCalls = state.ldk_server.as_ref();
    let peers = match ldk.list_peers(ListPeersRequest {}).await {
        Ok(r) => r.peers,
        Err(e) => { warn!("[observability] list_peers failed: {}", e); return; }
    };
    // counterparty node_id -> its user_channel_ids
    let mut cp_uids: HashMap<String, Vec<String>> = HashMap::new();
    for c in channels {
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

    fn channel(state: Option<ldk_server_client::ldk_server_grpc::types::ChannelShutdownState>) -> Channel {
        Channel {
            channel_id: "f9634c603646c60b0df9f07c3011708652125915c80300a9bb8fb37c9c0de05b".into(),
            user_channel_id: "189476124653200987495269098788434301048".into(),
            counterparty_node_id: "02465ed5be53d04fde66c9418ff14a5f2267723810176c9212b722e542dc1afb1b".into(),
            channel_shutdown_state: state.map(|s| s as i32),
            ..Default::default()
        }
    }

    #[test]
    fn shutdown_stage_changes_are_audited_with_a_per_stage_dedup_key() {
        use ldk_server_client::ldk_server_grpc::types::ChannelShutdownState::*;
        let (events, open) = shutdown_audit_data(&HashMap::new(), &[channel(Some(NotShuttingDown))]);
        assert!(events.is_empty(), "a normal channel is not a shutdown event");
        let (events, next) = shutdown_audit_data(&open, &[channel(Some(ShutdownInitiated))]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["user_channel_id"], "189476124653200987495269098788434301048");
        assert_eq!(events[0]["channel_id"], "f9634c603646c60b0df9f07c3011708652125915c80300a9bb8fb37c9c0de05b");
        assert_eq!(events[0]["counterparty_node_id"], "02465ed5be53d04fde66c9418ff14a5f2267723810176c9212b722e542dc1afb1b");
        assert_eq!(events[0]["previous_shutdown_state"], "NOT_SHUTTING_DOWN");
        assert_eq!(events[0]["shutdown_state"], "SHUTDOWN_INITIATED");
        assert_eq!(
            events[0]["dedup_key"],
            "lsp:channel-shutdown-stage:f9634c603646c60b0df9f07c3011708652125915c80300a9bb8fb37c9c0de05b:SHUTDOWN_INITIATED"
        );
        let (events, _) = shutdown_audit_data(&next, &[channel(Some(ShutdownInitiated))]);
        assert!(events.is_empty(), "an unchanged stage is not re-audited");
    }

    #[test]
    fn a_close_already_underway_after_a_restart_is_audited() {
        use ldk_server_client::ldk_server_grpc::types::ChannelShutdownState::*;
        let (events, _) = shutdown_audit_data(&HashMap::new(), &[channel(Some(ResolvingHtlcs))]);
        assert_eq!(events.len(), 1);
        assert!(events[0]["previous_shutdown_state"].is_null());
        assert_eq!(events[0]["shutdown_state"], "RESOLVING_HTLCS");
    }

    #[test]
    fn channels_from_an_older_ldk_server_never_emit_shutdown_rows() {
        let (events, next) = shutdown_audit_data(&HashMap::new(), &[channel(None)]);
        assert!(events.is_empty());
        assert!(next.is_empty());
    }
}
