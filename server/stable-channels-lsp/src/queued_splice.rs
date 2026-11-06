//! Operator-queued LSP splice-ins. The operator still decides when and how much; this module only
//! makes that splice finish when the phone is away. It pushes the phone first, even when it is
//! connected, so the app can hold its node for the splice; waits for the channel to be usable;
//! makes the same splice-in call the Splice+ button makes; and follows the splice to its lock.
//! Every step is in the `queued_splices` table, so a restart resumes it.
//!
//! One queued splice must never be spliced in twice, so a row that made a call is `pending_on_node`
//! until the node answers: it refused the call, dropped the splice (`SpliceNegotiationFailed`), the
//! channel closed, or the splice locked. Nothing else clears the flag, and the operator can release
//! it after checking the node. While it is set no second call and no new request go out.
//!
//! A lock is attributed by balance, not by events, which a user's splice can produce in the same
//! window: the channel's funding outpoint has moved on from the one the call was made on, the
//! channel's value has grown by the amount, and the LSP's side has grown by the amount. The job
//! checks this on every pass, so a missed event, a lost answer, a restart or a cancel never loses a
//! splice the node completed.

use std::sync::Arc;
use std::time::Duration;

use ldk_server_client::error::LdkServerErrorCode;
use ldk_server_client::ldk_server_grpc::api::{
    splice_in_request, GetBalancesRequest, ListChannelsRequest, SpliceInRequest,
};
use ldk_server_client::ldk_server_grpc::types::Channel;
use serde_json::{json, Value};
use stable_channels::audit::audit_event;
use stable_channels::db::{Database, QueueSpliceOutcome, QueuedSplice};
use tokio::sync::Mutex;
use tokio::time::{timeout, MissedTickBehavior};
use tracing::{info, warn};

use crate::push::PushService;
use crate::stable_manager::{channel_peer_balances, parse_user_channel_id, LdkServerCalls};
use crate::state::AppState;

/// The push direction for a queued splice; the wallet keeps its node up until the splice locks or fails.
pub const LSP_SPLICE_IN_DIRECTION: &str = "lsp_splice_in";
/// How often open rows are looked at; each pass costs one list_channels call while a row is open.
const JOB_INTERVAL_SECS: u64 = 5;
/// Time allowed for push delivery, app start, node start and reconnect, as for a stability wake.
pub(crate) const WAKE_TIMEOUT_SECS: i64 = 60;
/// The splice-in call waits this long after the push, so an app that was already connected has set its hold before the node sends splice_init.
pub(crate) const WAKE_GRACE_SECS: i64 = 3;
/// A call the node has not answered gets the phone pushed again this often: the node negotiates or finishes the splice when the phone reconnects.
pub(crate) const PENDING_WAKE_SECS: i64 = 300;
/// A call the node has not answered within this time stalls the row, which alerts the operator. The call stays pending: the node may still finish it, or may never have received it.
pub(crate) const PENDING_ALERT_SECS: i64 = 600;
/// A failed attempt is not retried before this long. A retry that needs a push also waits for the node's push cooldown.
pub(crate) const RETRY_BACKOFF_SECS: i64 = 120;
/// A stalled row is not retried before this long, so a refusal that keeps happening is recorded hourly and not every pass.
pub(crate) const STALLED_RETRY_BACKOFF_SECS: i64 = 3_600;
/// Failed attempts after which the row stalls and the operator is alerted. A stalled row is no longer pushed for, but still splices if the phone shows up.
pub(crate) const MAX_ATTEMPTS: u32 = 5;
/// On-chain sats kept above the amount for the splice transaction's fee.
pub(crate) const FEE_HEADROOM_SATS: u64 = 5_000;
/// How long a cancelled row whose call may still lock is watched for that lock.
pub(crate) const CANCELLED_RECONCILE_SECS: i64 = 7 * 86_400;
/// Bound on one node call, so a stalled call costs one pass and not the whole job.
const RPC_TIMEOUT_SECS: u64 = 5;
/// Bound on the splice-in call itself; the node selects coins and builds the funding template before it answers.
const SPLICE_IN_TIMEOUT_SECS: u64 = 30;

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(JOB_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        info!("[queued_splice] running every {}s while a splice is queued", JOB_INTERVAL_SECS);
        loop {
            ticker.tick().await;
            let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
            run_once(&state.db, ldk, &state.push, &crate::revenue::now_secs).await;
        }
    });
}

fn rpc_timeout() -> Duration {
    Duration::from_secs(RPC_TIMEOUT_SECS)
}

/// The decimal form of a user channel id, which every row is stored under; None when it is not an id.
fn row_key(user_channel_id: &str) -> Option<String> {
    parse_user_channel_id(user_channel_id).map(|uid| uid.to_string())
}

/// The channel's funding outpoint as the node's events spell it.
fn channel_funding_txo(channel: &Channel) -> Option<String> {
    channel.funding_txo.as_ref().map(|outpoint| format!("{}:{}", outpoint.txid, outpoint.vout))
}

/// The LSP's side of the channel, spendable plus reserve.
fn lsp_side_sats(channel: &Channel) -> u64 {
    channel_peer_balances(channel).0
}

/// Sats missing from the on-chain wallet for a splice of `amount_sats` plus fee headroom; None when it is funded.
pub(crate) fn funding_shortfall_sats(spendable_onchain_sats: u64, amount_sats: u64) -> Option<u64> {
    let needed = amount_sats.saturating_add(FEE_HEADROOM_SATS);
    (spendable_onchain_sats < needed).then(|| needed - spendable_onchain_sats)
}

/// Whether the channel shows the row's own splice locked: its funding has moved on from the outpoint the call was made on, and both the channel's value and the LSP's side have grown by at least nine tenths of the amount since the call. A user's splice-in grows the value but not the LSP's side; payments move the LSP's side but not the value.
fn locked_by_balance(row: &QueuedSplice, channel: &Channel) -> bool {
    let (Some(_), Some(value_before), Some(lsp_before)) =
        (row.channel_funding_txo.as_deref(), row.channel_value_before, row.lsp_sats_before)
    else {
        return false;
    };
    let grown = row.amount_sats / 10 * 9;
    channel_funding_txo(channel) != row.channel_funding_txo
        && channel.channel_value_sats >= value_before.saturating_add(grown)
        && lsp_side_sats(channel) >= lsp_before.saturating_add(grown)
}

fn audit(event: &str, row: &QueuedSplice, extra: &[(&str, Value)]) {
    let mut detail = serde_json::Map::new();
    detail.insert("user_channel_id".into(), row.user_channel_id.clone().into());
    detail.insert("node_id".into(), row.counterparty.clone().into());
    detail.insert("amount_sats".into(), row.amount_sats.into());
    detail.insert("splice_status".into(), row.status.clone().into());
    detail.insert("attempts".into(), row.attempts.into());
    detail.insert("pending_on_node".into(), row.pending_on_node.into());
    if let Some(txo) = &row.funding_txo {
        detail.insert("funding_txo".into(), txo.clone().into());
    }
    if let Some(error) = &row.last_error {
        detail.insert("error".into(), error.clone().into());
    }
    for (key, value) in extra {
        detail.insert((*key).into(), value.clone());
    }
    audit_event(event, Value::Object(detail));
}

fn dedup_key(step: &str, row: &QueuedSplice, suffix: impl std::fmt::Display) -> Value {
    json!(format!("lsp:queued-splice-{step}:{}:{}:{suffix}", row.user_channel_id, row.request_id))
}

fn db_failed(op: &str, user_channel_id: &str, error: rusqlite::Error) {
    audit_event(
        "DB_WRITE_FAILED",
        json!({ "op": op, "user_channel_id": user_channel_id, "error": error.to_string() }),
    );
}

/// Queue a splice-in for the operator: whether it was queued, and a message for them.
pub(crate) async fn queue(
    db: &Database,
    ldk: &dyn LdkServerCalls,
    user_channel_id: &str,
    counterparty: &str,
    amount_sats: u64,
    now: i64,
) -> (bool, String) {
    if amount_sats == 0 {
        return (false, "the amount must be above zero".into());
    }
    let Some(key) = row_key(user_channel_id) else {
        return (false, format!("{user_channel_id:?} is not a channel id"));
    };
    let channels = match timeout(rpc_timeout(), ldk.list_channels(ListChannelsRequest {})).await {
        Ok(Ok(response)) => response.channels,
        Ok(Err(error)) => return (false, format!("list_channels failed: {error}")),
        Err(_) => return (false, "list_channels timed out".into()),
    };
    let Some(channel) = channels.iter().find(|c| row_key(&c.user_channel_id).as_deref() == Some(key.as_str())) else {
        return (false, "no open channel has that id".into());
    };
    if channel.counterparty_node_id != counterparty {
        return (false, format!("channel {key} is with {}, not {counterparty}", channel.counterparty_node_id));
    }
    // A call the last request made may have locked since the job last looked.
    match db.load_queued_splice(&key) {
        Ok(Some(existing)) if existing.pending_on_node => {
            lock_if_shown(db, &existing, channel, now);
        }
        Ok(_) => {}
        Err(error) => return (false, format!("could not read the queued splice: {error}")),
    }
    let spendable = match timeout(rpc_timeout(), ldk.get_balances(GetBalancesRequest {})).await {
        Ok(Ok(balances)) => balances.spendable_onchain_balance_sats,
        Ok(Err(error)) => return (false, format!("get_balances failed: {error}")),
        Err(_) => return (false, "get_balances timed out".into()),
    };
    if let Some(shortfall) = funding_shortfall_sats(spendable, amount_sats) {
        audit_event(
            "QUEUED_SPLICE_UNFUNDED",
            json!({
                "user_channel_id": key, "node_id": counterparty, "amount_sats": amount_sats,
                "spendable_onchain_sats": spendable, "shortfall_sats": shortfall,
                "fee_headroom_sats": FEE_HEADROOM_SATS,
            }),
        );
        return (
            false,
            format!(
                "the on-chain wallet is {shortfall} sats short: {spendable} spendable, {} needed with {FEE_HEADROOM_SATS} sats kept for the fee",
                amount_sats.saturating_add(FEE_HEADROOM_SATS)
            ),
        );
    }
    match db.queue_splice_in(&key, counterparty, amount_sats, now) {
        Ok(QueueSpliceOutcome::Queued(row)) => {
            audit("QUEUED_SPLICE_CREATED", &row, &[("dedup_key", dedup_key("created", &row, ""))]);
            (true, format!("queued {amount_sats} sats; the phone is woken by push and the splice runs once the channel is usable"))
        }
        Ok(QueueSpliceOutcome::Busy(row)) => (
            false,
            format!("a splice of {} sats is already {} for this channel; cancel it first", row.amount_sats, row.status),
        ),
        Ok(QueueSpliceOutcome::PendingOnNode(row)) => (
            false,
            format!(
                "the splice-in call of {} sats made for this channel at {} may still be pending on the node ({}); a new request can go out once it locks or the node drops it, or once you have checked that the node holds no splice for this channel and cancelled it with force",
                row.amount_sats,
                row.last_attempt_at.unwrap_or(row.updated_at),
                row.last_error.as_deref().unwrap_or(&row.status)
            ),
        ),
        Err(error) => {
            let message = format!("could not save the queued splice: {error}");
            db_failed("queue_splice_in", &key, error);
            (false, message)
        }
    }
}

/// Cancel a channel's queued splice for the operator: whether a row was cancelled, and a message for them. With `force` the operator has checked that the node holds no splice for the channel, so a call still counted as pending is released.
pub(crate) fn cancel(db: &Database, user_channel_id: &str, force: bool, now: i64) -> (bool, String) {
    let Some(key) = row_key(user_channel_id) else {
        return (false, format!("{user_channel_id:?} is not a channel id"));
    };
    let reason = if force { "cancelled and released by the operator" } else { "cancelled by the operator" };
    match db.cancel_queued_splice(&key, Some(reason), force, now) {
        Ok(Some(row)) => {
            audit("QUEUED_SPLICE_CANCELLED", &row, &[("forced", json!(force)), ("dedup_key", dedup_key("cancelled", &row, force))]);
            let status = if row.pending_on_node {
                "cancelled; the splice-in call already made cannot be taken back: if the node completes it the row will show it locked, and a new request for this channel waits until then, or until you cancel with force after checking that the node holds no splice for it"
            } else if force {
                "cancelled and released; a new request for this channel may go out"
            } else {
                "cancelled"
            };
            (true, status.into())
        }
        Ok(None) => (false, "this channel has no queued splice that can be cancelled".into()),
        Err(error) => {
            let message = format!("could not cancel the queued splice: {error}");
            db_failed("cancel_queued_splice", &key, error);
            (false, message)
        }
    }
}

/// One pass over the open rows: settle, push, splice, or time out each one according to where it is.
pub(crate) async fn run_once(
    db: &Database,
    ldk: &dyn LdkServerCalls,
    push: &Arc<Mutex<PushService>>,
    clock: &(dyn Fn() -> i64 + Sync),
) {
    let rows = match db.list_open_queued_splices(clock(), CANCELLED_RECONCILE_SECS) {
        Ok(rows) => rows,
        Err(error) => {
            warn!("[queued_splice] could not read the queue: {}", error);
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    let channels = match timeout(rpc_timeout(), ldk.list_channels(ListChannelsRequest {})).await {
        Ok(Ok(response)) => response.channels,
        Ok(Err(error)) => {
            warn!("[queued_splice] list_channels failed: {}", error);
            return;
        }
        Err(_) => {
            warn!("[queued_splice] list_channels timed out");
            return;
        }
    };
    for row in rows {
        // A pass can spend a provider timeout and a splice-in call on earlier rows.
        let now = clock();
        let channel = channels.iter().find(|c| {
            row_key(&c.user_channel_id).as_deref() == Some(row.user_channel_id.as_str())
                && c.counterparty_node_id == row.counterparty
        });
        let Some(channel) = channel else {
            // The node's list is complete, so a channel missing from it is gone.
            end_for_channel(db, &row, "the node no longer lists this channel", now);
            continue;
        };
        if row.pending_on_node {
            follow_pending_call(db, push, &row, channel, now).await;
            continue;
        }
        let usable = channel.is_usable;
        match row.status.as_str() {
            "queued" | "failed" | "stalled" => {
                let backoff = if row.status == "stalled" { STALLED_RETRY_BACKOFF_SECS } else { RETRY_BACKOFF_SECS };
                if row.status != "queued" && now < row.last_attempt_at.unwrap_or(0) + backoff {
                    continue;
                }
                if row.status == "stalled" {
                    // No more pushes for a stalled row, but the splice still runs if the phone shows up.
                    if usable {
                        start_splice(db, ldk, &row, channel, now, false).await;
                    }
                } else {
                    wake_or_splice(db, ldk, push, &row, channel, now).await;
                }
            }
            "waking" => {
                let sent_at = row.wake_sent_at.unwrap_or(row.updated_at);
                if usable && now >= sent_at + WAKE_GRACE_SECS {
                    start_splice(db, ldk, &row, channel, now, true).await;
                } else if now >= sent_at + WAKE_TIMEOUT_SECS {
                    fail(db, &row, &format!("the phone did not come online within {WAKE_TIMEOUT_SECS}s of the push"), now);
                }
            }
            other => warn!("[queued_splice] {} is {:?} with no call pending", row.user_channel_id, other),
        }
    }
}

/// A row whose call the node has not answered: lock it when the channel shows the splice, otherwise alert the operator once it has waited too long and keep waking the phone, which is all the node needs to go on.
async fn follow_pending_call(db: &Database, push: &Arc<Mutex<PushService>>, row: &QueuedSplice, channel: &Channel, now: i64) {
    if lock_if_shown(db, row, channel, now) || row.status == "cancelled" {
        return;
    }
    let called_at = row.last_attempt_at.unwrap_or(row.updated_at);
    if row.status != "stalled" && now >= called_at + PENDING_ALERT_SECS {
        let error = match row.funding_txo.as_deref() {
            Some(txo) => format!("the splice was signed ({txo}) but has not locked within {PENDING_ALERT_SECS}s; the node finishes it when the phone reconnects"),
            None => format!("the node has not reported the splice-in call within {PENDING_ALERT_SECS}s; it may still be pending there, or may never have reached it. Cancel with force once you have checked that the node holds no splice for this channel"),
        };
        match db.stall_queued_splice_pending(&row.user_channel_id, &error, now) {
            Ok(Some(stalled)) => audit(
                "QUEUED_SPLICE_STALLED",
                &stalled,
                &[("status", json!("failed")), ("dedup_key", dedup_key("stalled", &stalled, "pending"))],
            ),
            Ok(None) => {}
            Err(error) => db_failed("stall_queued_splice_pending", &row.user_channel_id, error),
        }
    }
    let since = row.wake_sent_at.unwrap_or(0).max(called_at);
    if now >= since + PENDING_WAKE_SECS {
        wake_again(db, push, row, now).await;
    }
}

/// Lock the row when the channel shows its splice completed. True when it did.
fn lock_if_shown(db: &Database, row: &QueuedSplice, channel: &Channel, now: i64) -> bool {
    if !locked_by_balance(row, channel) {
        return false;
    }
    let current = channel_funding_txo(channel).unwrap_or_default();
    match db.mark_queued_splice_locked(&row.user_channel_id, &current, now) {
        Ok(Some(locked)) => {
            info!("[queued_splice] splice-in of {} sats locked on {} ({})", row.amount_sats, row.user_channel_id, current);
            audit(
                "QUEUED_SPLICE_LOCKED",
                &locked,
                &[
                    ("status", json!("completed")),
                    ("channel_value_before", json!(row.channel_value_before)),
                    ("channel_value_after", json!(channel.channel_value_sats)),
                    ("lsp_sats_before", json!(row.lsp_sats_before)),
                    ("lsp_sats_after", json!(lsp_side_sats(channel))),
                    ("dedup_key", dedup_key("locked", row, &current)),
                ],
            );
        }
        Ok(None) => {}
        Err(error) => db_failed("mark_queued_splice_locked", &row.user_channel_id, error),
    }
    true
}

/// Push the phone before anything else, so the app can hold its node; when no push can go out now and the phone is online anyway, splice while it is here.
async fn wake_or_splice(
    db: &Database,
    ldk: &dyn LdkServerCalls,
    push: &Arc<Mutex<PushService>>,
    row: &QueuedSplice,
    channel: &Channel,
    now: i64,
) {
    if send_push(db, push, row, now).await {
        match db.mark_queued_splice_waking(&row.user_channel_id, now) {
            Ok(Some(woken)) => audit("QUEUED_SPLICE_WAKE_SENT", &woken, &[("timeout_secs", json!(WAKE_TIMEOUT_SECS))]),
            Ok(None) => {}
            Err(error) => db_failed("mark_queued_splice_waking", &row.user_channel_id, error),
        }
        return;
    }
    if channel.is_usable {
        start_splice(db, ldk, row, channel, now, false).await;
    }
}

/// Push the phone again for a call the node is still to answer; the row keeps its state.
async fn wake_again(db: &Database, push: &Arc<Mutex<PushService>>, row: &QueuedSplice, now: i64) {
    if !send_push(db, push, row, now).await {
        return;
    }
    match db.note_queued_splice_wake_sent(&row.user_channel_id, now) {
        Ok(Some(woken)) => audit("QUEUED_SPLICE_WAKE_SENT", &woken, &[("timeout_secs", json!(PENDING_WAKE_SECS))]),
        Ok(None) => {}
        Err(error) => db_failed("note_queued_splice_wake_sent", &row.user_channel_id, error),
    }
}

/// True when the provider accepted a push for the row; otherwise the reason is noted on the row.
async fn send_push(db: &Database, push: &Arc<Mutex<PushService>>, row: &QueuedSplice, now: i64) -> bool {
    let node_id = row.counterparty.as_str();
    let (has_token, may_push) = {
        let service = push.lock().await;
        (service.has_token(node_id), service.should_notify(node_id))
    };
    let note = if !has_token {
        "no usable push token for this phone (none registered, or older than 30 days)"
    } else if !may_push {
        "waiting for the phone's push cooldown"
    } else if crate::push::notify(push, node_id, LSP_SPLICE_IN_DIRECTION).await {
        return true;
    } else {
        "the push provider did not accept the push"
    };
    if let Err(error) = db.note_queued_splice_wake_issue(&row.user_channel_id, note, now) {
        db_failed("note_queued_splice_wake_issue", &row.user_channel_id, error);
    }
    false
}

/// Make the splice-in call for a row with no call pending, on a channel that is usable.
async fn start_splice(
    db: &Database,
    ldk: &dyn LdkServerCalls,
    row: &QueuedSplice,
    channel: &Channel,
    now: i64,
    pushed: bool,
) {
    if row.pending_on_node {
        return;
    }
    let spendable = match timeout(rpc_timeout(), ldk.get_balances(GetBalancesRequest {})).await {
        Ok(Ok(balances)) => balances.spendable_onchain_balance_sats,
        // Not an attempt: the next pass asks again.
        Ok(Err(error)) => {
            warn!("[queued_splice] get_balances failed: {}", error);
            return;
        }
        Err(_) => {
            warn!("[queued_splice] get_balances timed out");
            return;
        }
    };
    if let Some(shortfall) = funding_shortfall_sats(spendable, row.amount_sats) {
        fail(
            db,
            row,
            &format!("the on-chain wallet is {shortfall} sats short: {spendable} spendable, {} needed", row.amount_sats.saturating_add(FEE_HEADROOM_SATS)),
            now,
        );
        return;
    }
    let current_txo = channel_funding_txo(channel);
    let lsp_before = lsp_side_sats(channel);
    let attempt = match db.mark_queued_splice_splicing(
        &row.user_channel_id,
        row.request_id,
        current_txo.as_deref(),
        channel.channel_value_sats,
        lsp_before,
        now,
    ) {
        Ok(Some(attempt)) => attempt,
        // The request was replaced or changed under this pass; the row as it is now gets its own pass.
        Ok(None) => return,
        Err(error) => {
            db_failed("mark_queued_splice_splicing", &row.user_channel_id, error);
            return;
        }
    };
    let request = SpliceInRequest {
        user_channel_id: channel.user_channel_id.clone(),
        counterparty_node_id: attempt.counterparty.clone(),
        amount: Some(splice_in_request::Amount::SpliceAmountSats(attempt.amount_sats)),
    };
    match timeout(Duration::from_secs(SPLICE_IN_TIMEOUT_SECS), ldk.splice_in(request)).await {
        Ok(Ok(_)) => {
            info!("[queued_splice] splice-in of {} sats started on {}", attempt.amount_sats, attempt.user_channel_id);
            audit(
                "QUEUED_SPLICE_STARTED",
                &attempt,
                &[
                    ("pushed", json!(pushed)),
                    ("attempt", json!(attempt.attempts + 1)),
                    ("channel_funding_txo", json!(current_txo)),
                    ("channel_value_before", json!(channel.channel_value_sats)),
                    ("lsp_sats_before", json!(lsp_before)),
                    ("dedup_key", dedup_key("started", &attempt, attempt.attempts + 1)),
                ],
            );
        }
        // The answer was lost in transport, or the request never arrived; either way the node may hold the call, so it stays pending until the node or the operator says otherwise.
        Ok(Err(error)) if matches!(error.error_code, LdkServerErrorCode::InternalError) => {
            warn!("[queued_splice] splice_in answer lost for {}: {}", attempt.user_channel_id, error);
        }
        // The node refused, so nothing went to the phone.
        Ok(Err(error)) => fail(db, &attempt, &format!("splice_in: {}", error.message), now),
        Err(_) => warn!("[queued_splice] splice_in timed out for {}; waiting for the node's events", attempt.user_channel_id),
    }
}

/// Count a failed attempt on the row as it was read, and alert the operator when the row stalls on it. The node answered, or no call was made, so nothing of the row is pending.
fn fail(db: &Database, row: &QueuedSplice, error: &str, now: i64) {
    match db.fail_queued_splice(&row.user_channel_id, &row.status, error, now, MAX_ATTEMPTS) {
        Ok(Some(failed)) => {
            audit("QUEUED_SPLICE_FAILED", &failed, &[("dedup_key", dedup_key("failed", &failed, failed.attempts))]);
            if failed.status == "stalled" && failed.attempts == MAX_ATTEMPTS {
                audit(
                    "QUEUED_SPLICE_STALLED",
                    &failed,
                    &[
                        ("status", json!("failed")),
                        ("max_attempts", json!(MAX_ATTEMPTS)),
                        ("dedup_key", dedup_key("stalled", &failed, failed.attempts)),
                    ],
                );
            }
        }
        Ok(None) => {}
        Err(error) => db_failed("fail_queued_splice", &row.user_channel_id, error),
    }
}

/// The channel is gone: nothing on it can be pending or lock.
fn end_for_channel(db: &Database, row: &QueuedSplice, reason: &str, now: i64) {
    match db.close_queued_splice_channel(&row.user_channel_id, reason, now) {
        Ok(Some(ended)) if row.status != "cancelled" => {
            audit("QUEUED_SPLICE_CANCELLED", &ended, &[("dedup_key", dedup_key("cancelled", &ended, "channel"))]);
        }
        Ok(_) => {}
        Err(error) => db_failed("close_queued_splice_channel", &row.user_channel_id, error),
    }
}

/// The node negotiated a splice on the channel. Only a row whose call is out notes it; whether it was this row's is settled at the lock, by balance.
pub(crate) fn on_splice_negotiated(db: &Database, user_channel_id: &str, funding_txo: &str, now: i64) {
    let Some(key) = row_key(user_channel_id) else { return };
    match db.note_queued_splice_negotiated(&key, funding_txo, now) {
        Ok(Some(row)) => audit("QUEUED_SPLICE_NEGOTIATED", &row, &[]),
        Ok(None) => {}
        Err(error) => db_failed("note_queued_splice_negotiated", &key, error),
    }
}

/// The node dropped the splice it was negotiating on the channel, so a row whose call was out goes back for a retry and nothing of it is pending any more.
pub(crate) fn on_splice_negotiation_failed(db: &Database, user_channel_id: &str, now: i64) {
    let Some(key) = row_key(user_channel_id) else { return };
    let row = match db.load_queued_splice(&key) {
        Ok(Some(row)) => row,
        Ok(None) => return,
        Err(error) => {
            audit_event("DB_READ_FAILED", json!({ "op": "load_queued_splice", "user_channel_id": key, "error": error.to_string() }));
            return;
        }
    };
    if !row.pending_on_node {
        return;
    }
    if matches!(row.status.as_str(), "splicing" | "negotiated") {
        fail(db, &row, "the splice negotiation failed", now);
    } else if let Err(error) = db.clear_queued_splice_pending(&key, "the node dropped the splice it was negotiating", now) {
        db_failed("clear_queued_splice_pending", &key, error);
    }
}

/// The channel closed, so nothing can be spliced into it.
pub(crate) fn on_channel_closed(db: &Database, user_channel_id: &str, now: i64) {
    let Some(key) = row_key(user_channel_id) else { return };
    let row = match db.load_queued_splice(&key) {
        Ok(Some(row)) => row,
        Ok(None) => return,
        Err(error) => {
            audit_event("DB_READ_FAILED", json!({ "op": "load_queued_splice", "user_channel_id": key, "error": error.to_string() }));
            return;
        }
    };
    end_for_channel(db, &row, "the channel closed", now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::testing::{service, FakeSender};
    use crate::stable_manager::tests::{
        make_channel, FakeLdkServer, AUDIT_TEST_GUARD, CHANNEL_ID_HEX, COUNTERPARTY_HEX,
        USER_CHANNEL_ID_DECIMAL,
    };
    use ldk_server_client::ldk_server_grpc::types::OutPoint;
    use std::sync::atomic::Ordering;

    const FUNDED_SATS: u64 = 1_000_000;
    const AMOUNT_SATS: u64 = 50_000;
    /// The LSP's side of the test channel before any splice.
    const LSP_SATS: u64 = 60_000;
    /// The user's side of the test channel, which only a user's splice changes; larger than the amount so a payment of the amount can be shown.
    const USER_SATS: u64 = 140_000;

    /// A channel whose value is the two sides added up.
    fn channel_sides(usable: bool, funding: &str, lsp_sats: u64, user_sats: u64) -> Channel {
        let mut channel = make_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, lsp_sats + user_sats, lsp_sats * 1_000, usable);
        let (txid, vout) = funding.split_once(':').unwrap();
        channel.funding_txo = Some(OutPoint { txid: txid.into(), vout: vout.parse().unwrap() });
        channel
    }

    fn channel_with(usable: bool, funding: &str, lsp_sats: u64) -> Channel {
        channel_sides(usable, funding, lsp_sats, USER_SATS)
    }

    fn channel(usable: bool, funding: &str) -> Channel {
        channel_with(usable, funding, LSP_SATS)
    }

    fn node(usable: bool) -> FakeLdkServer {
        let fake = FakeLdkServer::new(vec![channel(usable, "aa:0")]);
        fake.spendable_onchain_sats.store(FUNDED_SATS, Ordering::SeqCst);
        fake
    }

    fn database() -> Database {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        std::mem::forget(dir);
        db
    }

    fn queued(db: &Database, now: i64) -> QueuedSplice {
        match db.queue_splice_in(USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, now).unwrap() {
            QueueSpliceOutcome::Queued(row) => row,
            other => panic!("not queued: {other:?}"),
        }
    }

    fn row(db: &Database) -> QueuedSplice {
        db.load_queued_splice(USER_CHANNEL_ID_DECIMAL).unwrap().expect("row")
    }

    fn capture() -> Vec<(String, Value)> {
        let events = stable_channels::audit::drain_test_capture();
        stable_channels::audit::disable_test_capture();
        events
    }

    fn names(events: &[(String, Value)]) -> Vec<&str> {
        events.iter().map(|(name, _)| name.as_str()).collect()
    }

    async fn pass(db: &Database, ldk: &FakeLdkServer, push: &Arc<Mutex<PushService>>, now: i64) {
        run_once(db, ldk, push, &move || now).await;
    }

    /// A call made at `now` with no push possible (no token), so the row is splicing afterwards.
    async fn called(db: &Database, ldk: &FakeLdkServer, push: &Arc<Mutex<PushService>>, now: i64) {
        queued(db, now);
        pass(db, ldk, push, now).await;
        assert_eq!((row(db).status.as_str(), row(db).pending_on_node), ("splicing", true));
    }

    #[tokio::test]
    async fn pushes_first_then_splices_once_the_channel_is_usable_and_the_grace_has_passed() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        stable_channels::audit::enable_test_capture();

        // The phone is connected, and is still pushed so the app can hold its node.
        pass(&db, &ldk, &push, 100).await;
        assert_eq!(sender.sent(), vec![(COUNTERPARTY_HEX.to_string(), LSP_SPLICE_IN_DIRECTION.to_string())]);
        assert_eq!((row(&db).status.as_str(), row(&db).wake_sent_at), ("waking", Some(100)));
        assert!(ldk.splice_ins.lock().unwrap().is_empty(), "no splice before the grace");
        pass(&db, &ldk, &push, 100 + WAKE_GRACE_SECS - 1).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty());

        pass(&db, &ldk, &push, 100 + WAKE_GRACE_SECS).await;
        let calls = ldk.splice_ins.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].user_channel_id, USER_CHANNEL_ID_DECIMAL);
        assert_eq!(calls[0].counterparty_node_id, COUNTERPARTY_HEX);
        assert_eq!(calls[0].amount, Some(splice_in_request::Amount::SpliceAmountSats(AMOUNT_SATS)));
        let splicing = row(&db);
        assert_eq!(
            (splicing.status.as_str(), splicing.channel_funding_txo.as_deref(), splicing.channel_value_before, splicing.lsp_sats_before, splicing.pending_on_node),
            ("splicing", Some("aa:0"), Some(LSP_SATS + USER_SATS), Some(LSP_SATS), true)
        );
        // Later passes make no second call while the node has not reported the splice.
        pass(&db, &ldk, &push, 110).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);

        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_STARTED"]);
        assert_eq!(events[1].1["pushed"], true);
        assert_eq!(events[1].1["attempt"], 1);
    }

    #[tokio::test]
    async fn splices_at_once_when_the_phone_is_online_but_cannot_be_pushed() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[]);
        queued(&db, 100);
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, 100).await;
        assert!(sender.sent().is_empty(), "no token, no push");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!((row(&db).status.as_str(), row(&db).wake_note), ("splicing", None));
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_STARTED"]);
        assert_eq!(events[0].1["pushed"], false);
    }

    #[tokio::test]
    async fn an_offline_phone_times_the_wake_out_and_is_pushed_again_after_the_backoff_and_cooldown() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(false);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, 100).await;
        assert_eq!(row(&db).status, "waking");
        pass(&db, &ldk, &push, 100 + WAKE_TIMEOUT_SECS - 1).await;
        assert_eq!(row(&db).status, "waking");
        pass(&db, &ldk, &push, 100 + WAKE_TIMEOUT_SECS).await;
        let failed = row(&db);
        assert_eq!((failed.status.as_str(), failed.attempts, failed.pending_on_node), ("failed", 1, false));
        assert!(failed.last_error.unwrap().contains("did not come online"));
        // The retry waits for the backoff, then for the push cooldown, which it notes without touching the error.
        let retry_at = 100 + WAKE_TIMEOUT_SECS + RETRY_BACKOFF_SECS;
        pass(&db, &ldk, &push, retry_at - 1).await;
        assert_eq!(row(&db).wake_note, None);
        pass(&db, &ldk, &push, retry_at).await;
        let waiting = row(&db);
        assert_eq!((waiting.status.as_str(), waiting.wake_note.as_deref()), ("failed", Some("waiting for the phone's push cooldown")));
        assert!(waiting.last_error.unwrap().contains("did not come online"));
        assert_eq!(sender.sent().len(), 1);
        // Once the cooldown has run out the phone is pushed again.
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, retry_at + 1).await;
        let woken = row(&db);
        assert_eq!((woken.status.as_str(), woken.wake_sent_at, woken.wake_note), ("waking", Some(retry_at + 1), None));
        assert_eq!(sender.sent().len(), 2);
        assert!(ldk.splice_ins.lock().unwrap().is_empty());
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_WAKE_SENT"]);
    }

    #[tokio::test]
    async fn a_node_refusal_counts_an_attempt_and_the_row_stalls_at_the_cap() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::LightningError, "no such peer".into()));
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        queued(&db, 100);
        stable_channels::audit::enable_test_capture();

        let mut now = 100;
        for attempt in 1..=MAX_ATTEMPTS {
            pass(&db, &ldk, &push, now).await;
            let failed = row(&db);
            assert_eq!(failed.attempts, attempt);
            assert_eq!(failed.last_error.as_deref(), Some("splice_in: no such peer"));
            assert!(!failed.pending_on_node, "a refusal is an answer");
            assert_eq!(failed.status, if attempt < MAX_ATTEMPTS { "failed" } else { "stalled" });
            // Inside the backoff nothing happens.
            pass(&db, &ldk, &push, now + 1).await;
            assert_eq!(row(&db).attempts, attempt);
            now += RETRY_BACKOFF_SECS;
        }
        // A stalled row is not pushed for and retries far less often, but the phone being online still gets it a try.
        pass(&db, &ldk, &push, now).await;
        assert_eq!(row(&db).attempts, MAX_ATTEMPTS, "the short backoff no longer applies");
        pass(&db, &ldk, &push, now - RETRY_BACKOFF_SECS + STALLED_RETRY_BACKOFF_SECS).await;
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("stalled", MAX_ATTEMPTS + 1));
        let events = capture();
        let stalled: Vec<_> = events.iter().filter(|(name, _)| name == "QUEUED_SPLICE_STALLED").collect();
        assert_eq!(stalled.len(), 1, "the operator is alerted once");
        assert_eq!(stalled[0].1["max_attempts"], MAX_ATTEMPTS);
        assert_eq!(stalled[0].1["status"], "failed");
        assert_eq!(events.iter().filter(|(name, _)| name == "QUEUED_SPLICE_FAILED").count(), MAX_ATTEMPTS as usize + 1);
    }

    #[tokio::test]
    async fn a_lost_answer_keeps_the_call_pending_until_the_channel_shows_it_locked() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::InternalError, "gRPC request failed: connection reset".into()));
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        pass(&db, &ldk, &push, 100 + WAKE_GRACE_SECS).await;
        let called_at = 100 + WAKE_GRACE_SECS;
        assert_eq!((row(&db).status.as_str(), row(&db).attempts, row(&db).pending_on_node), ("splicing", 0, true));
        *ldk.splice_in_error.lock().unwrap() = None;
        stable_channels::audit::enable_test_capture();

        // No second call, whatever the phone does; the phone is pushed again now and then; a new request is refused.
        pass(&db, &ldk, &push, called_at + PENDING_WAKE_SECS - 1).await;
        assert_eq!(sender.sent().len(), 1);
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, called_at + PENDING_WAKE_SECS).await;
        assert_eq!(sender.sent().len(), 2);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("splicing", 0));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, called_at + PENDING_WAKE_SECS + 1).await;
        assert!(!ok && status.contains("already splicing"), "{status}");
        // After long enough the operator is alerted, and the call is still pending.
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS - 1).await;
        assert_eq!(row(&db).status, "splicing");
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.attempts, stalled.pending_on_node), ("stalled", 0, true));
        assert!(stalled.last_error.as_deref().unwrap().contains("may never have reached it"));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, called_at + PENDING_ALERT_SECS + 1).await;
        assert!(!ok && status.contains("may still be pending"), "{status}");
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS + STALLED_RETRY_BACKOFF_SECS).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty(), "a pending call is never followed by another");
        // The channel then shows the splice locked: new funding, value and LSP's side up by the amount.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, "bb:1", LSP_SATS + AMOUNT_SATS - 300)];
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS + STALLED_RETRY_BACKOFF_SECS + 1).await;
        let locked = row(&db);
        assert_eq!((locked.status.as_str(), locked.funding_txo.as_deref(), locked.pending_on_node), ("locked", Some("bb:1"), false));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, called_at + PENDING_ALERT_SECS + STALLED_RETRY_BACKOFF_SECS + 2).await;
        assert!(ok, "{status}");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_LOCKED", "QUEUED_SPLICE_CREATED"]);
        assert_eq!(events[1].1["status"], "failed");
        assert_eq!(events[2].1["status"], "completed");
        assert_eq!(events[2].1["lsp_sats_after"], LSP_SATS + AMOUNT_SATS - 300);
    }

    #[tokio::test]
    async fn the_operator_can_release_a_pending_call_after_checking_the_node() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::InternalError, "gRPC request failed".into()));
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        *ldk.splice_in_error.lock().unwrap() = None;
        stable_channels::audit::enable_test_capture();

        let (ok, status) = cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 101);
        assert!(ok && status.contains("cannot be taken back"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", true));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 1, 102).await;
        assert!(!ok && status.contains("cancelled it with force"), "{status}");
        let (ok, status) = cancel(&db, USER_CHANNEL_ID_DECIMAL, true, 103);
        assert!(ok && status.contains("released"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", false));
        assert!(db.list_open_queued_splices(104, CANCELLED_RECONCILE_SECS).unwrap().is_empty());
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 1, 104).await;
        assert!(ok, "{status}");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_CANCELLED", "QUEUED_SPLICE_CANCELLED", "QUEUED_SPLICE_CREATED"]);
        assert_eq!((events[0].1["forced"].as_bool(), events[1].1["forced"].as_bool()), (Some(false), Some(true)));
        assert_ne!(events[0].1["dedup_key"], events[1].1["dedup_key"]);
    }

    #[tokio::test]
    async fn a_queue_request_locks_a_pending_call_the_channel_shows_completed_before_saving() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        *ldk.channels.lock().unwrap() = vec![channel_with(true, "bb:1", LSP_SATS + AMOUNT_SATS)];
        stable_channels::audit::enable_test_capture();

        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 200).await;
        assert!(ok, "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).amount_sats), ("queued", AMOUNT_SATS * 2));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED", "QUEUED_SPLICE_CREATED"]);
    }

    #[tokio::test]
    async fn another_splice_locking_while_the_call_is_out_is_neither_this_rows_lock_nor_its_end() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, "bb:1", 101);
        assert_eq!(row(&db).status, "negotiated");
        stable_channels::audit::enable_test_capture();

        // The user's splice-in locks on bb:1: the funding moved and the value grew, but the LSP's side did not.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, "bb:1", LSP_SATS, USER_SATS + 80_000)];
        pass(&db, &ldk, &push, 102).await;
        let waiting = row(&db);
        assert_eq!((waiting.status.as_str(), waiting.attempts, waiting.pending_on_node), ("negotiated", 0, true));
        assert!(ldk.splice_ins.lock().unwrap().len() == 1, "no second call: the node may still run this row's splice after the user's");
        // Then the node runs this row's splice, which locks on cc:2.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, "cc:2", LSP_SATS + AMOUNT_SATS, USER_SATS + 80_000)];
        pass(&db, &ldk, &push, 103).await;
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo.as_deref()), ("locked", Some("cc:2")));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn a_negotiated_splice_waits_for_its_lock_wakes_the_phone_again_and_alerts_the_operator() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        pass(&db, &ldk, &push, 100 + WAKE_GRACE_SECS).await;
        let called_at = 100 + WAKE_GRACE_SECS;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, "bb:1", called_at + 1);
        assert_eq!(row(&db).status, "negotiated");
        // The phone left after signing.
        *ldk.channels.lock().unwrap() = vec![channel(false, "aa:0")];
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, called_at + PENDING_WAKE_SECS - 1).await;
        assert_eq!(sender.sent().len(), 1);
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, called_at + PENDING_WAKE_SECS).await;
        let woken = row(&db);
        assert_eq!((woken.status.as_str(), woken.attempts, woken.wake_sent_at), ("negotiated", 0, Some(called_at + PENDING_WAKE_SECS)));
        assert_eq!(sender.sent().len(), 2);
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second call");
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.pending_on_node), ("stalled", true));
        assert!(stalled.last_error.unwrap().contains("signed (bb:1) but has not locked"));
        // Back online, the node finishes the splice.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, "bb:1", LSP_SATS + AMOUNT_SATS)];
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS + 30).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn a_cancelled_row_whose_call_the_node_completes_is_recorded_as_locked() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        stable_channels::audit::enable_test_capture();
        let (ok, status) = cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 101);
        assert!(ok && status.contains("cannot be taken back"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", true));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 1, 102).await;
        assert!(!ok && status.contains("may still be pending"), "{status}");

        *ldk.channels.lock().unwrap() = vec![channel_with(true, "bb:1", LSP_SATS + AMOUNT_SATS)];
        pass(&db, &ldk, &push, 103).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("locked", false));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CANCELLED", "QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn an_unfunded_wallet_fails_the_attempt_instead_of_calling_the_node() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        ldk.spendable_onchain_sats.store(AMOUNT_SATS + FEE_HEADROOM_SATS - 1, Ordering::SeqCst);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        queued(&db, 100);
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, 100).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty());
        let failed = row(&db);
        assert_eq!((failed.status.as_str(), failed.attempts, failed.pending_on_node), ("failed", 1, false));
        assert!(failed.last_error.unwrap().contains("1 sats short"));
        capture();
    }

    #[tokio::test]
    async fn a_request_replaced_during_the_pass_is_not_spliced_for_the_old_amount() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        let stale = queued(&db, 100);
        // The operator cancels and queues a different amount while a pass holds the old row.
        cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 101);
        db.queue_splice_in(USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 3, 102).unwrap();
        stable_channels::audit::enable_test_capture();

        start_splice(&db, &ldk, &stale, &channel(true, "aa:0"), 103, false).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty());
        assert_eq!((row(&db).status.as_str(), row(&db).amount_sats), ("queued", AMOUNT_SATS * 3));
        assert!(capture().is_empty());
    }

    #[tokio::test]
    async fn a_channel_the_node_no_longer_lists_ends_the_row() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        *ldk.channels.lock().unwrap() = vec![];
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, 101).await;
        let ended = row(&db);
        assert_eq!((ended.status.as_str(), ended.pending_on_node, ended.lsp_sats_before), ("cancelled", false, None));
        assert!(ended.last_error.unwrap().contains("no longer lists"));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CANCELLED"]);
        assert!(db.list_open_queued_splices(102, CANCELLED_RECONCILE_SECS).unwrap().is_empty());
    }

    #[tokio::test]
    async fn node_events_move_the_row() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let request_id = queued(&db, 100).request_id;
        stable_channels::audit::enable_test_capture();

        // Another splice on the channel, before this row's call is out, is not this row's.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, "cc:0", 101);
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 102);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("queued", 0));

        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some("aa:0"), LSP_SATS + USER_SATS, LSP_SATS, 103).unwrap();
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 104);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts, row(&db).pending_on_node), ("failed", 1, false));

        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some("aa:0"), LSP_SATS + USER_SATS, LSP_SATS, 105).unwrap();
        // The hex form of the same channel id finds the row too.
        let hex_id = format!("{:032x}", USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap());
        on_splice_negotiated(&db, &hex_id, "bb:1", 106);
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo.as_deref()), ("negotiated", Some("bb:1")));
        // The node dropping the splice it was negotiating ends the pending call, so the row goes back for a retry.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 107);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts, row(&db).pending_on_node), ("failed", 2, false));
        // Once the call is no longer pending, such an event is someone else's splice.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 108);
        assert_eq!(row(&db).attempts, 2);

        on_channel_closed(&db, USER_CHANNEL_ID_DECIMAL, 109);
        assert_eq!((row(&db).status.as_str(), row(&db).last_error.as_deref()), ("cancelled", Some("the channel closed")));
        on_channel_closed(&db, USER_CHANNEL_ID_DECIMAL, 110);

        let events = capture();
        assert_eq!(
            names(&events),
            ["QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_NEGOTIATED", "QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_CANCELLED"]
        );
        assert_ne!(events[0].1["dedup_key"], events[2].1["dedup_key"]);
    }

    #[tokio::test]
    async fn queue_checks_the_request_the_channel_and_the_wallet_before_saving() {
        let _guard = AUDIT_TEST_GUARD.lock().unwrap();
        let db = database();
        let ldk = node(true);
        stable_channels::audit::enable_test_capture();

        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 0, 100).await;
        assert!(!ok && status.contains("above zero"), "{status}");
        let (ok, status) = queue(&db, &ldk, "not-an-id", COUNTERPARTY_HEX, AMOUNT_SATS, 100).await;
        assert!(!ok && status.contains("not a channel id"), "{status}");
        let (ok, status) = queue(&db, &ldk, "12345", COUNTERPARTY_HEX, AMOUNT_SATS, 100).await;
        assert!(!ok && status.contains("no open channel"), "{status}");
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, "02aa", AMOUNT_SATS, 100).await;
        assert!(!ok && status.contains("not 02aa"), "{status}");
        assert!(db.load_queued_splice(USER_CHANNEL_ID_DECIMAL).unwrap().is_none());

        ldk.spendable_onchain_sats.store(AMOUNT_SATS, Ordering::SeqCst);
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 100).await;
        assert!(!ok && status.contains(&format!("{FEE_HEADROOM_SATS} sats short")), "{status}");
        assert!(db.load_queued_splice(USER_CHANNEL_ID_DECIMAL).unwrap().is_none());

        ldk.spendable_onchain_sats.store(FUNDED_SATS, Ordering::SeqCst);
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 100).await;
        assert!(ok, "{status}");
        let first = row(&db);
        assert_eq!(first.status, "queued");
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 101).await;
        assert!(!ok && status.contains("already queued"), "{status}");
        assert_eq!(row(&db).amount_sats, AMOUNT_SATS);

        let (ok, status) = cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 102);
        assert_eq!((ok, status.as_str()), (true, "cancelled"));
        assert_eq!(row(&db).status, "cancelled");
        let (ok, _) = cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 103);
        assert!(!ok, "nothing left to cancel");
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 102).await;
        assert!(ok, "{status}");
        let second = row(&db);
        assert_eq!(second.amount_sats, AMOUNT_SATS * 2);
        assert_ne!(second.request_id, first.request_id);

        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_UNFUNDED", "QUEUED_SPLICE_CREATED", "QUEUED_SPLICE_CANCELLED", "QUEUED_SPLICE_CREATED"]);
        assert_eq!(events[0].1["shortfall_sats"], FEE_HEADROOM_SATS);
        assert_ne!(events[1].1["dedup_key"], events[3].1["dedup_key"], "a request made in the same second is its own record");
    }

    #[test]
    fn funding_shortfall_keeps_the_fee_headroom() {
        assert_eq!(funding_shortfall_sats(AMOUNT_SATS + FEE_HEADROOM_SATS, AMOUNT_SATS), None);
        assert_eq!(funding_shortfall_sats(AMOUNT_SATS, AMOUNT_SATS), Some(FEE_HEADROOM_SATS));
        assert_eq!(funding_shortfall_sats(0, u64::MAX), Some(u64::MAX));
    }

    #[test]
    fn a_lock_needs_new_funding_with_the_value_and_the_lsp_side_grown_by_most_of_the_amount() {
        let mut row = QueuedSplice {
            request_id: 1,
            user_channel_id: USER_CHANNEL_ID_DECIMAL.into(),
            counterparty: COUNTERPARTY_HEX.into(),
            amount_sats: AMOUNT_SATS,
            status: "splicing".into(),
            attempts: 0,
            last_error: None,
            wake_note: None,
            funding_txo: None,
            wake_sent_at: None,
            last_attempt_at: Some(100),
            channel_funding_txo: Some("aa:0".into()),
            channel_value_before: Some(LSP_SATS + USER_SATS),
            lsp_sats_before: Some(LSP_SATS),
            pending_on_node: true,
            created_at: 100,
            updated_at: 100,
        };
        let grown = AMOUNT_SATS / 10 * 9;
        assert!(!locked_by_balance(&row, &channel_with(true, "aa:0", LSP_SATS + AMOUNT_SATS)), "same funding: not locked yet");
        assert!(!locked_by_balance(&row, &channel_sides(true, "bb:1", LSP_SATS, USER_SATS + AMOUNT_SATS)), "a user's splice-in");
        assert!(!locked_by_balance(&row, &channel_sides(true, "bb:1", LSP_SATS + AMOUNT_SATS, USER_SATS - AMOUNT_SATS)), "payments to the LSP around a user's splice, with the value unchanged");
        assert!(!locked_by_balance(&row, &channel_with(true, "bb:1", LSP_SATS + AMOUNT_SATS / 2)), "half the amount is a payment, not the splice");
        assert!(locked_by_balance(&row, &channel_with(true, "bb:1", LSP_SATS + grown)));
        assert!(locked_by_balance(&row, &channel_sides(true, "bb:1", LSP_SATS + AMOUNT_SATS - 2_000, USER_SATS + 2_000)), "a stability payment after the lock");
        assert!(locked_by_balance(&row, &channel_sides(true, "cc:2", LSP_SATS + AMOUNT_SATS, USER_SATS + 80_000)), "the user's splice and this one");
        row.channel_value_before = None;
        assert!(!locked_by_balance(&row, &channel_with(true, "bb:1", LSP_SATS + AMOUNT_SATS)), "no baseline, no lock");
    }
}
