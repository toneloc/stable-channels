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
const ONCHAIN_DISCOVERY_PAGES: usize = 4;
const ONCHAIN_PENDING_BATCH: usize = 100;
const ONCHAIN_LEDGER_BATCH: usize = 500;

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

/// A bounded compatibility snapshot; the durable work table, not this set, owns retry obligations.
pub(crate) fn pending_onchain_payment_ids(db: &Database) -> HashSet<String> {
    match db.refresh_onchain_audit_pending(ONCHAIN_LEDGER_BATCH)
        .and_then(|_| db.onchain_audit_pending_ids(ONCHAIN_PENDING_BATCH, false))
    {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => { warn!("[observability] onchain recovery failed: {}", e); HashSet::new() }
    }
}

/// Discover up to four pages per tick, resuming unfinished paging after restart. Once a pass
/// reaches the preceding completed head, finish refreshing that page before stopping.
/// Separately follow a fair, bounded batch until LDK reports a terminal PaymentStatus.
pub(crate) async fn record_onchain_channel_txs(
    ldk: &dyn LdkServerCalls,
    db: &Database,
    channels: &[Channel],
    pending: &mut HashSet<String>,
) {
    let resolve = user_channel_id_for(channels, db);
    if let Err(e) = db.refresh_onchain_audit_pending(ONCHAIN_LEDGER_BATCH) {
        warn!("[observability] onchain ledger refresh failed: {}", e);
        return;
    }
    let mut seen = HashSet::new();
    if let Err(e) = discover_onchain_payments(ldk, db, &resolve, &mut seen).await {
        warn!("[observability] onchain discovery paused: {}", e);
    }
    let ids = match db.onchain_audit_pending_ids(ONCHAIN_PENDING_BATCH, true) {
        Ok(ids) => ids,
        Err(e) => { warn!("[observability] onchain pending query failed: {}", e); return; }
    };
    for payment_id in ids.into_iter().filter(|id| !seen.contains(id)) {
        match ldk.get_payment_details(GetPaymentDetailsRequest { payment_id: payment_id.clone() }).await {
            Ok(r) => match r.payment {
                Some(payment) => {
                    if let Err(e) = record_onchain_payment(db, &payment, &resolve) {
                        warn!("[observability] onchain write({}) failed: {}", payment_id, e);
                    }
                }
                // Absence is not a terminal LDK status; retain the obligation for a later retry.
                None => {},
            },
            Err(e) => warn!("[observability] get_payment_details({}) failed: {}", payment_id, e),
        }
    }
    if let Ok(ids) = db.onchain_audit_pending_ids(ONCHAIN_PENDING_BATCH, false) {
        *pending = ids.into_iter().collect();
    }
}

async fn discover_onchain_payments(
    ldk: &dyn LdkServerCalls,
    db: &Database,
    resolve: &(dyn Fn(&str) -> Option<String> + Sync),
    seen: &mut HashSet<String>,
) -> Result<(), String> {
    let (mut token, boundary, mut head) = db.onchain_audit_scan().map_err(|e| e.to_string())?;
    let mut cursors = HashSet::new();
    if let Some(token) = &token { cursors.insert(token.clone()); }
    for _ in 0..ONCHAIN_DISCOVERY_PAGES {
        let response = ldk.list_payments(ListPaymentsRequest { page_token: token.clone() })
            .await.map_err(|e| e.to_string())?;
        if token.is_none() || head.is_none() {
            head = response.payments.first().map(|p| p.payment_id.clone());
        }
        let mut reached_boundary = false;
        for payment in &response.payments {
            // Refresh the whole page, including rows older than the boundary: recent payments
            // may gain channel classification or change terminal/confirmation state in place.
            record_onchain_payment(db, payment, resolve).map_err(|e| e.to_string())?;
            seen.insert(payment.payment_id.clone());
            if boundary.as_deref() == Some(payment.payment_id.as_str()) {
                reached_boundary = true;
            }
        }
        if reached_boundary || response.next_page_token.is_none() {
            db.save_onchain_audit_scan(None, head.as_deref().or(boundary.as_deref()), None)
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        let next = response.next_page_token.unwrap();
        if !cursors.insert(next.clone()) {
            return Err("LDK repeated a payment page cursor; checkpoint retained".into());
        }
        // Page persistence may replay after a crash; its stop boundary must never jump ahead.
        db.save_onchain_audit_scan(Some(&next), boundary.as_deref(), head.as_deref())
            .map_err(|e| e.to_string())?;
        token = Some(next);
    }
    Ok(())
}

fn record_onchain_payment(
    db: &Database,
    payment: &Payment,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> rusqlite::Result<()> {
    let Some(mut data) = crate::channel_audit::onchain_channel_tx_audit_data(payment, resolve) else {
        return Ok(());
    };
    data["source"] = serde_json::json!("onchain_poll");
    db.append_onchain_audit_event(&stable_channels::ledger::LedgerEventDraft::from_audit_event(
        "CHANNEL_ONCHAIN_TX", data,
    ))?;
    Ok(())
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
    for p in &peers {
        if cp_uids.contains_key(&p.node_id) {
            current.insert(p.node_id.clone(), p.is_connected);
        }
    }
    let stable_nodes: std::collections::HashSet<String> = state
        .stable_manager
        .lock()
        .await
        .stable_channels
        .iter()
        .map(|sc| sc.counterparty.to_string())
        .collect();
    let sightings: Vec<(String, String, bool)> =
        peers.iter().map(|p| (p.node_id.clone(), p.address.clone(), p.is_connected)).collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    crate::geoip::record_sightings(&state.db, state.geoip.as_ref(), &sightings, &stable_nodes, now);
    for (node, connected) in peer_transitions(prev, &current, *first_run) {
        stable_channels::audit::audit_event(
            if connected { "PEER_CONNECTED" } else { "PEER_DISCONNECTED" },
            crate::geoip::peer_event_detail(&node, cp_uids.get(&node)),
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

    mod onchain_recovery {
        use super::*;
        use ldk_server_client::error::LdkServerError;
        use ldk_server_client::ldk_server_grpc::api::*;
        use ldk_server_client::ldk_server_grpc::types::{
            confirmation_status, payment_kind, transaction_type, ConfirmationStatus, Confirmed,
            Funding, Onchain, PaymentKind, PaymentStatus, TransactionChannel, TransactionType,
            Unconfirmed,
        };
        use std::sync::Mutex;

        struct Payments {
            rows: Mutex<Vec<Payment>>,
            pages: Mutex<Vec<Option<String>>>,
            details: Mutex<Vec<String>>,
        }

        impl Payments {
            fn new(rows: Vec<Payment>) -> Self {
                Self { rows: Mutex::new(rows), pages: Mutex::new(Vec::new()), details: Mutex::new(Vec::new()) }
            }
        }

        #[async_trait::async_trait]
        impl LdkServerCalls for Payments {
            async fn list_channels(&self, _: ListChannelsRequest) -> Result<ListChannelsResponse, LdkServerError> { unreachable!() }
            async fn spontaneous_send(&self, _: SpontaneousSendRequest) -> Result<SpontaneousSendResponse, LdkServerError> { unreachable!() }
            async fn sign_message(&self, _: SignMessageRequest) -> Result<SignMessageResponse, LdkServerError> { unreachable!() }
            async fn verify_signature(&self, _: VerifySignatureRequest) -> Result<VerifySignatureResponse, LdkServerError> { unreachable!() }
            async fn list_payments(&self, req: ListPaymentsRequest) -> Result<ListPaymentsResponse, LdkServerError> {
                self.pages.lock().unwrap().push(req.page_token.clone());
                let rows = self.rows.lock().unwrap();
                // Keyset pagination, matching the pinned LDK store: insertion at the head does
                // not move the saved cursor or make the resumed scan skip older entries.
                let start = req.page_token.as_ref().map(|token| {
                    rows.iter().position(|p| p.payment_id == *token).unwrap() + 1
                }).unwrap_or(0);
                let end = (start + 50).min(rows.len());
                Ok(ListPaymentsResponse {
                    payments: rows[start..end].to_vec(),
                    next_page_token: (end < rows.len()).then(|| rows[end - 1].payment_id.clone()),
                })
            }
            async fn get_payment_details(&self, req: GetPaymentDetailsRequest) -> Result<GetPaymentDetailsResponse, LdkServerError> {
                self.details.lock().unwrap().push(req.payment_id.clone());
                Ok(GetPaymentDetailsResponse {
                    payment: self.rows.lock().unwrap().iter().find(|p| p.payment_id == req.payment_id).cloned(),
                })
            }
        }

        fn payment(id: &str, status: PaymentStatus, block: Option<&str>, revision: u64) -> Payment {
            Payment {
                payment_id: id.into(), status: status as i32, latest_update_timestamp: revision,
                kind: Some(PaymentKind { kind: Some(payment_kind::Kind::Onchain(Onchain {
                    txid: format!("tx-{id}"),
                    status: Some(ConfirmationStatus { status: Some(match block {
                        Some(hash) => confirmation_status::Status::Confirmed(Confirmed {
                            block_hash: hash.into(), height: 100, timestamp: 1_758_000_000,
                        }),
                        None => confirmation_status::Status::Unconfirmed(Unconfirmed {}),
                    }) }),
                    tx_type: Some(TransactionType { kind: Some(transaction_type::Kind::Funding(Funding {
                        channels: vec![TransactionChannel { channel_id: "channel".into(), counterparty_node_id: "peer".into() }],
                    })) }),
                })) }),
                ..Default::default()
            }
        }

        fn count(db: &Database) -> u64 {
            db.list_ledger_events(&stable_channels::ledger::LedgerQuery { limit: 1, ..Default::default() })
                .unwrap().overview.total_events
        }

        fn ordinary_payments(count: usize) -> Vec<Payment> {
            (0..count).map(|i| Payment { payment_id: format!("ordinary-{i}"), ..Default::default() }).collect()
        }

        #[tokio::test]
        async fn discovers_more_than_50_transactions_without_rescanning_completed_history() {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open(dir.path()).unwrap();
            let ldk = Payments::new((0..125).map(|i| payment(&format!("p{i}"), PaymentStatus::Succeeded, Some("a"), 1)).collect());
            let mut pending = HashSet::new();
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert_eq!(count(&db), 125);
            assert_eq!(ldk.pages.lock().unwrap().len(), 3);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert_eq!(count(&db), 125);
            assert_eq!(ldk.pages.lock().unwrap().len(), 4, "steady state reads only the head page");
        }

        #[tokio::test]
        async fn newest_page_refreshes_late_classification_and_terminal_reorgs_past_boundary() {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open(dir.path()).unwrap();
            let mut rows = ordinary_payments(51);
            let mut unclassified = payment("recent", PaymentStatus::Succeeded, Some("a"), 1);
            let Some(payment_kind::Kind::Onchain(tx)) = unclassified.kind.as_mut().and_then(|k| k.kind.as_mut()) else { unreachable!() };
            tx.tx_type = None;
            rows[20] = unclassified;
            let ldk = Payments::new(rows);
            let mut pending = HashSet::new();
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert_eq!(count(&db), 0);
            assert!(pending.is_empty());
            assert_eq!(ldk.pages.lock().unwrap().len(), 2);

            // The prior head stays at index 0. Classification appears at index 20, without
            // any new payment or pending obligation to make GetPaymentDetails discover it.
            ldk.rows.lock().unwrap()[20] = payment("recent", PaymentStatus::Succeeded, Some("a"), 2);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert_eq!(count(&db), 1);
            assert!(pending.is_empty());

            // Even a previously terminal payment can reorg while still on the newest page.
            ldk.rows.lock().unwrap()[20] = payment("recent", PaymentStatus::Pending, None, 3);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert_eq!(count(&db), 2);
            assert!(pending.contains("recent"));
            ldk.rows.lock().unwrap()[20] = payment("recent", PaymentStatus::Pending, Some("b"), 4);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert_eq!(count(&db), 3);
            assert!(pending.contains("recent"));
            ldk.rows.lock().unwrap()[20] = payment("recent", PaymentStatus::Succeeded, Some("b"), 5);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert_eq!(count(&db), 4);
            assert!(pending.is_empty());
            assert!(ldk.details.lock().unwrap().is_empty(), "all updates come from the full newest-page refresh");
            let pages = ldk.pages.lock().unwrap();
            assert_eq!(pages.len(), 6, "two initial pages, then one page per refresh despite a next-page token");
            assert!(pages[2..].iter().all(Option::is_none));
            assert_eq!(db.onchain_audit_scan().unwrap(), (None, Some("ordinary-0".into()), None));
        }

        #[tokio::test]
        async fn paging_budget_resumes_after_restart_and_then_catches_new_arrivals() {
            let dir = tempfile::tempdir().unwrap();
            let ldk = Payments::new((0..351).map(|i| payment(&format!("p{i}"), PaymentStatus::Succeeded, Some("a"), 1)).collect());
            {
                let db = Database::open(dir.path()).unwrap();
                record_onchain_channel_txs(&ldk, &db, &[], &mut HashSet::new()).await;
                assert_eq!(count(&db), 200);
                assert_eq!(db.onchain_audit_scan().unwrap().0.as_deref(), Some("p199"));
            }
            ldk.rows.lock().unwrap().insert(0, payment("arrived-during-scan", PaymentStatus::Succeeded, Some("a"), 1));
            let db = Database::open(dir.path()).unwrap();
            record_onchain_channel_txs(&ldk, &db, &[], &mut HashSet::new()).await;
            assert_eq!(ldk.pages.lock().unwrap()[4].as_deref(), Some("p199"));
            assert_eq!(count(&db), 351);
            assert_eq!(db.onchain_audit_scan().unwrap().1.as_deref(), Some("p0"));
            record_onchain_channel_txs(&ldk, &db, &[], &mut HashSet::new()).await;
            assert_eq!(count(&db), 352);
            assert_eq!(ldk.pages.lock().unwrap().len(), 9);
        }

        #[tokio::test]
        async fn late_backfill_and_off_page_reorg_follow_actual_payment_terminality() {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open(dir.path()).unwrap();
            let ldk = Payments::new(ordinary_payments(51));
            let mut pending = HashSet::new();
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            let first = payment("late", PaymentStatus::Pending, Some("a"), 1);
            let detail = crate::channel_audit::onchain_channel_tx_audit_data(&first, &|_| None).unwrap();
            db.append_ledger_event(&stable_channels::ledger::LedgerEventDraft::from_audit_event("CHANNEL_ONCHAIN_TX", detail)).unwrap();
            ldk.rows.lock().unwrap().push(first);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert!(pending.contains("late"), "a first confirmation/backfill stays enrolled");
            ldk.rows.lock().unwrap()[51] = payment("late", PaymentStatus::Pending, None, 2);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            ldk.rows.lock().unwrap()[51] = payment("late", PaymentStatus::Pending, Some("b"), 3);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert!(pending.contains("late"));
            ldk.rows.lock().unwrap()[51] = payment("late", PaymentStatus::Succeeded, Some("b"), 4);
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert!(pending.is_empty());
            assert_eq!(count(&db), 4);
            assert_eq!(*ldk.details.lock().unwrap(), vec!["late"; 4], "off-page changes require detail RPCs");
            let pages = ldk.pages.lock().unwrap();
            assert_eq!(pages.len(), 6, "two initial pages, then only the newest page per poll");
            assert!(pages[2..].iter().all(Option::is_none));
        }

        #[tokio::test]
        async fn page_insert_failure_keeps_the_cursor_for_a_deduplicated_retry() {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open(dir.path()).unwrap();
            let conn = rusqlite::Connection::open(dir.path().join(stable_channels::db::DB_FILENAME)).unwrap();
            conn.execute_batch("CREATE TRIGGER reject_onchain BEFORE INSERT ON ledger_events
                WHEN json_extract(NEW.detail_json, '$.payment_id') = 'p60'
                BEGIN SELECT RAISE(FAIL, 'injected ledger failure'); END;").unwrap();
            let ldk = Payments::new((0..125).map(|i| payment(&format!("p{i}"), PaymentStatus::Succeeded, Some("a"), 1)).collect());
            record_onchain_channel_txs(&ldk, &db, &[], &mut HashSet::new()).await;
            assert_eq!(count(&db), 60);
            assert_eq!(db.onchain_audit_scan().unwrap().0.as_deref(), Some("p49"));
            conn.execute_batch("DROP TRIGGER reject_onchain").unwrap();
            record_onchain_channel_txs(&ldk, &db, &[], &mut HashSet::new()).await;
            assert_eq!(count(&db), 125);
            assert!(db.onchain_audit_scan().unwrap().0.is_none());
        }

        #[tokio::test]
        async fn failed_terminal_write_and_missing_details_keep_the_poll_obligation() {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open(dir.path()).unwrap();
            let mut rows = ordinary_payments(51);
            rows.push(payment("retry", PaymentStatus::Pending, None, 1));
            let ldk = Payments::new(rows);
            let mut pending = HashSet::new();
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert!(pending.contains("retry"));
            // Temporarily absent from LDK is not a successful terminal observation.
            assert_eq!(ldk.rows.lock().unwrap().pop().unwrap().payment_id, "retry");
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert!(pending.contains("retry"));
            ldk.rows.lock().unwrap().push(payment("retry", PaymentStatus::Succeeded, Some("a"), 2));
            let conn = rusqlite::Connection::open(dir.path().join(stable_channels::db::DB_FILENAME)).unwrap();
            conn.execute_batch("CREATE TRIGGER reject_terminal BEFORE INSERT ON ledger_events
                WHEN NEW.status = 'completed'
                BEGIN SELECT RAISE(FAIL, 'injected ledger failure'); END;").unwrap();
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert!(pending.contains("retry"));
            assert_eq!(count(&db), 1);
            conn.execute_batch("DROP TRIGGER reject_terminal").unwrap();
            record_onchain_channel_txs(&ldk, &db, &[], &mut pending).await;
            assert!(pending.is_empty());
            assert_eq!(count(&db), 2);
            assert_eq!(*ldk.details.lock().unwrap(), vec!["retry"; 3], "missing, failed-write and successful off-page retries");
            let pages = ldk.pages.lock().unwrap();
            assert_eq!(pages.len(), 5, "two initial pages, then only the newest page per poll");
            assert!(pages[2..].iter().all(Option::is_none));
        }
    }
}
