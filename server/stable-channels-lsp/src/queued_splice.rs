//! Operator-queued LSP splice-ins. The operator still decides when and how much; this module only
//! makes that splice finish when the phone is away. It pushes the phone first, even when it is
//! connected, so the app can hold its node for the splice; waits for the channel to be usable;
//! makes the same splice-in call the Splice+ button makes; and follows the splice to its lock.
//! Every step is in the `queued_splices` table, so a restart resumes it.
//!
//! One queued splice must never be spliced in twice, so a row that made a call is `pending_on_node`
//! until the node answers: it refused the call, dropped the splice (`SpliceNegotiationFailed`), the
//! channel closed, or the splice locked. Nothing else clears the flag, and the operator can release
//! it after checking the node. While it is set no second call and no new request go out, and a row
//! whose call was answered is still checked for a lock before any new call.
//!
//! A lock is attributed by what only the LSP's own splice produces. The channel's funding outpoint
//! has moved on from the one the call was made on and the channel's value has grown by the amount,
//! which a user's splice can also do; and the node's wallet took part in the new funding
//! transaction, which it records as an on-chain payment of its own at broadcast and never does for
//! a splice the user funded. The job checks this on every pass, so a missed event, a lost answer, a
//! restart or a cancel never loses a splice the node completed. While the node cannot be asked, a
//! row that may have locked gets no new call and no replacement, and stalls with an alert if that
//! lasts.
//!
//! The phone has one push slot per cooldown. A stability wake the tick decided on owns it: the
//! splice job yields its push while one is due, and splices as soon as the channel is usable.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ldk_server_client::error::LdkServerErrorCode;
use ldk_server_client::ldk_server_grpc::api::{
    splice_in_request, GetBalancesRequest, GetPaymentDetailsRequest, ListChannelsRequest,
    SpliceInRequest,
};
use ldk_server_client::ldk_server_grpc::types::{Channel, PaymentDirection};
use serde_json::{json, Value};
use stable_channels::audit::audit_event;
use stable_channels::db::{Database, QueueSpliceOutcome, QueuedSplice};
use tokio::sync::Mutex;
use tokio::time::{timeout, MissedTickBehavior};
use tracing::{info, warn};

use crate::push::PushService;
use crate::revenue::onchain_payment_id;
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
/// A call the node has not reported as negotiated within this time stalls the row, which alerts the operator. The call stays pending: the node may still run it, or may never have received it.
pub(crate) const PENDING_ALERT_SECS: i64 = 600;
/// A signed splice that has not locked within this time of the signing stalls the row. A splice that is not zero-conf locks only once its transaction has confirmed, so this is on the scale of confirmations, not of a negotiation.
pub(crate) const NEGOTIATED_ALERT_SECS: i64 = 3 * 3_600;
/// A row that has not had a call within this time of being queued, or of its last attempt, stalls, so a phone that never comes online, or has no push token, is reported instead of waiting forever.
pub(crate) const QUEUED_ALERT_SECS: i64 = 24 * 3_600;
/// What the node's own failure event records on the row it fails; a refusal records the call's error instead. A negotiation that follows such a failure this soon, before any retry could go out, revives the call: the event names no outpoint, so a stale or replayed one can fail a call whose splice is still being signed.
const NEGOTIATION_FAILED_ERROR: &str = "the splice negotiation failed";
/// A failed attempt is not retried before this long, so a phone that comes back is tried within two minutes and a refusal that keeps happening is recorded no more often than that. A retry that needs a push also waits for the node's push cooldown.
pub(crate) const RETRY_BACKOFF_SECS: i64 = 120;
/// Failed attempts after which the row stalls and the operator is alerted. A stalled row is no longer pushed for, but is still tried when the phone shows up.
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

/// The new funding transaction when the channel shows a splice that could be the row's own: its funding has moved on from the outpoint the call was made on, and its value has grown by at least nine tenths of the amount since. A user's splice-in does the same, so this alone attributes nothing; see [`wallet_funded`].
fn funding_grew_since_call(row: &QueuedSplice, channel: &Channel) -> Option<String> {
    let (Some(baseline), Some(value_before)) = (row.channel_funding_txo.as_deref(), row.channel_value_before) else {
        return None;
    };
    let outpoint = channel.funding_txo.as_ref()?;
    let current = format!("{}:{}", outpoint.txid, outpoint.vout);
    let grown = (row.amount_sats / 10 * 9).max(1);
    (current != baseline && channel.channel_value_sats >= value_before.saturating_add(grown))
        .then(|| outpoint.txid.clone())
}

/// Whether the node's wallet took part in the transaction, which it records as an outbound on-chain payment of its own as soon as it broadcasts a splice it contributed to, and never for a splice only the user funded. None when the node could not be asked.
async fn wallet_funded(ldk: &dyn LdkServerCalls, txid: &str, now: i64) -> Option<bool> {
    let payment_id = onchain_payment_id(txid)?;
    match timeout(rpc_timeout(), ldk.get_payment_details(GetPaymentDetailsRequest { payment_id })).await {
        Ok(Ok(response)) => {
            note_lookup_available(true, "", now);
            Some(response.payment.is_some_and(|payment| payment.direction == PaymentDirection::Outbound as i32))
        }
        Ok(Err(error)) => {
            note_lookup_available(false, &format!("wallet lookup for funding tx {txid} failed: {error}"), now);
            None
        }
        Err(_) => {
            note_lookup_available(false, &format!("wallet lookup for funding tx {txid} timed out"), now);
            None
        }
    }
}

/// What the channel says about a row's last call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockCheck {
    /// The row's own splice is on the channel; the row is locked.
    Locked,
    /// Nothing on the channel is the row's own splice.
    NotOurs,
    /// The channel moved, but the node could not say whether the LSP funded it; nothing may be assumed.
    Unknown,
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
    // A call the last request made may have locked since the job last looked; not one the node or the operator said is over, whose channel may since have been spliced by hand.
    match db.load_queued_splice(&key) {
        Ok(Some(existing)) if existing.channel_funding_txo.is_some() && existing.status != "locked" && !released_for_good(&existing) => {
            if lock_if_shown(db, ldk, &existing, channel, now).await == LockCheck::Unknown {
                return (
                    false,
                    "the node could not say whether the last splice-in call for this channel landed; try again once it answers".into(),
                );
            }
        }
        Ok(_) => {}
        Err(error) => return (false, format!("could not read the queued splice: {error}")),
    }
    // A failure event may have released the last call by mistake, and its signing can still revive the row inside the backoff; a replacement could not take that signing, so none goes out before then.
    match db.load_queued_splice(&key) {
        Ok(Some(existing)) if released_by_a_failure_event(&existing, now) => return (false, revivable_refusal(&existing, now)),
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
    match db.queue_splice_in(&key, counterparty, amount_sats, now, RETRY_BACKOFF_SECS, NEGOTIATION_FAILED_ERROR) {
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
        // The event landed while this request was away from the table, after its own check above.
        Ok(QueueSpliceOutcome::Revivable(row)) => (false, revivable_refusal(&row, now)),
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
    match db.cancel_queued_splice(&key, Some(reason), force, NEGOTIATION_FAILED_ERROR, now) {
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

/// Whether the last pass could not list the node's channels, so the next failure is not logged again.
static NODE_UNREACHABLE: AtomicBool = AtomicBool::new(false);
/// When wallet lookups started failing; 0 while they work. Logged once per episode, and a row kept from a new call this long stalls with an alert.
static LOOKUP_UNAVAILABLE_SINCE: AtomicI64 = AtomicI64::new(0);

/// Log a node failure once per episode, and the recovery once.
fn note_node_reachable(reachable: bool, error: &str) {
    let was_unreachable = NODE_UNREACHABLE.swap(!reachable, Ordering::SeqCst);
    if !reachable && !was_unreachable {
        warn!("[queued_splice] {}; the queue waits until the node answers again", error);
    } else if reachable && was_unreachable {
        info!("[queued_splice] the node answers again");
    }
}

/// Log a wallet lookup failure once per episode; a lock waits until the lookup works again.
fn note_lookup_available(available: bool, error: &str, now: i64) {
    if available {
        if LOOKUP_UNAVAILABLE_SINCE.swap(0, Ordering::SeqCst) != 0 {
            info!("[queued_splice] the node answers wallet lookups again");
        }
    } else if LOOKUP_UNAVAILABLE_SINCE.compare_exchange(0, now.max(1), Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        warn!("[queued_splice] {}; the lock is checked again next pass", error);
    }
}

/// How long wallet lookups have been failing; 0 while they work.
fn lookup_unavailable_for(now: i64) -> i64 {
    match LOOKUP_UNAVAILABLE_SINCE.load(Ordering::SeqCst) {
        0 => 0,
        since => now.saturating_sub(since),
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
        Ok(Ok(response)) => {
            note_node_reachable(true, "");
            response.channels
        }
        Ok(Err(error)) => {
            note_node_reachable(false, &format!("list_channels failed: {error}"));
            return;
        }
        Err(_) => {
            note_node_reachable(false, "list_channels timed out");
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
            // The node's list is complete, so a channel missing from it is gone; but the list is not the node's word on a call, so one that is pending stays so until it locks or the operator releases it.
            end_for_channel(db, &row, "the node no longer lists this channel", false, now);
            continue;
        };
        if row.pending_on_node {
            follow_pending_call(db, ldk, push, &row, channel, now, clock).await;
            continue;
        }
        // The node answered the last call, but the splice it reported dropped or refused may be the one on the channel now, so look before any new call.
        if row.channel_funding_txo.is_some() {
            match lock_if_shown(db, ldk, &row, channel, now).await {
                LockCheck::Locked => continue,
                LockCheck::Unknown => {
                    hold_for_lookup(db, &row, now);
                    continue;
                }
                LockCheck::NotOurs => {}
            }
        }
        let usable = channel.is_usable;
        match row.status.as_str() {
            "queued" | "failed" | "stalled" => {
                if row.status != "stalled" && now >= row.last_attempt_at.unwrap_or(row.created_at) + QUEUED_ALERT_SECS {
                    stall_uncalled(db, &row, now);
                    continue;
                }
                if row.status != "queued" && now < row.last_attempt_at.unwrap_or(0) + RETRY_BACKOFF_SECS {
                    continue;
                }
                if row.status == "stalled" {
                    // No more pushes for a stalled row, but the splice still runs when the phone shows up.
                    if usable {
                        start_splice(db, ldk, &row, channel, now, false).await;
                    }
                } else {
                    wake_or_splice(db, ldk, push, &row, channel, now, clock).await;
                }
            }
            "waking" => {
                let sent_at = row.wake_sent_at.unwrap_or(row.updated_at);
                if usable && now >= sent_at + WAKE_GRACE_SECS {
                    start_splice(db, ldk, &row, channel, now, true).await;
                }
                // Still waking past the timeout: the phone never came, or no call could be made while it was here. The failure applies only to a row still waking, so a call just made is not failed.
                if now >= sent_at + WAKE_TIMEOUT_SECS {
                    let error = if usable {
                        format!("no splice-in call could be made within {WAKE_TIMEOUT_SECS}s of the push")
                    } else {
                        format!("the phone did not come online within {WAKE_TIMEOUT_SECS}s of the push")
                    };
                    fail(db, &row, &error, now);
                }
            }
            other => warn!("[queued_splice] {} is {:?} with no call pending", row.user_channel_id, other),
        }
    }
}

/// A row whose call the node has not answered: lock it when the channel shows the splice, otherwise keep waking the phone, which is all the node needs to go on, and alert the operator once it has waited too long. A stalled row is left to the operator: the lock is still caught here, but the phone is not pushed for it any more.
async fn follow_pending_call(
    db: &Database,
    ldk: &dyn LdkServerCalls,
    push: &Arc<Mutex<PushService>>,
    row: &QueuedSplice,
    channel: &Channel,
    now: i64,
    clock: &(dyn Fn() -> i64 + Sync),
) {
    if lock_if_shown(db, ldk, row, channel, now).await == LockCheck::Locked
        || matches!(row.status.as_str(), "cancelled" | "stalled")
    {
        return;
    }
    let called_at = row.last_attempt_at.unwrap_or(row.updated_at);
    let alert = match row.funding_txo.as_deref() {
        Some(txo) if now >= row.negotiated_at.unwrap_or(called_at) + NEGOTIATED_ALERT_SECS => Some(format!(
            "a splice was signed ({txo}) but has not locked within {NEGOTIATED_ALERT_SECS}s; it locks once its transaction has confirmed and the phone is online"
        )),
        None if now >= called_at + PENDING_ALERT_SECS => Some(format!(
            "the node has not reported the splice-in call within {PENDING_ALERT_SECS}s; it may still be pending there, or may never have reached it. Cancel with force once you have checked that the node holds no splice for this channel"
        )),
        _ => None,
    };
    if let Some(error) = alert {
        match db.stall_queued_splice_pending(&row.user_channel_id, row.request_id, &error, now) {
            Ok(Some(stalled)) => audit(
                "QUEUED_SPLICE_STALLED",
                &stalled,
                &[("dedup_key", dedup_key("stalled", &stalled, format_args!("pending:{called_at}")))],
            ),
            Ok(None) => {}
            Err(error) => db_failed("stall_queued_splice_pending", &row.user_channel_id, error),
        }
        return;
    }
    let since = row.wake_sent_at.unwrap_or(0).max(called_at);
    if now >= since + PENDING_WAKE_SECS {
        wake_again(db, push, row, now, clock).await;
    }
}

/// The channel moved since the row's last call, but the node cannot say whether that was the LSP's own splice: no new call goes out, and once that has lasted as long as an unanswered call may, the row stalls so the operator hears of it, whether it was waiting for its retry or already pushed for it.
fn hold_for_lookup(db: &Database, row: &QueuedSplice, now: i64) {
    let note = "the node could not say whether the earlier splice-in landed; no new call until it answers";
    if let Err(error) = db.note_queued_splice_wake_issue(&row.user_channel_id, row.request_id, note, now) {
        db_failed("note_queued_splice_wake_issue", &row.user_channel_id, error);
    }
    if row.status == "stalled" || lookup_unavailable_for(now) < PENDING_ALERT_SECS {
        return;
    }
    let error = format!("{note}; it has not answered for {PENDING_ALERT_SECS}s");
    match db.stall_queued_splice(&row.user_channel_id, row.request_id, &error, now) {
        Ok(Some(stalled)) => audit("QUEUED_SPLICE_STALLED", &stalled, &[("dedup_key", dedup_key("stalled", &stalled, format_args!("lookup:{}", since_call(row))))]),
        Ok(None) => {}
        Err(error) => db_failed("stall_queued_splice", &row.user_channel_id, error),
    }
}

/// When the row's last call was made, or when it was queued if none was: what a ledger row for the row's state since then is keyed on, so the same state after a later call gets its own row.
fn since_call(row: &QueuedSplice) -> i64 {
    row.last_attempt_at.unwrap_or(row.created_at)
}

/// A row that gets no call for a day, since it was queued or since its last attempt, stalls, so the operator hears about a phone that never comes online.
fn stall_uncalled(db: &Database, row: &QueuedSplice, now: i64) {
    let since = if row.last_attempt_at.is_some() { "the last attempt" } else { "queueing" };
    let error = match row.wake_note.as_deref() {
        Some(note) => format!("no splice-in call could be made within {QUEUED_ALERT_SECS}s of {since}: {note}"),
        None => format!("no splice-in call could be made within {QUEUED_ALERT_SECS}s of {since}"),
    };
    match db.stall_queued_splice(&row.user_channel_id, row.request_id, &error, now) {
        Ok(Some(stalled)) => audit("QUEUED_SPLICE_STALLED", &stalled, &[("dedup_key", dedup_key("stalled", &stalled, format_args!("queued:{}", since_call(row))))]),
        Ok(None) => {}
        Err(error) => db_failed("stall_queued_splice", &row.user_channel_id, error),
    }
}

/// Lock the row when the channel shows its own splice completed: the funding grew since its call, and the node's wallet funded the new transaction. The lock is keyed to the request that was read, so one the operator replaced during the wallet lookup is left to its own pass.
async fn lock_if_shown(db: &Database, ldk: &dyn LdkServerCalls, row: &QueuedSplice, channel: &Channel, now: i64) -> LockCheck {
    let Some(txid) = funding_grew_since_call(row, channel) else {
        return LockCheck::NotOurs;
    };
    match wallet_funded(ldk, &txid, now).await {
        Some(true) => {}
        Some(false) => return LockCheck::NotOurs,
        None => return LockCheck::Unknown,
    }
    let current = channel_funding_txo(channel).unwrap_or_default();
    match db.mark_queued_splice_locked(&row.user_channel_id, row.request_id, &current, now) {
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
        // Already locked, or replaced since it was read: either way the splice the channel shows is not for a new call.
        Ok(None) => {}
        Err(error) => db_failed("mark_queued_splice_locked", &row.user_channel_id, error),
    }
    LockCheck::Locked
}

/// Push the phone before anything else, so the app can hold its node; when no push can go out now and the phone is online anyway, splice while it is here.
async fn wake_or_splice(
    db: &Database,
    ldk: &dyn LdkServerCalls,
    push: &Arc<Mutex<PushService>>,
    row: &QueuedSplice,
    channel: &Channel,
    now: i64,
    clock: &(dyn Fn() -> i64 + Sync),
) {
    if send_push(db, push, row, now).await {
        // The provider took its time; the phone's window starts now, not when the pass began.
        match db.mark_queued_splice_waking(&row.user_channel_id, row.request_id, clock()) {
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
async fn wake_again(db: &Database, push: &Arc<Mutex<PushService>>, row: &QueuedSplice, now: i64, clock: &(dyn Fn() -> i64 + Sync)) {
    if !send_push(db, push, row, now).await {
        return;
    }
    match db.note_queued_splice_wake_sent(&row.user_channel_id, row.request_id, clock()) {
        Ok(Some(woken)) => audit("QUEUED_SPLICE_WAKE_SENT", &woken, &[("timeout_secs", json!(PENDING_WAKE_SECS))]),
        Ok(None) => {}
        Err(error) => db_failed("note_queued_splice_wake_sent", &row.user_channel_id, error),
    }
}

/// True when the provider accepted a push for the row; otherwise the reason is noted on the row.
async fn send_push(db: &Database, push: &Arc<Mutex<PushService>>, row: &QueuedSplice, now: i64) -> bool {
    let node_id = row.counterparty.as_str();
    let (stability_due, has_token, may_push) = {
        let service = push.lock().await;
        (service.stability_wake_due(node_id), service.has_token(node_id), service.should_notify(node_id))
    };
    let note = if stability_due {
        "a stability wake is due for this phone and takes its push; the splice runs once the channel is usable"
    } else if !has_token {
        "no usable push token for this phone (none registered, or older than 30 days)"
    } else if !may_push {
        "waiting for the phone's push cooldown or backoff"
    } else if crate::push::notify(push, node_id, LSP_SPLICE_IN_DIRECTION).await {
        return true;
    } else {
        "the push provider did not accept the push"
    };
    if let Err(error) = db.note_queued_splice_wake_issue(&row.user_channel_id, row.request_id, note, now) {
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
    // A row with an earlier call behind it: the listing this pass began with may predate that call's lock, so a fresh listing is checked right before a new call.
    let fresh_channel: Channel;
    let channel = if row.channel_funding_txo.is_some() {
        let channels = match timeout(rpc_timeout(), ldk.list_channels(ListChannelsRequest {})).await {
            Ok(Ok(response)) => response.channels,
            Ok(Err(error)) => {
                warn!("[queued_splice] list_channels failed before the retry: {}; the next pass asks again", error);
                return;
            }
            Err(_) => {
                warn!("[queued_splice] list_channels timed out before the retry; the next pass asks again");
                return;
            }
        };
        let found = channels.into_iter().find(|c| {
            row_key(&c.user_channel_id).as_deref() == Some(row.user_channel_id.as_str()) && c.counterparty_node_id == row.counterparty
        });
        // Gone since the pass began: the next pass ends the row.
        let Some(found) = found else { return };
        fresh_channel = found;
        match lock_if_shown(db, ldk, row, &fresh_channel, now).await {
            LockCheck::Locked => return,
            LockCheck::Unknown => {
                hold_for_lookup(db, row, now);
                return;
            }
            LockCheck::NotOurs => {}
        }
        if !fresh_channel.is_usable {
            return;
        }
        &fresh_channel
    } else {
        channel
    };
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
                    ("dedup_key", dedup_key("started", &attempt, since_call(&attempt))),
                ],
            );
        }
        // The answer was lost in transport, or the request never arrived; either way the node may hold the call, so it stays pending until the node or the operator says otherwise.
        Ok(Err(error)) if matches!(error.error_code, LdkServerErrorCode::InternalError) => {
            warn!("[queued_splice] splice_in answer lost for {}: {}", attempt.user_channel_id, error);
        }
        // The node refused, so nothing went to the phone.
        Ok(Err(error)) => record_refusal(db, &attempt, &format!("splice_in: {}", error.message), now),
        Err(_) => warn!("[queued_splice] splice_in timed out for {}; waiting for the node's events", attempt.user_channel_id),
    }
}

/// The node refused the call. Normally that is a failed attempt, also when someone else's splice was negotiated on the channel while the call was out and moved the row on; when the row was cancelled meanwhile, the refusal still means nothing is pending.
fn record_refusal(db: &Database, attempt: &QueuedSplice, error: &str, now: i64) {
    if fail(db, attempt, error, now) || fail_from(db, attempt, "negotiated", error, now) {
        return;
    }
    match db.clear_queued_splice_pending(&attempt.user_channel_id, attempt.request_id, error, now) {
        Ok(_) => {}
        Err(error) => db_failed("clear_queued_splice_pending", &attempt.user_channel_id, error),
    }
}

/// Count a failed attempt on the row as it was read, and alert the operator when the row stalls on it. The node answered, or no call was made, so nothing of the row is pending. False when the row was no longer in that state.
fn fail(db: &Database, row: &QueuedSplice, error: &str, now: i64) -> bool {
    fail_from(db, row, &row.status, error, now)
}

/// [`fail`] for the row's request while it is in `from_status` rather than the state it was read in.
fn fail_from(db: &Database, row: &QueuedSplice, from_status: &str, error: &str, now: i64) -> bool {
    match db.fail_queued_splice(&row.user_channel_id, row.request_id, from_status, error, now, MAX_ATTEMPTS) {
        Ok(Some(failed)) => {
            // A stalled row's later tries keep the row's own error current but write no more ledger rows.
            if failed.attempts <= MAX_ATTEMPTS {
                audit("QUEUED_SPLICE_FAILED", &failed, &[("dedup_key", dedup_key("failed", &failed, failed.attempts))]);
            }
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
            true
        }
        Ok(None) => false,
        Err(error) => {
            db_failed("fail_queued_splice", &row.user_channel_id, error);
            false
        }
    }
}

/// The channel is gone, so the row is cancelled. The node's closed event means nothing on it can be pending or lock, and `release` then clears the call; a listing without the channel is not the node's word on the call, so the call stays pending until it locks or the operator releases it.
fn end_for_channel(db: &Database, row: &QueuedSplice, reason: &str, release: bool, now: i64) {
    match db.close_queued_splice_channel(&row.user_channel_id, reason, release, now) {
        Ok(Some(ended)) if row.status != "cancelled" => {
            audit("QUEUED_SPLICE_CANCELLED", &ended, &[("dedup_key", dedup_key("cancelled", &ended, "channel"))]);
        }
        Ok(_) => {}
        Err(error) => db_failed("close_queued_splice_channel", &row.user_channel_id, error),
    }
}

/// The channel's row, for an event about the channel; None when there is none, or the read failed (which is audited).
fn row_for_event(db: &Database, key: &str) -> Option<QueuedSplice> {
    match db.load_queued_splice(key) {
        Ok(row) => row,
        Err(error) => {
            audit_event("DB_READ_FAILED", json!({ "op": "load_queued_splice", "user_channel_id": key, "error": error.to_string() }));
            None
        }
    }
}

/// Whether the row's last call was released by a failure event so recently that a negotiation on the channel may still be that call's: the row still shows the event (it fails a splicing or stalled row and clears a cancelled one, with the same text and time), and no retry can have gone out inside the backoff. Until then no request replaces the row either, since a replacement could not take the signing.
fn released_by_a_failure_event(row: &QueuedSplice, now: i64) -> bool {
    matches!(row.status.as_str(), "failed" | "stalled" | "cancelled")
        && row.last_error.as_deref() == Some(NEGOTIATION_FAILED_ERROR)
        && row.last_attempt_at.is_some_and(|failed_at| now < failed_at + RETRY_BACKOFF_SECS)
}

/// Why a request is refused while a signing could still revive the row's last call.
fn revivable_refusal(row: &QueuedSplice, now: i64) -> String {
    format!(
        "the node reported the last splice-in call's negotiation failed {}s ago; a new request can replace it once {RETRY_BACKOFF_SECS}s have passed, in case that call's splice is signed after all",
        now - row.last_attempt_at.unwrap_or(now)
    )
}

/// A cancelled row whose call is over on the node's or the operator's word: the node refused it, or the operator cancelled after a refusal or with force, which left a text other than the failure event's. Nothing of it can still lock, so a splice the channel shows later is someone else's, the operator's own by hand included, and is not taken for the row's.
fn released_for_good(row: &QueuedSplice) -> bool {
    row.status == "cancelled" && !row.pending_on_node && row.last_error.as_deref() != Some(NEGOTIATION_FAILED_ERROR)
}

/// The node negotiated a splice on the channel. Every row with a call pending records it, whatever its status, since a signed splice keeps the call through a later negotiation failure; whether the splice was the row's own is settled at the lock. A row a failure event released this side of the retry backoff is made pending again, since the event may have been for someone else's splice.
pub(crate) fn on_splice_negotiated(db: &Database, user_channel_id: &str, funding_txo: &str, now: i64) {
    let Some(key) = row_key(user_channel_id) else { return };
    let Some(row) = row_for_event(db, &key) else { return };
    let (op, result) = if row.pending_on_node {
        ("note_queued_splice_negotiated", db.note_queued_splice_negotiated(&key, row.request_id, funding_txo, now))
    } else if released_by_a_failure_event(&row, now) {
        ("revive_queued_splice_negotiated", db.revive_queued_splice_negotiated(&key, row.request_id, funding_txo, NEGOTIATION_FAILED_ERROR, now))
    } else {
        return;
    };
    match result {
        Ok(Some(noted)) => audit("QUEUED_SPLICE_NEGOTIATED", &noted, &[("revived", json!(!row.pending_on_node))]),
        Ok(None) => {}
        Err(error) => db_failed(op, &key, error),
    }
}

/// The node dropped the splice it was negotiating on the channel. A row whose call is out and whose splice was never signed goes back for a retry, and nothing of it is pending any more. A row that already holds a signed splice keeps it and its pending call: the dropped splice may be someone else's, and once a later splice has replaced the signed one on the channel, a check against the channel's current funding could not tell that a second call would fund it twice. Such a row locks when the channel shows its splice, or alerts the operator [`NEGOTIATED_ALERT_SECS`] after the signing.
pub(crate) fn on_splice_negotiation_failed(db: &Database, user_channel_id: &str, now: i64) {
    let Some(key) = row_key(user_channel_id) else { return };
    let Some(row) = row_for_event(db, &key) else { return };
    if !row.pending_on_node || row.funding_txo.is_some() {
        return;
    }
    // A call the node answered is a failed attempt, whether the row was waiting for the answer or had stalled waiting; a cancelled row only loses its pending call. Each keeps the event's text and time, so a signing inside the backoff can still revive the call.
    if matches!(row.status.as_str(), "splicing" | "stalled") {
        fail(db, &row, NEGOTIATION_FAILED_ERROR, now);
    } else if let Err(error) = db.clear_queued_splice_pending(&key, row.request_id, NEGOTIATION_FAILED_ERROR, now) {
        db_failed("clear_queued_splice_pending", &key, error);
    }
}

/// The channel closed, so nothing can be spliced into it.
pub(crate) fn on_channel_closed(db: &Database, user_channel_id: &str, now: i64) {
    let Some(key) = row_key(user_channel_id) else { return };
    let Some(row) = row_for_event(db, &key) else { return };
    end_for_channel(db, &row, "the channel closed", true, now);
}

/// The channel's queued splice while the job may still call for it, or a splice of its own may still land: any row but a locked one or a cancelled one, since the job retries a failed or stalled row too, plus a cancelled row whose last call may still be pending on the node or that the node's failure event released, since a signing can revive such a call inside the backoff and its splice is still looked for at the next request after it; only a forced cancel ends that. A manual splice-in then would be a second splice for the request, or be taken for its lock, so the operator route refuses one; an unreadable queue counts as open.
pub(crate) fn open_row(db: &Database, user_channel_id: &str, now: i64) -> Result<Option<QueuedSplice>, rusqlite::Error> {
    let Some(key) = row_key(user_channel_id) else { return Ok(None) };
    Ok(db.load_queued_splice(&key)?.filter(|row| match row.status.as_str() {
        "locked" => false,
        "cancelled" => row.pending_on_node || released_by_a_failure_event(row, now) || row.last_error.as_deref() == Some(NEGOTIATION_FAILED_ERROR),
        _ => true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::testing::{service, FakeSender};
    use crate::stable_manager::tests::{
        make_channel, FakeLdkServer, AUDIT_TEST_GUARD, CHANNEL_ID_HEX, COUNTERPARTY_HEX,
        USER_CHANNEL_ID_DECIMAL,
    };
    use ldk_server_client::ldk_server_grpc::types::{
        payment_kind, Onchain, OutPoint, Payment as GrpcPayment, PaymentKind, PaymentStatus,
    };
    use std::sync::atomic::{AtomicI64, Ordering};

    const FUNDED_SATS: u64 = 1_000_000;
    const AMOUNT_SATS: u64 = 50_000;
    /// The LSP's side of the test channel before any splice.
    const LSP_SATS: u64 = 60_000;
    /// The user's side of the test channel, which only a user's splice changes; larger than the amount so a payment of the amount can be shown.
    const USER_SATS: u64 = 140_000;
    /// The channel's funding before any splice, and the transactions of later splices.
    const TX_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TX_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TX_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn txo(txid: &str, vout: u32) -> String {
        format!("{txid}:{vout}")
    }

    /// A channel whose value is the two sides added up.
    fn channel_sides(usable: bool, txid: &str, vout: u32, lsp_sats: u64, user_sats: u64) -> Channel {
        let mut channel = make_channel(CHANNEL_ID_HEX, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, lsp_sats + user_sats, lsp_sats * 1_000, usable);
        channel.funding_txo = Some(OutPoint { txid: txid.into(), vout });
        channel
    }

    fn channel_with(usable: bool, txid: &str, vout: u32, lsp_sats: u64) -> Channel {
        channel_sides(usable, txid, vout, lsp_sats, USER_SATS)
    }

    fn channel(usable: bool, txid: &str, vout: u32) -> Channel {
        channel_with(usable, txid, vout, LSP_SATS)
    }

    fn node(usable: bool) -> FakeLdkServer {
        let fake = FakeLdkServer::new(vec![channel(usable, TX_A, 0)]);
        fake.spendable_onchain_sats.store(FUNDED_SATS, Ordering::SeqCst);
        fake
    }

    /// The node's wallet took part in `txid`: ldk-node records such a splice as an outbound on-chain payment of its own when it broadcasts it.
    fn wallet_funded_tx(fake: &FakeLdkServer, txid: &str) {
        fake.payments.lock().unwrap().push(GrpcPayment {
            payment_id: onchain_payment_id(txid).unwrap(),
            kind: Some(PaymentKind {
                kind: Some(payment_kind::Kind::Onchain(Onchain { txid: txid.into(), status: None, tx_type: None })),
            }),
            amount_msat: Some(AMOUNT_SATS * 1_000),
            direction: PaymentDirection::Outbound as i32,
            status: PaymentStatus::Pending as i32,
            ..Default::default()
        });
    }

    /// The tests share the audit capture, so they run one at a time; a test that failed must not fail the rest through a poisoned lock.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        AUDIT_TEST_GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn database() -> Database {
        // The job's episode markers are process-wide: a test starts with neither episode open.
        LOOKUP_UNAVAILABLE_SINCE.store(0, Ordering::SeqCst);
        NODE_UNREACHABLE.store(false, Ordering::SeqCst);
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        std::mem::forget(dir);
        db
    }

    fn queued(db: &Database, now: i64) -> QueuedSplice {
        match db.queue_splice_in(USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, now, RETRY_BACKOFF_SECS, NEGOTIATION_FAILED_ERROR).unwrap() {
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
        let _guard = guard();
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
            (splicing.status.as_str(), splicing.channel_funding_txo, splicing.channel_value_before, splicing.lsp_sats_before, splicing.pending_on_node),
            ("splicing", Some(txo(TX_A, 0)), Some(LSP_SATS + USER_SATS), Some(LSP_SATS), true)
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
        let _guard = guard();
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
        let _guard = guard();
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
        assert_eq!((waiting.status.as_str(), waiting.wake_note.as_deref()), ("failed", Some("waiting for the phone's push cooldown or backoff")));
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
    async fn a_row_that_never_gets_a_call_stalls_after_a_day_and_is_still_tried_when_the_phone_shows_up() {
        let _guard = guard();
        let db = database();
        let ldk = node(false);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        queued(&db, 100);
        stable_channels::audit::enable_test_capture();

        // No token and no phone: the row waits, with the reason noted.
        pass(&db, &ldk, &push, 100).await;
        pass(&db, &ldk, &push, 100 + QUEUED_ALERT_SECS - 1).await;
        let waiting = row(&db);
        assert_eq!(waiting.status, "queued");
        assert!(waiting.wake_note.as_deref().unwrap().contains("no usable push token"));
        pass(&db, &ldk, &push, 100 + QUEUED_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.attempts, stalled.pending_on_node), ("stalled", 0, false));
        assert!(stalled.last_error.as_deref().unwrap().contains("no usable push token"));
        // The phone shows up a day later: the splice still runs.
        *ldk.channels.lock().unwrap() = vec![channel(true, TX_A, 0)];
        pass(&db, &ldk, &push, 100 + 2 * QUEUED_ALERT_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!(row(&db).status, "splicing");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_STARTED"]);
        assert!(events[0].1.get("status").is_none(), "a row waiting for its phone has not failed");
    }

    #[tokio::test]
    async fn a_node_refusal_counts_an_attempt_and_the_row_stalls_at_the_cap() {
        let _guard = guard();
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
        // A stalled row is not pushed for, but the phone being online still gets it a try after the backoff.
        pass(&db, &ldk, &push, now - 1).await;
        assert_eq!(row(&db).attempts, MAX_ATTEMPTS);
        pass(&db, &ldk, &push, now).await;
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("stalled", MAX_ATTEMPTS + 1));
        let events = capture();
        let stalled: Vec<_> = events.iter().filter(|(name, _)| name == "QUEUED_SPLICE_STALLED").collect();
        assert_eq!(stalled.len(), 1, "the operator is alerted once");
        assert_eq!(stalled[0].1["max_attempts"], MAX_ATTEMPTS);
        assert_eq!(stalled[0].1["status"], "failed");
        assert_eq!(events.iter().filter(|(name, _)| name == "QUEUED_SPLICE_FAILED").count(), MAX_ATTEMPTS as usize, "tries past the cap write no ledger rows");
    }

    #[tokio::test]
    async fn a_failing_wallet_lookup_makes_no_new_call_refuses_a_new_request_and_alerts_in_time() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        // The node reports a failure, so the call is no longer pending, but the channel then shows a splice that may be ours.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        ldk.payment_lookup_failures.store(1_000, Ordering::SeqCst);
        stable_channels::audit::enable_test_capture();

        let retry_at = 101 + RETRY_BACKOFF_SECS;
        pass(&db, &ldk, &push, retry_at).await;
        let held = row(&db);
        assert_eq!((held.status.as_str(), held.attempts), ("failed", 1));
        assert!(held.wake_note.as_deref().unwrap().contains("could not say"));
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no new call while the node cannot say");
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, retry_at + 1).await;
        assert!(!ok && status.contains("could not say"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).amount_sats), ("failed", AMOUNT_SATS), "the row is not replaced");
        // Kept from a call for as long as an unanswered call may wait, the row stalls so the operator hears of it.
        pass(&db, &ldk, &push, retry_at + PENDING_ALERT_SECS - 1).await;
        assert_eq!(row(&db).status, "failed");
        pass(&db, &ldk, &push, retry_at + PENDING_ALERT_SECS).await;
        assert_eq!(row(&db).status, "stalled");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        // Once the node answers, the splice turns out to be ours: locked, still one call.
        ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, retry_at + PENDING_ALERT_SECS + RETRY_BACKOFF_SECS).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_LOCKED"]);
        assert!(events[0].1.get("status").is_none());
        assert!(events[0].1["error"].as_str().unwrap().contains("not answered"));
    }

    #[tokio::test]
    async fn a_pending_row_with_a_failing_lookup_still_reaches_its_alert() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        ldk.payment_lookup_failures.store(1_000, Ordering::SeqCst);
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, 100 + PENDING_ALERT_SECS - 1).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("splicing", true));
        pass(&db, &ldk, &push, 100 + PENDING_ALERT_SECS).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("stalled", true), "no silent freeze");
        ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 100 + PENDING_ALERT_SECS + 5).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn a_row_pushed_for_a_retry_and_held_by_a_failing_lookup_stalls_with_an_alert() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        pass(&db, &ldk, &push, 100 + WAKE_GRACE_SECS).await;
        assert_eq!(row(&db).status, "splicing");
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 104);
        // The phone has gone by the retry, so it is pushed again.
        *ldk.channels.lock().unwrap() = vec![channel(false, TX_A, 0)];
        push.lock().await.forget_cooldowns();
        let pushed_at = 104 + RETRY_BACKOFF_SECS;
        pass(&db, &ldk, &push, pushed_at).await;
        assert_eq!((row(&db).status.as_str(), row(&db).wake_sent_at), ("waking", Some(pushed_at)));
        // The channel then moves while the node cannot say whose splice that was.
        *ldk.channels.lock().unwrap() = vec![channel_with(false, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        ldk.payment_lookup_failures.store(1_000, Ordering::SeqCst);
        stable_channels::audit::enable_test_capture();

        let held_from = pushed_at + 1;
        pass(&db, &ldk, &push, held_from).await;
        let held = row(&db);
        assert_eq!((held.status.as_str(), held.attempts), ("waking", 1));
        assert!(held.wake_note.as_deref().unwrap().contains("could not say"));
        // Not timed out as a wake either: the hold comes first, and it is not a failure.
        pass(&db, &ldk, &push, pushed_at + WAKE_TIMEOUT_SECS).await;
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("waking", 1));
        pass(&db, &ldk, &push, held_from + PENDING_ALERT_SECS - 1).await;
        assert_eq!(row(&db).status, "waking");
        pass(&db, &ldk, &push, held_from + PENDING_ALERT_SECS).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("stalled", false), "no silent wait");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no new call");
        // Once the node answers, the splice turns out to be ours.
        ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, held_from + PENDING_ALERT_SECS + 1).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_LOCKED"]);
        assert!(events[0].1["error"].as_str().unwrap().contains("not answered"));
    }

    #[tokio::test]
    async fn a_lock_found_for_a_request_replaced_during_the_lookup_leaves_the_replacement_alone() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        let stale = row(&db);
        assert_eq!(stale.status, "failed");
        // The channel then shows the call's splice after all. While a pass holds the failed row for its wallet lookup, the operator's new request locks the row itself and replaces it.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 102).await;
        assert!(ok, "{status}");
        let replacement = row(&db);
        assert_eq!((replacement.status.as_str(), replacement.amount_sats), ("queued", AMOUNT_SATS * 2));
        assert_ne!(replacement.request_id, stale.request_id);
        stable_channels::audit::enable_test_capture();

        // The pass's lookup lands: the lock was the old request's, so the new one is not marked locked for a splice it never made.
        let shown = channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS);
        assert_eq!(lock_if_shown(&db, &ldk, &stale, &shown, 103).await, LockCheck::Locked);
        let untouched = row(&db);
        assert_eq!((untouched.request_id, untouched.status.as_str(), untouched.funding_txo), (replacement.request_id, "queued", None));
        // The replacement gets its own call.
        pass(&db, &ldk, &push, 104).await;
        let calls = ldk.splice_ins.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].amount, Some(splice_in_request::Amount::SpliceAmountSats(AMOUNT_SATS * 2)));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_STARTED"]);
    }

    #[tokio::test]
    async fn a_refusal_after_someone_elses_negotiation_counts_the_attempt_and_the_row_is_retried() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        let request = queued(&db, 100);
        let attempt = db
            .mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request.request_id, Some(&txo(TX_A, 0)), LSP_SATS + USER_SATS, LSP_SATS, 101)
            .unwrap()
            .unwrap();
        // While the call is out, the user's own splice is negotiated on the channel, and the node then refuses ours for it.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 102);
        assert_eq!(row(&db).status, "negotiated");
        stable_channels::audit::enable_test_capture();
        record_refusal(&db, &attempt, "splice_in: a splice is already in progress", 103);
        let failed = row(&db);
        assert_eq!(
            (failed.status.as_str(), failed.attempts, failed.pending_on_node, failed.last_error.as_deref()),
            ("failed", 1, false, Some("splice_in: a splice is already in progress"))
        );
        assert!(!failed.is_active(), "a new request is not refused for it");
        // The user's splice locks; it is not ours, and the retry goes out after the backoff.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_B, 1, LSP_SATS, USER_SATS + AMOUNT_SATS)];
        pass(&db, &ldk, &push, 103 + RETRY_BACKOFF_SECS).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("splicing", true));
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_STARTED"]);
    }

    #[tokio::test]
    async fn a_due_stability_wake_takes_the_push_and_the_splice_runs_once_the_channel_is_usable() {
        let _guard = guard();
        let db = database();
        let ldk = node(false);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        push.lock().await.set_stability_wakes_due(vec![COUNTERPARTY_HEX.to_string()]);
        queued(&db, 100);
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, 100).await;
        assert!(sender.sent().is_empty(), "the splice job yields the push slot");
        let waiting = row(&db);
        assert_eq!(waiting.status, "queued");
        assert!(waiting.wake_note.as_deref().unwrap().contains("stability wake"));
        // The stability wake brought the phone online: the splice runs without a push of its own.
        *ldk.channels.lock().unwrap() = vec![channel(true, TX_A, 0)];
        pass(&db, &ldk, &push, 105).await;
        assert!(sender.sent().is_empty());
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!(row(&db).status, "splicing");
        // With no wake due, the next request is pushed for as usual.
        push.lock().await.set_stability_wakes_due(Vec::<String>::new());
        db.cancel_queued_splice(USER_CHANNEL_ID_DECIMAL, None, true, NEGOTIATION_FAILED_ERROR, 106).unwrap();
        queued(&db, 107);
        pass(&db, &ldk, &push, 107).await;
        assert_eq!(sender.sent().len(), 1);
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_STARTED", "QUEUED_SPLICE_WAKE_SENT"]);
    }

    #[tokio::test]
    async fn a_lost_answer_keeps_the_call_pending_until_the_channel_shows_it_locked() {
        let _guard = guard();
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
        // Once stalled, the phone is left alone too: no more pushes, and never another call.
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS + 3_600).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty(), "a pending call is never followed by another");
        assert_eq!(sender.sent().len(), 2, "a stalled pending row is not pushed for");
        // The channel then shows the splice locked: new funding the node's wallet took part in, and the value up by the amount.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS - 300)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS + 3_601).await;
        let locked = row(&db);
        assert_eq!((locked.status.as_str(), locked.funding_txo, locked.pending_on_node), ("locked", Some(txo(TX_B, 1)), false));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, called_at + PENDING_ALERT_SECS + 3_602).await;
        assert!(ok, "{status}");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_LOCKED", "QUEUED_SPLICE_CREATED"]);
        assert!(events[1].1.get("status").is_none(), "a pending call is unanswered, not failed");
        assert_eq!(events[2].1["status"], "completed");
        assert_eq!(events[2].1["lsp_sats_after"], LSP_SATS + AMOUNT_SATS - 300);
    }

    #[tokio::test]
    async fn the_operator_can_release_a_pending_call_after_checking_the_node() {
        let _guard = guard();
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
    async fn a_refusal_that_lands_after_a_cancel_still_clears_the_pending_call() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::InternalError, "gRPC request failed".into()));
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        let attempt = row(&db);
        // The operator cancels while the call is out; then the node's refusal arrives.
        cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 101);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", true));
        stable_channels::audit::enable_test_capture();
        record_refusal(&db, &attempt, "splice_in: no such peer", 102);
        let released = row(&db);
        assert_eq!((released.status.as_str(), released.pending_on_node, released.attempts), ("cancelled", false, 0));
        assert_eq!(released.last_error.as_deref(), Some("splice_in: no such peer"));
        assert!(capture().is_empty(), "a cancelled row counts no attempt");
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 1, 103).await;
        assert!(ok, "{status}");
    }

    #[tokio::test]
    async fn a_queue_request_locks_a_pending_call_the_channel_shows_completed_before_saving() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        stable_channels::audit::enable_test_capture();

        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 200).await;
        assert!(ok, "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).amount_sats), ("queued", AMOUNT_SATS * 2));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED", "QUEUED_SPLICE_CREATED"]);
    }

    #[tokio::test]
    async fn another_splice_locking_while_the_call_is_out_is_neither_this_rows_lock_nor_its_end() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 101);
        assert_eq!(row(&db).status, "negotiated");
        stable_channels::audit::enable_test_capture();

        // The user's splice-in locks: the funding moved and the value grew, but the node's wallet had no part in it.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_B, 1, LSP_SATS, USER_SATS + 80_000)];
        pass(&db, &ldk, &push, 102).await;
        let waiting = row(&db);
        assert_eq!((waiting.status.as_str(), waiting.attempts, waiting.pending_on_node), ("negotiated", 0, true));
        assert!(ldk.splice_ins.lock().unwrap().len() == 1, "no second call: the node may still run this row's splice after the user's");
        // Then the node runs this row's splice, which its wallet funds.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_C, 2, LSP_SATS + AMOUNT_SATS, USER_SATS + 80_000)];
        wallet_funded_tx(&ldk, TX_C);
        pass(&db, &ldk, &push, 103).await;
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo), ("locked", Some(txo(TX_C, 2))));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn a_users_splice_plus_a_payment_to_the_lsp_is_not_taken_as_the_lsps_lock() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        stable_channels::audit::enable_test_capture();

        // The user splices in and pays the LSP the amount: every balance moves as the LSP's own splice would move it.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_B, 1, LSP_SATS + AMOUNT_SATS, USER_SATS)];
        pass(&db, &ldk, &push, 101).await;
        pass(&db, &ldk, &push, 102).await;
        let waiting = row(&db);
        assert_eq!((waiting.status.as_str(), waiting.pending_on_node), ("splicing", true));
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second call either");
        assert!(capture().is_empty(), "nothing is recorded as locked");
        // Only the node's wallet taking part in the new funding attributes it.
        wallet_funded_tx(&ldk, TX_B);
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, 103).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn a_row_whose_call_was_reported_failed_is_locked_not_called_again_when_the_channel_shows_its_splice() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        // A negotiation failure on the channel is taken as this row's: the call is no longer pending.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        let failed = row(&db);
        assert_eq!((failed.status.as_str(), failed.attempts, failed.pending_on_node), ("failed", 1, false));
        assert!(failed.channel_funding_txo.is_some(), "the baseline of the call is kept");
        stable_channels::audit::enable_test_capture();

        // But the splice that failed was someone else's, and this row's call locks before the retry is due.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 101 + RETRY_BACKOFF_SECS).await;
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo), ("locked", Some(txo(TX_B, 1))));
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second splice-in");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn a_stability_payment_during_the_splice_does_not_hide_the_lock() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        stable_channels::audit::enable_test_capture();

        // The LSP paid the user a fifth of the amount while the splice was under way: its side grew by less than the amount.
        let paid = AMOUNT_SATS / 5;
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_B, 1, LSP_SATS + AMOUNT_SATS - paid, USER_SATS + paid)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 101).await;
        assert_eq!(row(&db).status, "locked");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_LOCKED"]);
        assert_eq!(events[0].1["lsp_sats_after"], LSP_SATS + AMOUNT_SATS - paid);
    }

    #[tokio::test]
    async fn a_negotiated_splice_waits_for_its_lock_wakes_the_phone_again_and_alerts_the_operator_on_the_scale_of_confirmations() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        pass(&db, &ldk, &push, 100 + WAKE_GRACE_SECS).await;
        let called_at = 100 + WAKE_GRACE_SECS;
        let signed_at = called_at + 1;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), signed_at);
        assert_eq!(row(&db).status, "negotiated");
        // The phone left after signing.
        *ldk.channels.lock().unwrap() = vec![channel(false, TX_A, 0)];
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, called_at + PENDING_WAKE_SECS - 1).await;
        assert_eq!(sender.sent().len(), 1);
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, called_at + PENDING_WAKE_SECS).await;
        let woken = row(&db);
        assert_eq!((woken.status.as_str(), woken.attempts, woken.wake_sent_at), ("negotiated", 0, Some(called_at + PENDING_WAKE_SECS)));
        assert_eq!(sender.sent().len(), 2);
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second call");
        // A signed splice waiting for confirmations is not an alert at the negotiation timeout.
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS).await;
        assert_eq!(row(&db).status, "negotiated");
        // The alert is timed from the signing, not from the call.
        pass(&db, &ldk, &push, called_at + NEGOTIATED_ALERT_SECS).await;
        assert_eq!(row(&db).status, "negotiated");
        pass(&db, &ldk, &push, signed_at + NEGOTIATED_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.pending_on_node), ("stalled", true));
        assert!(stalled.last_error.unwrap().contains("has not locked within"));
        // Back online and confirmed, the node finishes the splice.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, signed_at + NEGOTIATED_ALERT_SECS + 30).await;
        assert_eq!(row(&db).status, "locked");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_LOCKED"]);
        assert!(events[1].1.get("status").is_none(), "waiting for confirmations is not a failure");
    }

    #[tokio::test]
    async fn a_cancelled_row_whose_call_the_node_completes_is_recorded_as_locked() {
        let _guard = guard();
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

        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 103).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("locked", false));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CANCELLED", "QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn an_unfunded_wallet_fails_the_attempt_instead_of_calling_the_node() {
        let _guard = guard();
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
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let stale = queued(&db, 100);
        // The operator cancels and queues a different amount while a pass holds the old row.
        cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 101);
        db.queue_splice_in(USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 3, 102, RETRY_BACKOFF_SECS, NEGOTIATION_FAILED_ERROR).unwrap();
        stable_channels::audit::enable_test_capture();

        start_splice(&db, &ldk, &stale, &channel(true, TX_A, 0), 103, false).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty());
        assert_eq!((row(&db).status.as_str(), row(&db).amount_sats), ("queued", AMOUNT_SATS * 3));
        assert!(capture().is_empty());
    }

    #[tokio::test]
    async fn a_channel_the_node_no_longer_lists_ends_the_row_but_its_call_stays_pending_until_it_locks() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        *ldk.channels.lock().unwrap() = vec![];
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, 101).await;
        let ended = row(&db);
        assert_eq!((ended.status.as_str(), ended.pending_on_node, ended.lsp_sats_before), ("cancelled", true, None));
        assert!(ended.last_error.as_deref().unwrap().contains("no longer lists"));
        // Nothing more happens while the channel stays missing; the job keeps watching the row.
        pass(&db, &ldk, &push, 102).await;
        assert_eq!(db.list_open_queued_splices(102, CANCELLED_RECONCILE_SECS).unwrap().len(), 1);
        // The listing was incomplete: the channel is back with the call's splice, which locks instead of being made again.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 103).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("locked", false));
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        // A row with no call pending is ended outright.
        queued(&db, 104);
        *ldk.channels.lock().unwrap() = vec![];
        pass(&db, &ldk, &push, 105).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", false));
        assert!(db.list_open_queued_splices(105, CANCELLED_RECONCILE_SECS).unwrap().is_empty());
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CANCELLED", "QUEUED_SPLICE_LOCKED", "QUEUED_SPLICE_CANCELLED"]);
    }

    #[tokio::test]
    async fn node_events_move_the_row() {
        let _guard = guard();
        let db = database();
        let request_id = queued(&db, 100).request_id;
        stable_channels::audit::enable_test_capture();

        // Another splice on the channel, before this row's call is out, is not this row's.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_C, 0), 101);
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 102);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("queued", 0));

        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some(&txo(TX_A, 0)), LSP_SATS + USER_SATS, LSP_SATS, 103).unwrap();
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 104);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts, row(&db).pending_on_node), ("failed", 1, false));

        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some(&txo(TX_A, 0)), LSP_SATS + USER_SATS, LSP_SATS, 105).unwrap();
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 106);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts, row(&db).pending_on_node), ("failed", 2, false));

        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some(&txo(TX_A, 0)), LSP_SATS + USER_SATS, LSP_SATS, 107).unwrap();
        // The hex form of the same channel id finds the row too.
        let hex_id = format!("{:032x}", USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap());
        on_splice_negotiated(&db, &hex_id, &txo(TX_B, 1), 108);
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo, row(&db).negotiated_at), ("negotiated", Some(txo(TX_B, 1)), Some(108)));
        // A negotiation failing after the row's splice was signed is not taken as the row's: the signed splice may land, so the call stays pending.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 109);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts, row(&db).pending_on_node), ("negotiated", 2, true));

        on_channel_closed(&db, USER_CHANNEL_ID_DECIMAL, 110);
        assert_eq!((row(&db).status.as_str(), row(&db).last_error.as_deref()), ("cancelled", Some("the channel closed")));
        on_channel_closed(&db, USER_CHANNEL_ID_DECIMAL, 111);

        let events = capture();
        assert_eq!(
            names(&events),
            ["QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_NEGOTIATED", "QUEUED_SPLICE_CANCELLED"]
        );
        assert_ne!(events[0].1["dedup_key"], events[1].1["dedup_key"]);
    }

    #[tokio::test]
    async fn queue_checks_the_request_the_channel_and_the_wallet_before_saving() {
        let _guard = guard();
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
    fn a_lock_candidate_needs_new_funding_and_the_value_grown_by_most_of_the_amount() {
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
            negotiated_at: None,
            wake_sent_at: None,
            last_attempt_at: Some(100),
            channel_funding_txo: Some(txo(TX_A, 0)),
            channel_value_before: Some(LSP_SATS + USER_SATS),
            lsp_sats_before: Some(LSP_SATS),
            pending_on_node: true,
            created_at: 100,
            updated_at: 100,
        };
        let grown = AMOUNT_SATS / 10 * 9;
        assert_eq!(funding_grew_since_call(&row, &channel_with(true, TX_A, 0, LSP_SATS + AMOUNT_SATS)), None, "same funding: not locked yet");
        assert_eq!(funding_grew_since_call(&row, &channel_sides(true, TX_B, 1, LSP_SATS + AMOUNT_SATS, USER_SATS - AMOUNT_SATS)), None, "the value did not grow");
        assert_eq!(funding_grew_since_call(&row, &channel_with(true, TX_B, 1, LSP_SATS + grown - 1)), None, "grew by less than most of the amount");
        assert_eq!(funding_grew_since_call(&row, &channel_with(true, TX_B, 1, LSP_SATS + grown)), Some(TX_B.to_string()));
        assert_eq!(funding_grew_since_call(&row, &channel_sides(true, TX_B, 1, LSP_SATS, USER_SATS + AMOUNT_SATS)), Some(TX_B.to_string()), "a user's splice is a candidate too; the wallet check tells them apart");
        assert_eq!(funding_grew_since_call(&row, &channel_sides(true, TX_C, 2, LSP_SATS + AMOUNT_SATS, USER_SATS + 80_000)), Some(TX_C.to_string()), "the user's splice and this one");
        row.channel_value_before = None;
        assert_eq!(funding_grew_since_call(&row, &channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)), None, "no baseline, no candidate");
    }

    #[tokio::test]
    async fn only_a_transaction_the_wallet_funded_counts_as_the_lsps() {
        let _guard = guard();
        let _db = database();
        let ldk = node(true);
        assert_eq!(wallet_funded(&ldk, TX_B, 1).await, Some(false), "the wallet has no record of it");
        wallet_funded_tx(&ldk, TX_B);
        assert_eq!(wallet_funded(&ldk, TX_B, 1).await, Some(true));
        // A record the wallet only received on is not a splice it funded.
        ldk.payments.lock().unwrap().push(GrpcPayment {
            payment_id: onchain_payment_id(TX_C).unwrap(),
            direction: PaymentDirection::Inbound as i32,
            ..Default::default()
        });
        assert_eq!(wallet_funded(&ldk, TX_C, 1).await, Some(false));
        assert_eq!(wallet_funded(&ldk, "not-a-txid", 1).await, None);
        ldk.payment_lookup_failures.store(1, Ordering::SeqCst);
        assert_eq!(wallet_funded(&ldk, TX_B, 1).await, None, "a node that cannot be asked attributes nothing");
        assert_eq!(wallet_funded(&ldk, TX_B, 1).await, Some(true));
    }

    #[tokio::test]
    async fn a_negotiation_failing_after_the_rows_splice_was_signed_makes_no_second_call_even_once_a_later_splice_hides_it() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        let signed_at = 101;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), signed_at);
        // The row's splice confirms, and a splice of the user's own replaces it on the channel before any pass sees it; then a negotiation on the channel fails.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_C, 0, LSP_SATS + AMOUNT_SATS, USER_SATS + 80_000)];
        wallet_funded_tx(&ldk, TX_B);
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 102);
        let kept = row(&db);
        assert_eq!((kept.status.as_str(), kept.attempts, kept.pending_on_node, kept.funding_txo), ("negotiated", 0, true, Some(txo(TX_B, 1))));
        stable_channels::audit::enable_test_capture();

        // The channel's funding is the user's, so the row cannot lock on it, but it makes no second call either.
        pass(&db, &ldk, &push, 102 + RETRY_BACKOFF_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "a signed splice is never made again");
        assert_eq!(row(&db).status, "negotiated");
        // The operator hears of it on the scale of confirmations, with the call still pending.
        pass(&db, &ldk, &push, signed_at + NEGOTIATED_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.pending_on_node), ("stalled", true));
        assert!(stalled.last_error.as_deref().unwrap().contains("has not locked within"));
        pass(&db, &ldk, &push, signed_at + NEGOTIATED_ALERT_SECS + RETRY_BACKOFF_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_STALLED"]);
    }

    #[tokio::test]
    async fn a_failed_row_that_gets_no_further_call_stalls_a_day_after_its_last_attempt() {
        let _guard = guard();
        let db = database();
        let ldk = node(false);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        let failed_at = 100 + WAKE_TIMEOUT_SECS;
        pass(&db, &ldk, &push, failed_at).await;
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("failed", 1));
        stable_channels::audit::enable_test_capture();

        // No push can go out after that (here the cooldown; a token that expires does the same), so the row waits with the reason noted.
        pass(&db, &ldk, &push, failed_at + RETRY_BACKOFF_SECS).await;
        pass(&db, &ldk, &push, failed_at + QUEUED_ALERT_SECS - 1).await;
        let waiting = row(&db);
        assert_eq!(waiting.status, "failed");
        assert!(waiting.wake_note.as_deref().unwrap().contains("cooldown"));
        pass(&db, &ldk, &push, failed_at + QUEUED_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.attempts), ("stalled", 1));
        assert!(stalled.last_error.as_deref().unwrap().contains("of the last attempt: waiting for the phone's push cooldown"));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_STALLED"]);
    }

    #[tokio::test]
    async fn a_negotiation_is_recorded_on_a_row_stalled_or_cancelled_with_its_call_pending_so_a_later_failure_releases_nothing() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        // The phone is quiet for ten minutes after the call: the row stalls with the call pending.
        let stalled_at = 100 + PENDING_ALERT_SECS;
        pass(&db, &ldk, &push, stalled_at).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).funding_txo), ("stalled", true, None));
        // The splice is signed after all, confirms, and a splice of the user's own replaces it before any pass sees it; then a negotiation on the channel fails.
        let signed_at = stalled_at + 1;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), signed_at);
        let signed = row(&db);
        assert_eq!((signed.status.as_str(), signed.pending_on_node, signed.funding_txo, signed.negotiated_at), ("stalled", true, Some(txo(TX_B, 1)), Some(signed_at)));
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_C, 0, LSP_SATS + AMOUNT_SATS, USER_SATS + 80_000)];
        wallet_funded_tx(&ldk, TX_B);
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, signed_at + 1);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("stalled", true));
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, signed_at + 1 + RETRY_BACKOFF_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "a stalled row's signed splice is never made again");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("stalled", true));

        // Cancelled with the call pending, the row records a signing too, and holds the channel until the operator releases it.
        let cancelled_at = signed_at + 2 + RETRY_BACKOFF_SECS;
        db.cancel_queued_splice(USER_CHANNEL_ID_DECIMAL, Some("by the operator"), false, NEGOTIATION_FAILED_ERROR, cancelled_at).unwrap();
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_C, 0), cancelled_at + 1);
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo), ("cancelled", Some(txo(TX_C, 0))));
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, cancelled_at + 2);
        assert!(row(&db).pending_on_node);
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, cancelled_at + 3).await;
        assert!(!ok, "{status}");
        let events = capture();
        let events = names(&events);
        assert!(events.contains(&"QUEUED_SPLICE_NEGOTIATED") && !events.contains(&"QUEUED_SPLICE_STARTED"), "{events:?}");
    }

    #[tokio::test]
    async fn a_negotiation_soon_after_a_failure_event_revives_the_call_so_no_retry_goes_out() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        // A stale failure event fails the call while its splice is still being signed.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("failed", false));
        stable_channels::audit::enable_test_capture();
        let signed_at = 101 + RETRY_BACKOFF_SECS - 1;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), signed_at);
        let revived = row(&db);
        assert_eq!(
            (revived.status.as_str(), revived.pending_on_node, revived.funding_txo, revived.attempts, revived.last_error),
            ("negotiated", true, Some(txo(TX_B, 1)), 1, None)
        );
        // The retry that was due makes no call; the splice locks once the channel shows it.
        pass(&db, &ldk, &push, 101 + RETRY_BACKOFF_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second call");
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 102 + RETRY_BACKOFF_SECS).await;
        assert_eq!(row(&db).status, "locked");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_NEGOTIATED", "QUEUED_SPLICE_LOCKED"]);
        assert_eq!(events[0].1["revived"], true);
    }

    #[tokio::test]
    async fn a_negotiation_at_the_backoff_or_after_a_refusal_revives_nothing() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        // By the backoff a retry may have gone out, so a negotiation then is left to the pass.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 101 + RETRY_BACKOFF_SECS);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).funding_txo), ("failed", false, None));

        // A refusal is the node's own answer to the call: a negotiation after it is someone else's.
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::LightningError, "no such peer".into()));
        pass(&db, &ldk, &push, 101 + RETRY_BACKOFF_SECS).await;
        let refused = row(&db);
        assert_eq!((refused.status.as_str(), refused.attempts, refused.last_attempt_at), ("failed", 2, Some(101 + RETRY_BACKOFF_SECS)));
        assert!(refused.last_error.as_deref().unwrap().starts_with("splice_in:"));
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 102 + RETRY_BACKOFF_SECS);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("failed", false));
    }

    #[tokio::test]
    async fn the_phones_window_starts_once_the_provider_has_taken_the_push() {
        let _guard = guard();
        let db = database();
        let ldk = node(false);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        // A clock that moves a second per reading: the pass reads it once to list the rows, once for the row, and once more when the provider has answered.
        let readings = AtomicI64::new(100);
        let clock = move || readings.fetch_add(1, Ordering::SeqCst);
        run_once(&db, &ldk, &push, &clock).await;
        let woken = row(&db);
        assert_eq!((woken.status.as_str(), woken.wake_sent_at), ("waking", Some(102)));

        // The same for a pending call pushed again once the re-wake is due: the pass lists at 410, reads 411 for the row, and 412 once the push is out.
        let request_id = woken.request_id;
        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some(&txo(TX_A, 0)), LSP_SATS + USER_SATS, LSP_SATS, 103).unwrap();
        push.lock().await.forget_cooldowns();
        let readings = AtomicI64::new(103 + PENDING_WAKE_SECS + 7);
        let clock = move || readings.fetch_add(1, Ordering::SeqCst);
        run_once(&db, &ldk, &push, &clock).await;
        assert_eq!((row(&db).status.as_str(), row(&db).wake_sent_at), ("splicing", Some(103 + PENDING_WAKE_SECS + 9)));
    }

    #[tokio::test]
    async fn a_manual_splice_is_refused_while_the_channel_has_a_row_the_job_works_on_or_a_call_pending() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 100).unwrap().is_none());
        assert!(open_row(&db, "not a channel", 100).unwrap().is_none());
        let request_id = queued(&db, 100).request_id;
        assert_eq!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 100).unwrap().map(|row| row.status), Some("queued".into()));
        let hex_id = format!("{:032x}", USER_CHANNEL_ID_DECIMAL.parse::<u128>().unwrap());
        assert!(open_row(&db, &hex_id, 100).unwrap().is_some(), "the hex form finds it too");
        pass(&db, &ldk, &push, 100).await;
        // A failed or stalled row is still retried by the job, so it holds the channel whether or not a signing could revive it.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        assert_eq!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 101 + 2 * RETRY_BACKOFF_SECS).unwrap().map(|row| row.status), Some("failed".into()));
        db.stall_queued_splice(USER_CHANNEL_ID_DECIMAL, request_id, "no phone", 102).unwrap();
        assert_eq!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 102 + 2 * RETRY_BACKOFF_SECS).unwrap().map(|row| row.status), Some("stalled".into()));
        // A cancelled row holds it only while its call is pending or could be revived.
        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some(&txo(TX_A, 0)), LSP_SATS + USER_SATS, LSP_SATS, 103).unwrap();
        db.cancel_queued_splice(USER_CHANNEL_ID_DECIMAL, None, false, NEGOTIATION_FAILED_ERROR, 104).unwrap();
        assert_eq!(
            open_row(&db, USER_CHANNEL_ID_DECIMAL, 104).unwrap().map(|row| (row.status, row.pending_on_node)),
            Some(("cancelled".into(), true)),
            "a cancelled row with its call pending still holds the channel"
        );
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 105);
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 105 + RETRY_BACKOFF_SECS - 1).unwrap().is_some(), "a signing could still revive the call");
        // Past the backoff its splice is still looked for at the next request, so the channel stays held until the operator releases it with force.
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 105 + 10 * RETRY_BACKOFF_SECS).unwrap().is_some(), "released by the node's event only");
        db.cancel_queued_splice(USER_CHANNEL_ID_DECIMAL, None, true, NEGOTIATION_FAILED_ERROR, 106).unwrap();
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 106).unwrap().is_none(), "released by the operator");
        db.mark_queued_splice_locked(USER_CHANNEL_ID_DECIMAL, request_id, &txo(TX_B, 1), 107).unwrap();
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 107).unwrap().is_none(), "a locked row is done");
    }

    #[tokio::test]
    async fn a_failure_event_on_a_stalled_pending_call_counts_the_attempt_so_the_retry_waits_and_a_signing_revives_it() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        let stalled_at = 100 + PENDING_ALERT_SECS;
        pass(&db, &ldk, &push, stalled_at).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).funding_txo), ("stalled", true, None));
        stable_channels::audit::enable_test_capture();
        // The phone reconnects and the node resumes the splice; a stale failure event lands while it is being signed.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, stalled_at + 1);
        let failed = row(&db);
        assert_eq!(
            (failed.status.as_str(), failed.attempts, failed.pending_on_node, failed.last_error.as_deref(), failed.last_attempt_at),
            ("failed", 1, false, Some(NEGOTIATION_FAILED_ERROR), Some(stalled_at + 1))
        );
        // No retry inside the backoff, so the signing that follows revives the call instead of racing a second one.
        pass(&db, &ldk, &push, stalled_at + 5).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), stalled_at + 6);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).funding_txo), ("negotiated", true, Some(txo(TX_B, 1))));
        pass(&db, &ldk, &push, stalled_at + 1 + RETRY_BACKOFF_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second call for the request");
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, stalled_at + 2 + RETRY_BACKOFF_SECS).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_NEGOTIATED", "QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn no_request_replaces_a_row_while_a_signing_could_still_revive_its_call() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        let first = row(&db);
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        assert_eq!(row(&db).status, "failed");
        // The operator reads "failed" and queues again at once: refused, since the call's signing could still arrive.
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 130).await;
        assert!(!ok && status.contains("can replace it once"), "{status}");
        assert_eq!(row(&db).request_id, first.request_id);
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 131);
        assert_eq!((row(&db).request_id, row(&db).status.as_str(), row(&db).pending_on_node), (first.request_id, "negotiated", true));
        // After the backoff, a row the event released for good is replaced as before.
        let (ok, _) = cancel(&db, USER_CHANNEL_ID_DECIMAL, true, 132);
        assert!(ok);
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 133);
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 134).await;
        assert!(ok, "{status}");
        assert_ne!(row(&db).request_id, first.request_id);
    }

    #[tokio::test]
    async fn a_failure_event_on_a_cancelled_pending_call_holds_the_channel_through_the_backoff_and_a_signing_keeps_it_cancelled_but_pending() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        let (ok, _) = cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 101);
        assert!(ok);
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 102);
        let released = row(&db);
        assert_eq!(
            (released.status.as_str(), released.pending_on_node, released.last_error.as_deref(), released.last_attempt_at),
            ("cancelled", false, Some(NEGOTIATION_FAILED_ERROR), Some(102))
        );
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 103).await;
        assert!(!ok && status.contains("can replace it once"), "{status}");
        // The failure was stale: the call's splice is signed, and the cancelled row holds its call again.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 104);
        let held = row(&db);
        assert_eq!((held.status.as_str(), held.pending_on_node, held.funding_txo), ("cancelled", true, Some(txo(TX_B, 1))));
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 105);
        assert!(row(&db).pending_on_node, "a signed splice keeps its call");
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 102 + RETRY_BACKOFF_SECS).await;
        assert!(!ok && status.contains("pending"), "{status}");
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, 102 + RETRY_BACKOFF_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "a cancelled row makes no call");
        // It locks when the channel shows the splice, as any pending call does.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 103 + RETRY_BACKOFF_SECS).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("locked", false));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn ledger_rows_for_one_request_are_keyed_to_the_call_they_belong_to() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        stable_channels::audit::enable_test_capture();
        // Call, stall unanswered, the node's failure event, the retry's call, and that one stalling too.
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        pass(&db, &ldk, &push, 100 + PENDING_ALERT_SECS).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101 + PENDING_ALERT_SECS);
        let second_call_at = 101 + PENDING_ALERT_SECS + RETRY_BACKOFF_SECS;
        pass(&db, &ldk, &push, second_call_at).await;
        pass(&db, &ldk, &push, second_call_at + PENDING_ALERT_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 2);
        let events = capture();
        assert_eq!(
            names(&events),
            ["QUEUED_SPLICE_STARTED", "QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_STARTED", "QUEUED_SPLICE_STALLED"]
        );
        let keys: Vec<&str> = events.iter().map(|(_, detail)| detail["dedup_key"].as_str().unwrap()).collect();
        let mut distinct = keys.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), keys.len(), "every ledger row has its own key: {keys:?}");
    }

    #[tokio::test]
    async fn a_retry_checks_a_fresh_listing_right_before_its_call_so_a_lock_that_landed_during_the_pass_is_not_made_again() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        stable_channels::audit::enable_test_capture();
        // The pass lists the channel still on its old funding; the call's splice locks before the retry's call would go out.
        *ldk.channels_after_listing.lock().unwrap() = Some(vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)]);
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 101 + RETRY_BACKOFF_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "the lock was seen before the call");
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo), ("locked", Some(txo(TX_B, 1))));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn a_pushed_phone_that_is_here_but_gets_no_call_fails_the_wake_at_the_timeout() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        assert_eq!(row(&db).status, "waking");
        // The node cannot report its balances, so no call can be made while the phone is here.
        ldk.balances_fail.store(true, Ordering::SeqCst);
        pass(&db, &ldk, &push, 100 + WAKE_GRACE_SECS).await;
        pass(&db, &ldk, &push, 100 + WAKE_TIMEOUT_SECS - 1).await;
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("waking", 0));
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, 100 + WAKE_TIMEOUT_SECS).await;
        let failed = row(&db);
        assert_eq!((failed.status.as_str(), failed.attempts), ("failed", 1));
        assert!(failed.last_error.as_deref().unwrap().contains("no splice-in call could be made"));
        assert!(ldk.splice_ins.lock().unwrap().is_empty());
        // Once the node answers again, the retry calls.
        ldk.balances_fail.store(false, Ordering::SeqCst);
        pass(&db, &ldk, &push, 100 + WAKE_TIMEOUT_SECS + RETRY_BACKOFF_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_STARTED"]);
    }

    #[tokio::test]
    async fn a_plain_cancel_inside_the_revive_window_keeps_it_so_a_signing_still_holds_the_channel() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        // The operator reads "failed" and cancels, without force: the window stays, so no request replaces the row yet.
        let (ok, _) = cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 110);
        assert!(ok);
        let cancelled = row(&db);
        assert_eq!((cancelled.status.as_str(), cancelled.pending_on_node, cancelled.last_error.as_deref()), ("cancelled", false, Some(NEGOTIATION_FAILED_ERROR)));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 111).await;
        assert!(!ok && status.contains("can replace it once"), "{status}");
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 111).unwrap().is_some());
        // The failure was stale: the signing holds the cancelled row's call again, and it locks.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 112);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", true));
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 113).await;
        assert_eq!((row(&db).status.as_str(), ldk.splice_ins.lock().unwrap().len()), ("locked", 1));
        // Past the window, a plain cancel is the end of the row as before: a new request replaces it.
        queued(&db, 200);
        pass(&db, &ldk, &push, 200).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 201);
        let (ok, _) = cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 201 + RETRY_BACKOFF_SECS);
        assert!(ok);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", false));
        // The row still carries the node's answer, so its splice is still looked for: the manual route stays shut until a forced cancel.
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 201 + RETRY_BACKOFF_SECS).unwrap().is_some());
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 202 + RETRY_BACKOFF_SECS).await;
        assert!(ok, "{status}");
    }

    #[tokio::test]
    async fn a_cancelled_request_the_node_or_the_operator_released_is_not_locked_by_a_later_manual_splice() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        // The operator releases the call with force, then splices by hand for most of the amount before queueing again.
        called(&db, &ldk, &push, 100).await;
        let (ok, _) = cancel(&db, USER_CHANNEL_ID_DECIMAL, true, 101);
        assert!(ok);
        let dead = row(&db);
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 102).unwrap().is_none(), "the manual route is open");
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        stable_channels::audit::enable_test_capture();
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 103).await;
        assert!(ok, "{status}");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CREATED"], "the manual splice is not the dead request's lock");
        assert_ne!(row(&db).request_id, dead.request_id);
        // The node refuses the new request's call, the operator cancels plainly, and splices by hand again.
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::LightningError, "no such peer".into()));
        pass(&db, &ldk, &push, 104).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).channel_funding_txo.as_deref()), ("failed", false, Some(txo(TX_B, 1).as_str())));
        let (ok, _) = cancel(&db, USER_CHANNEL_ID_DECIMAL, false, 105);
        assert!(ok);
        assert_eq!(row(&db).last_error.as_deref(), Some("cancelled by the operator"));
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 106).unwrap().is_none());
        *ldk.splice_in_error.lock().unwrap() = None;
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_C, 1, LSP_SATS + AMOUNT_SATS * 3)];
        wallet_funded_tx(&ldk, TX_C);
        stable_channels::audit::enable_test_capture();
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 107).await;
        assert!(ok, "{status}");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CREATED"]);
    }

    #[tokio::test]
    async fn a_failure_event_during_the_requests_wallet_check_cannot_let_a_replacement_past_the_window() {
        let _guard = guard();
        let db = Arc::new(database());
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        let first = row(&db);
        // The event lands after the request's own window check and before its save.
        let released = Arc::clone(&db);
        *ldk.balances_hook.lock().unwrap() = Some(Box::new(move || on_splice_negotiation_failed(&released, USER_CHANNEL_ID_DECIMAL, 130)));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 130).await;
        assert!(!ok && status.contains("can replace it once"), "{status}");
        assert_eq!((row(&db).request_id, row(&db).status.as_str()), (first.request_id, "failed"));
        // So the signing revives that call instead of being lost to a replacement.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 131);
        assert_eq!((row(&db).request_id, row(&db).status.as_str(), row(&db).pending_on_node), (first.request_id, "negotiated", true));
    }
}
