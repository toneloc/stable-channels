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
//! A lock is attributed by what only the LSP's own splice produces. On the LSP's node LDK reports a
//! negotiated splice only when the node's own wallet took part in it, so every outpoint negotiated
//! for a request is kept, and the request has locked once the channel is on one of them: a fee bump,
//! or a splice-out the user added to the same transaction, does not hide it, and nothing has to be
//! looked up. The node's ready event for such an outpoint locks the request too, even when a later
//! splice has moved the channel on by the time the job looks. When no negotiation was seen (an event
//! lost in a restart), the channel's funding has moved on from the outpoint the call was made on and
//! its value has grown by the amount, which a user's splice can also do; and the node's wallet took
//! part in the new funding transaction, which it records as an on-chain payment of its own at
//! broadcast and never does for a splice the user funded. The job checks this on every pass, so a
//! missed event, a lost answer, a restart or a cancel never loses a splice the node completed. While
//! the node cannot be asked, a row that may have locked gets no new call and no replacement, and
//! stalls with an alert if that lasts. A negotiation the node reports while the request has no call
//! pending (one the node ran after all, after a failure event, a refusal or a release) makes the
//! call pending again, so no second call goes out.
//!
//! The phone has one push slot per cooldown. A stability wake the tick decided on owns it: the
//! splice job yields its push while one is due, and splices as soon as the channel is usable.
//!
//! A request is never given up on. A failed try is tried again 3, 10, 20 and 40 minutes later, then
//! every hour, each gap moved by 10 to 20 percent either way; a try that failed only because the phone
//! was away is made as soon as the phone is online, since the schedule is for timing the pushes. A
//! call the node has not answered gets the phone pushed again on the same schedule, and stalls once,
//! to alert the operator, if the node has not answered it in 20 minutes or its signed splice has not
//! locked in 3 hours. The splice job's pushes keep their own spacing, shorter than the shortest gap,
//! rather than the 10-minute cooldown of other pushes, so the schedule holds. Every try is written to
//! the log as `Splice attempted Try N: ...`. Only a lock, the channel closing, or the operator ends
//! it, and the operator's release first checks the channel for the call's lock, and is refused for a
//! signed splice until it has had 3 hours to lock.
//!
//! The phone is not pushed for a try that only the LSP can make go through. While the on-chain
//! wallet is short the try fails without a push, and the next one on the schedule pushes again once
//! the wallet is funded. Once the node has refused the call and the row has failed five tries, the
//! phone is no longer pushed for it; a call still goes out on the schedule whenever the phone is
//! online, and a try that fails any other way is pushed for again.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
use ldk_server_client::ldk_server_grpc::api::{
    splice_in_request, GetBalancesRequest, GetPaymentDetailsRequest, ListChannelsRequest,
    SpliceInRequest,
};
use ldk_server_client::ldk_server_grpc::types::{Channel, PaymentDirection};
use serde_json::{json, Value};
use stable_channels::audit::audit_event;
use stable_channels::db::{Database, ManualNoteOutcome, ManualSplice, QueueSpliceOutcome, QueuedSplice};
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
/// While the node cannot say whether the row's earlier call locked, no new call goes out; once that has lasted this long the row stalls so the operator hears of it.
pub(crate) const LOOKUP_ALERT_SECS: i64 = 600;
/// A row that has had no call for this long, since it was queued or since its last try, stalls once, so the operator hears of a phone that never comes online or has no push token. The schedule goes on.
pub(crate) const QUEUED_ALERT_SECS: i64 = 24 * 3_600;
/// A call the node has not reported as negotiated within this time stalls the row once, which alerts the operator. The call stays pending and the phone is still pushed for it: the node may still run it once the phone is back, or may never have received it. Later than the second push for the call, so the phone has had two chances.
pub(crate) const PENDING_ALERT_SECS: i64 = 1_200;
/// A signed splice that has not locked within this time of the signing stalls the row once, the same way. A splice that is not zero-conf locks only once its transaction has confirmed, so this is on the scale of confirmations, not of a negotiation.
pub(crate) const NEGOTIATED_ALERT_SECS: i64 = 3 * 3_600;
/// The gaps before the first four retries of a failed try, and before the first four pushes for a call the node has not answered: 3, 10, 20 and 40 minutes.
const RETRY_GAPS_SECS: [i64; 4] = [3 * 60, 10 * 60, 20 * 60, 40 * 60];
/// The gap after those, for as long as the request is open.
const RETRY_EVERY_SECS: i64 = 3_600;
/// Each gap is moved by this many percent either way, so requests that failed together are not all tried again at once.
const RETRY_JITTER_PERCENT: (u64, u64) = (10, 20);
/// What the node's own failure event records on the row it fails; a refusal records the call's error instead. A negotiation that follows such a failure this soon, before any retry could go out, revives the call: the event names no outpoint, so a stale or replayed one can fail a call whose splice is still being signed.
const NEGOTIATION_FAILED_ERROR: &str = "the splice negotiation failed";
/// How long after a failure event a negotiation on the channel may still be that call's (see [`NEGOTIATION_FAILED_ERROR`]). The shortest retry gap is longer, so no retry can have gone out inside it.
pub(crate) const REVIVE_WINDOW_SECS: i64 = 120;
/// Failed tries written to the ledger; later ones go to the log only, so a phone that stays away for weeks does not fill the ledger. Also the refused tries in a row after which the phone is no longer pushed for the call.
pub(crate) const LEDGER_FAILED_TRIES: u32 = 5;
/// What a refused call records on the row, ahead of the node's own message.
const REFUSAL_PREFIX: &str = "splice_in: ";
/// The card's note for a row the node keeps refusing; see [`refused_by_node`].
const REFUSED_NOTE: &str = "the node refused the last call, so the phone is no longer pushed for this request; a call still goes out on the schedule whenever the phone is online. Cancel and queue again to push it again";
/// The splice job's own spacing between two pushes to a phone, in place of the 10-minute cooldown of other pushes: shorter than the shortest retry gap, so the schedule times the pushes.
pub(crate) const SPLICE_PUSH_SPACING_SECS: u64 = 120;
/// On-chain sats kept above the amount for the splice transaction's fee.
pub(crate) const FEE_HEADROOM_SATS: u64 = 5_000;
/// Bound on one node call, so a stalled call costs one pass and not the whole job.
const RPC_TIMEOUT_SECS: u64 = 5;

/// The gap before retry `retry` (1 is the first) of request `request_id`: the schedule's gap moved by 10 to 20 percent either way. The move is drawn from the request and the retry, so it is the same on every pass and after a restart.
pub(crate) fn retry_gap_secs(request_id: i64, retry: u32) -> i64 {
    let base = RETRY_GAPS_SECS.get(retry.saturating_sub(1) as usize).copied().unwrap_or(RETRY_EVERY_SECS);
    // splitmix64 of the request and the retry
    let mut z = (request_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::from(retry).wrapping_add(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let (low, high) = RETRY_JITTER_PERCENT;
    let percent = (low + (z >> 1) % (high - low + 1)) as i64;
    let shift = base * percent / 100;
    if z & 1 == 0 { base + shift } else { base - shift }
}

/// When a call made at `called_at` and last pushed for at `pushed_at` is due its next push: the pushes since the call follow the retry schedule.
pub(crate) fn next_pending_push_at(request_id: i64, called_at: i64, pushed_at: Option<i64>) -> i64 {
    let last = pushed_at.unwrap_or(called_at).max(called_at);
    let mut due = called_at;
    let mut retry = 1;
    loop {
        due += retry_gap_secs(request_id, retry);
        if due > last {
            return due;
        }
        retry += 1;
    }
}

/// The try number the log names for a row: its failed tries plus the one under way.
fn try_number(row: &QueuedSplice) -> u32 {
    row.attempts + 1
}

/// What a push records on the row when the phone did not come online in time for it.
fn phone_away_error() -> String {
    format!("the phone did not come online within {WAKE_TIMEOUT_SECS}s of the push")
}

/// Whether the row's last try failed only because the phone was away: no call was made, so one may go out as soon as the phone is online.
fn phone_was_away(row: &QueuedSplice) -> bool {
    row.last_error.as_deref() == Some(phone_away_error().as_str())
}
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
    // A splice the operator made by hand is still on the channel until the funding moves past where it was made: a request now could not tell that splice's lock from its own. Once the channel moved past it, that splice locked or something else ended it; its note is cleared when this request is saved.
    let made_by_hand = match db.load_manual_splice(&key) {
        Ok(Some(manual)) if channel_funding_txo(channel).as_deref() == Some(manual.channel_funding_txo.as_str()) => {
            return (false, manual_refusal(&manual));
        }
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(error) => return (false, format!("could not read the splice made by hand: {error}")),
    };
    // A call the last request made may have locked since the job last looked, also one the node or the operator released. A request that was released before a splice was made by hand is looked for only on the outpoints the node negotiated for it, since the channel's move may be that splice. A request that has just locked here ends this one too: the operator may have queued from a card that did not show the lock yet, and queues again for another splice.
    match db.load_queued_splice(&key) {
        Ok(Some(existing))
            if existing.status != "locked" && (existing.channel_funding_txo.is_some() || !existing.negotiated_txos.is_empty()) =>
        {
            let negotiated_only = made_by_hand && released_for_good(&existing);
            match lock_if_shown_with(db, ldk, &existing, channel, now, negotiated_only).await {
                LockCheck::Locked => {
                    return (
                        false,
                        format!(
                            "the last request's splice-in of {} sats on this channel has just locked; queue again if you want another one",
                            existing.amount_sats
                        ),
                    )
                }
                LockCheck::Unknown => {
                    return (
                        false,
                        "the node could not say whether the last splice-in call for this channel landed; try again once it answers".into(),
                    )
                }
                LockCheck::NotOurs => {}
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
    match db.queue_splice_in(&key, counterparty, amount_sats, now, REVIVE_WINDOW_SECS, NEGOTIATION_FAILED_ERROR, channel_funding_txo(channel).as_deref()) {
        Ok(QueueSpliceOutcome::Queued(row)) => {
            if made_by_hand {
                if let Err(error) = db.clear_manual_splice(&key) {
                    db_failed("clear_manual_splice", &key, error);
                }
            }
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
        // A splice by hand was noted while this request was away from the table.
        Ok(QueueSpliceOutcome::ManualPending(manual)) => (false, manual_refusal(&manual)),
        Err(error) => {
            let message = format!("could not save the queued splice: {error}");
            db_failed("queue_splice_in", &key, error);
            (false, message)
        }
    }
}

/// Cancel a channel's queued splice for the operator: whether a row was cancelled, and a message for them. With `force` the operator has checked that the node holds no splice for the channel, so a call still counted as pending is released; the channel is checked for the call's lock first (see [`release_refusal`]).
pub(crate) async fn cancel(db: &Database, ldk: &dyn LdkServerCalls, user_channel_id: &str, force: bool, now: i64) -> (bool, String) {
    let Some(key) = row_key(user_channel_id) else {
        return (false, format!("{user_channel_id:?} is not a channel id"));
    };
    if force {
        if let Some(refusal) = release_refusal(db, ldk, &key, now).await {
            return (false, refusal);
        }
    }
    let reason = if force { "cancelled and released by the operator" } else { "cancelled by the operator" };
    // The operator's word that the node holds no splice covers one made by hand too.
    let manual_released = force
        && match db.clear_manual_splice(&key) {
            Ok(cleared) => cleared,
            Err(error) => {
                let message = format!("could not release the splice made by hand: {error}");
                db_failed("clear_manual_splice", &key, error);
                return (false, message);
            }
        };
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
        Ok(None) if manual_released => (true, "released the splice-in made by hand on this channel; a new request may go out".into()),
        Ok(None) => (false, "this channel has no queued splice that can be cancelled".into()),
        Err(error) => {
            let message = format!("could not cancel the queued splice: {error}");
            db_failed("cancel_queued_splice", &key, error);
            (false, message)
        }
    }
}

/// Why the operator's release (a forced cancel) must not go out now, if it must not. A signed splice locks once its transaction has confirmed and the phone is online, and a new call behind it would be a second splice, so a signed call can be released only once it has had [`NEGOTIATED_ALERT_SECS`] to lock. And the call's splice may be on the channel already: then the request is locked instead, and while the node cannot say, nothing is released. A request already released before a splice was made by hand is looked for only on the outpoints the node negotiated for it.
async fn release_refusal(db: &Database, ldk: &dyn LdkServerCalls, key: &str, now: i64) -> Option<String> {
    let row = match db.load_queued_splice(key) {
        Ok(Some(row)) if row.status != "locked" => row,
        Ok(_) => return None,
        Err(error) => return Some(format!("could not read the queued splice: {error}")),
    };
    if let (true, Some(txo)) = (row.pending_on_node, row.funding_txo.as_deref()) {
        let signed_at = row.negotiated_at.unwrap_or(row.updated_at);
        if now < signed_at + NEGOTIATED_ALERT_SECS {
            return Some(format!(
                "the node signed this call's splice ({txo}) {}s ago; it locks once its transaction has confirmed and the phone is online. A signed call can be released only {NEGOTIATED_ALERT_SECS}s after the signing, and only if that transaction can never confirm",
                now.saturating_sub(signed_at)
            ));
        }
    }
    if row.channel_funding_txo.is_none() && row.negotiated_txos.is_empty() {
        return None;
    }
    let channels = match timeout(rpc_timeout(), ldk.list_channels(ListChannelsRequest {})).await {
        Ok(Ok(response)) => response.channels,
        Ok(Err(error)) => return Some(format!("could not check the channel for the call's splice first: list_channels failed: {error}")),
        Err(_) => return Some("could not check the channel for the call's splice first: list_channels timed out".into()),
    };
    let channel = channels.iter().find(|c| row_key(&c.user_channel_id).as_deref() == Some(key) && c.counterparty_node_id == row.counterparty)?;
    match lock_if_shown_with(db, ldk, &row, channel, now, released_for_good(&row)).await {
        LockCheck::Locked => Some("the call's splice is on the channel, so the request is locked and there is nothing to release".into()),
        LockCheck::Unknown => Some("the node could not say whether the call's splice landed; release once it answers".into()),
        LockCheck::NotOurs => None,
    }
}

/// Whether the last pass could not list the node's channels, so the next failure is not logged again.
static NODE_UNREACHABLE: AtomicBool = AtomicBool::new(false);
/// When wallet lookups started failing; 0 while they work. Logged once per episode, and a row kept from a new call this long stalls with an alert.
static LOOKUP_UNAVAILABLE_SINCE: AtomicI64 = AtomicI64::new(0);
/// Whether a lookup failed since the current pass began.
static LOOKUP_FAILED_IN_PASS: AtomicBool = AtomicBool::new(false);

/// Watches a pass for failed lookups. A pass in which none failed ends the outage, so a lookup that failed for a request, or for a row that has since ended, does not stall the next row at its first failed lookup.
struct LookupWatch;

impl LookupWatch {
    fn begin() -> Self {
        LOOKUP_FAILED_IN_PASS.store(false, Ordering::SeqCst);
        LookupWatch
    }
}

impl Drop for LookupWatch {
    fn drop(&mut self) {
        if !LOOKUP_FAILED_IN_PASS.swap(false, Ordering::SeqCst) && LOOKUP_UNAVAILABLE_SINCE.swap(0, Ordering::SeqCst) != 0 {
            info!("[queued_splice] a pass made no failed wallet lookup; the outage is over");
        }
    }
}

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
    } else {
        LOOKUP_FAILED_IN_PASS.store(true, Ordering::SeqCst);
        if LOOKUP_UNAVAILABLE_SINCE.compare_exchange(0, now.max(1), Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            warn!("[queued_splice] {}; the lock is checked again next pass", error);
        }
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
    let _lookups = LookupWatch::begin();
    let rows = match db.list_open_queued_splices() {
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
        if row.channel_funding_txo.is_some() || !row.negotiated_txos.is_empty() {
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
        if matches!(row.status.as_str(), "queued" | "failed") && now >= since_call(&row) + QUEUED_ALERT_SECS {
            // The next pass tries it from its new state, so a failure then is counted.
            stall_uncalled(db, &row, now);
            continue;
        }
        match row.status.as_str() {
            "queued" | "failed" | "stalled" => {
                // A row that failed a try waits for its next one on the schedule; one never tried goes now. So does one whose last try failed only because the phone was away, once the phone is online: the schedule times the pushes, and a phone that comes back by itself is not kept waiting for the next one.
                let due = row.attempts == 0 || now >= row.last_attempt_at.unwrap_or(0) + retry_gap_secs(row.request_id, row.attempts);
                if !due && !(usable && phone_was_away(&row)) {
                    continue;
                }
                wake_or_splice(db, ldk, push, &row, channel, now, clock).await;
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
                        phone_away_error()
                    };
                    fail(db, &row, &error, now);
                }
            }
            other => warn!("[queued_splice] {} is {:?} with no call pending", row.user_channel_id, other),
        }
    }
}

/// A row whose call the node has not answered: lock it when the channel shows the splice, otherwise keep waking the phone on the retry schedule, which is all the node needs to go on, and alert the operator once if it has waited too long. No new call goes out while one is pending, so this never stops: only the node's answer, the lock, or the operator's release ends it.
async fn follow_pending_call(
    db: &Database,
    ldk: &dyn LdkServerCalls,
    push: &Arc<Mutex<PushService>>,
    row: &QueuedSplice,
    channel: &Channel,
    now: i64,
    clock: &(dyn Fn() -> i64 + Sync),
) {
    if lock_if_shown(db, ldk, row, channel, now).await == LockCheck::Locked || row.status == "cancelled" {
        return;
    }
    let called_at = row.last_attempt_at.unwrap_or(row.updated_at);
    if row.status != "stalled" {
        alert_pending_call(db, row, called_at, now);
    }
    if now >= next_pending_push_at(row.request_id, called_at, row.wake_sent_at) {
        wake_again(db, push, row, now, clock).await;
    }
}

/// Stall a pending call the node has not answered within [`PENDING_ALERT_SECS`], or whose signed splice has not locked within [`NEGOTIATED_ALERT_SECS`] of the signing. The card shows it stalled with the reason, and the ledger gets one row for the call. The row keeps its call, so nothing else changes: it still locks, and the phone is still pushed for it.
fn alert_pending_call(db: &Database, row: &QueuedSplice, called_at: i64, now: i64) {
    // Each alert has its own ledger row: a call stalled before the node reported it can stall again once signed.
    let (alert, key) = match row.funding_txo.as_deref() {
        Some(txo) if now >= row.negotiated_at.unwrap_or(called_at) + NEGOTIATED_ALERT_SECS => (
            format!(
                "a splice was signed ({txo}) but has not locked within {NEGOTIATED_ALERT_SECS}s; it locks once its transaction has confirmed and the phone is online. Release it only if that transaction can never confirm, or the LSP's funds go into the channel twice"
            ),
            format!("signed:{called_at}:{txo}"),
        ),
        None if now >= called_at + PENDING_ALERT_SECS => (
            format!(
                "the node has not reported the splice-in call within {PENDING_ALERT_SECS}s; it may still hold it and run it once the phone is back, so the phone is still pushed for it. Release it only if the node has restarted or dropped it since: otherwise a new request's call could land as a second splice"
            ),
            format!("pending:{called_at}"),
        ),
        _ => return,
    };
    match db.stall_queued_splice_pending(&row.user_channel_id, row.request_id, &alert, now) {
        Ok(Some(stalled)) => {
            warn!("[queued_splice] Splice attempted Try {}: stalled on {}: {}; the phone is still pushed for it", try_number(row), row.user_channel_id, alert);
            audit("QUEUED_SPLICE_STALLED", &stalled, &[("dedup_key", dedup_key("stalled", &stalled, key))]);
        }
        Ok(None) => {}
        Err(error) => db_failed("stall_queued_splice_pending", &row.user_channel_id, error),
    }
}

/// The channel moved since the row's last call, but the node cannot say whether that was the LSP's own splice: no new call goes out, and once that has lasted as long as an unanswered call may, the row stalls so the operator hears of it, whether it was waiting for its retry or already pushed for it.
fn hold_for_lookup(db: &Database, row: &QueuedSplice, now: i64) {
    let note = "the node could not say whether the earlier splice-in landed; no new call until it answers";
    if let Err(error) = db.note_queued_splice_wake_issue(&row.user_channel_id, row.request_id, note, now) {
        db_failed("note_queued_splice_wake_issue", &row.user_channel_id, error);
    }
    if row.status == "stalled" || lookup_unavailable_for(now) < LOOKUP_ALERT_SECS {
        return;
    }
    let error = format!("{note}; it has not answered for {LOOKUP_ALERT_SECS}s");
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

/// Stall a row that has had no call for [`QUEUED_ALERT_SECS`], with why no push or call went out, and write one ledger row for it. The row is still tried on the schedule.
fn stall_uncalled(db: &Database, row: &QueuedSplice, now: i64) {
    let since = if row.last_attempt_at.is_some() { "the last try" } else { "queueing" };
    let error = match row.wake_note.as_deref() {
        Some(note) => format!("no splice-in call could be made within {QUEUED_ALERT_SECS}s of {since}: {note}"),
        None => format!("no splice-in call could be made within {QUEUED_ALERT_SECS}s of {since}"),
    };
    match db.stall_queued_splice(&row.user_channel_id, row.request_id, &error, now) {
        Ok(Some(stalled)) => {
            warn!("[queued_splice] Splice attempted Try {}: stalled on {}: {}; still tried on the schedule", try_number(row), row.user_channel_id, error);
            audit("QUEUED_SPLICE_STALLED", &stalled, &[("dedup_key", dedup_key("stalled", &stalled, format_args!("queued:{}", since_call(row))))]);
        }
        Ok(None) => {}
        Err(error) => db_failed("stall_queued_splice", &row.user_channel_id, error),
    }
}

/// Lock the row when the channel shows its own splice completed: the channel is on an outpoint the node negotiated for the request, or, failing that, the funding grew since its call and the node's wallet funded the new transaction. The lock is keyed to the request that was read, so one the operator replaced during the wallet lookup is left to its own pass.
async fn lock_if_shown(db: &Database, ldk: &dyn LdkServerCalls, row: &QueuedSplice, channel: &Channel, now: i64) -> LockCheck {
    lock_if_shown_with(db, ldk, row, channel, now, false).await
}

/// [`lock_if_shown`]; with `negotiated_only`, only an outpoint the node negotiated for the request counts, for a request released before a splice was made by hand, which may be what moved the channel.
async fn lock_if_shown_with(db: &Database, ldk: &dyn LdkServerCalls, row: &QueuedSplice, channel: &Channel, now: i64, negotiated_only: bool) -> LockCheck {
    let current = channel_funding_txo(channel).unwrap_or_default();
    if !row.negotiated_txos.iter().any(|txo| *txo == current) {
        if negotiated_only {
            return LockCheck::NotOurs;
        }
        let Some(txid) = funding_grew_since_call(row, channel) else {
            return LockCheck::NotOurs;
        };
        match wallet_funded(ldk, &txid, now).await {
            Some(true) => {}
            Some(false) => return LockCheck::NotOurs,
            None => return LockCheck::Unknown,
        }
    }
    record_lock(db, row, &current, Some(channel), now);
    LockCheck::Locked
}

/// Mark the row's request locked on `funding_txo`, with the channel as listed when it was seen. A request already locked, or replaced since it was read, is left alone: either way the splice is not for a new call.
fn record_lock(db: &Database, row: &QueuedSplice, funding_txo: &str, channel: Option<&Channel>, now: i64) {
    match db.mark_queued_splice_locked(&row.user_channel_id, row.request_id, funding_txo, now) {
        Ok(Some(locked)) => {
            info!("[queued_splice] splice-in of {} sats locked on {} ({})", row.amount_sats, row.user_channel_id, funding_txo);
            audit(
                "QUEUED_SPLICE_LOCKED",
                &locked,
                &[
                    ("status", json!("completed")),
                    ("channel_value_before", json!(row.channel_value_before)),
                    ("channel_value_after", json!(channel.map(|c| c.channel_value_sats))),
                    ("lsp_sats_before", json!(row.lsp_sats_before)),
                    ("lsp_sats_after", json!(channel.map(lsp_side_sats))),
                    ("dedup_key", dedup_key("locked", row, funding_txo)),
                ],
            );
        }
        Ok(None) => {}
        Err(error) => db_failed("mark_queued_splice_locked", &row.user_channel_id, error),
    }
}

/// Push the phone before anything else, so the app can hold its node; when no push can go out now and the phone is online anyway, splice while it is here. A try that only the LSP can make go through is not pushed for: a short wallet fails it at once, and a node that keeps refusing the call gets it only when the phone is online by itself. Either way the schedule goes on.
async fn wake_or_splice(
    db: &Database,
    ldk: &dyn LdkServerCalls,
    push: &Arc<Mutex<PushService>>,
    row: &QueuedSplice,
    channel: &Channel,
    now: i64,
    clock: &(dyn Fn() -> i64 + Sync),
) {
    match wallet_shortfall(ldk, row.amount_sats).await {
        Ok(None) => {}
        Ok(Some(short)) => {
            fail(db, row, &format!("{short}; the phone is not pushed until it is funded"), now);
            return;
        }
        Err(()) => {
            note_no_push(db, row, "the node could not report its on-chain balance; no push or call until it does", now);
            return;
        }
    }
    if refused_by_node(row) {
        note_no_push(db, row, REFUSED_NOTE, now);
        if channel.is_usable {
            start_splice(db, ldk, row, channel, now, false).await;
        }
        return;
    }
    if send_push(db, push, row, now).await {
        info!(
            "[queued_splice] Splice attempted Try {}: pushed the phone for {} sats on {}; splicing once it is online",
            try_number(row),
            row.amount_sats,
            row.user_channel_id
        );
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
    let called_at = row.last_attempt_at.unwrap_or(row.updated_at);
    let pushed_at = clock();
    info!(
        "[queued_splice] Splice attempted Try {}: call on {} still pending on the node; pushed the phone again, next push in about {} min",
        try_number(row),
        row.user_channel_id,
        (next_pending_push_at(row.request_id, called_at, Some(pushed_at)) - pushed_at) / 60
    );
    match db.note_queued_splice_wake_sent(&row.user_channel_id, row.request_id, pushed_at) {
        Ok(Some(woken)) => audit(
            "QUEUED_SPLICE_WAKE_SENT",
            &woken,
            &[("timeout_secs", json!(next_pending_push_at(row.request_id, called_at, Some(pushed_at)) - pushed_at))],
        ),
        Ok(None) => {}
        Err(error) => db_failed("note_queued_splice_wake_sent", &row.user_channel_id, error),
    }
}

/// True when the provider accepted a push for the row; otherwise the reason is noted on the row.
async fn send_push(db: &Database, push: &Arc<Mutex<PushService>>, row: &QueuedSplice, now: i64) -> bool {
    let node_id = row.counterparty.as_str();
    let (stability_due, has_token, may_push) = {
        let service = push.lock().await;
        (service.stability_wake_due(node_id), service.has_token(node_id), service.should_notify_within(node_id, SPLICE_PUSH_SPACING_SECS))
    };
    let note = if stability_due {
        "a stability wake is due for this phone and takes its push; the splice runs once the channel is usable"
    } else if !has_token {
        "no usable push token for this phone (none registered, or older than 30 days)"
    } else if !may_push {
        "waiting for the phone's push spacing or backoff"
    } else if crate::push::notify_within(push, node_id, LSP_SPLICE_IN_DIRECTION, SPLICE_PUSH_SPACING_SECS).await {
        return true;
    } else {
        "the push provider did not accept the push"
    };
    note_no_push(db, row, note, now);
    false
}

/// Note on the row why no push went out, and log it when the reason changes.
fn note_no_push(db: &Database, row: &QueuedSplice, note: &str, now: i64) {
    if row.wake_note.as_deref() != Some(note) {
        info!("[queued_splice] Splice attempted Try {}: no push for {}: {}", try_number(row), row.user_channel_id, note);
    }
    if let Err(error) = db.note_queued_splice_wake_issue(&row.user_channel_id, row.request_id, note, now) {
        db_failed("note_queued_splice_wake_issue", &row.user_channel_id, error);
    }
}

/// Whether the node has refused the row's call often enough that the phone is no longer pushed for it: the node refused the last [`LEDGER_FAILED_TRIES`] tries in a row. Only the LSP's operator can change what the node keeps refusing, so waking the phone every hour would not help. A call still goes out on the schedule whenever the phone is online, and once a try fails any other way, the next one is pushed for again.
fn refused_by_node(row: &QueuedSplice) -> bool {
    row.refusals >= LEDGER_FAILED_TRIES
}

/// Why the on-chain wallet cannot fund a splice-in of `amount_sats` now, or None when it can. Err when the node could not say, which is not a try: the next pass asks again.
async fn wallet_shortfall(ldk: &dyn LdkServerCalls, amount_sats: u64) -> Result<Option<String>, ()> {
    let spendable = match timeout(rpc_timeout(), ldk.get_balances(GetBalancesRequest {})).await {
        Ok(Ok(balances)) => balances.spendable_onchain_balance_sats,
        Ok(Err(error)) => {
            warn!("[queued_splice] get_balances failed: {}", error);
            return Err(());
        }
        Err(_) => {
            warn!("[queued_splice] get_balances timed out");
            return Err(());
        }
    };
    Ok(funding_shortfall_sats(spendable, amount_sats).map(|shortfall| {
        format!("the on-chain wallet is {shortfall} sats short: {spendable} spendable, {} needed", amount_sats.saturating_add(FEE_HEADROOM_SATS))
    }))
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
    // A row with an earlier call or negotiation behind it: the listing this pass began with may predate that splice's lock, so a fresh listing is checked right before a new call.
    let fresh_channel: Channel;
    let channel = if row.channel_funding_txo.is_some() || !row.negotiated_txos.is_empty() {
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
    match wallet_shortfall(ldk, row.amount_sats).await {
        Ok(None) => {}
        Ok(Some(short)) => {
            fail(db, row, &short, now);
            return;
        }
        // Not an attempt: the next pass asks again.
        Err(()) => return,
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
            info!(
                "[queued_splice] Splice attempted Try {}: splice-in of {} sats started on {}",
                try_number(&attempt),
                attempt.amount_sats,
                attempt.user_channel_id
            );
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
        Ok(Err(error)) if !node_answered(&error) => {
            warn!("[queued_splice] splice_in answer lost for {}: {}", attempt.user_channel_id, error);
        }
        // The node refused, so nothing went to the phone.
        Ok(Err(error)) => record_refusal(db, &attempt, &format!("{REFUSAL_PREFIX}{}", error.message), now),
        Err(_) => warn!("[queued_splice] splice_in timed out for {}; waiting for the node's events", attempt.user_channel_id),
    }
}

/// The node refused the call, which is a failed attempt. When the row moved on while the call was out, the node reported a negotiation for the request meanwhile, which on the LSP's node is a splice of its own, signed (a call it ran after all): the row keeps that splice and its pending call, so a refusal of the new call, for that very splice, makes no second call possible. A row the operator cancelled meanwhile, with nothing signed, has nothing pending any more.
fn record_refusal(db: &Database, attempt: &QueuedSplice, error: &str, now: i64) {
    if record_failure(db, attempt, &attempt.status, error, true, now) {
        return;
    }
    match db.load_queued_splice(&attempt.user_channel_id) {
        Ok(Some(current)) if current.request_id == attempt.request_id && current.funding_txo.is_some() => {
            warn!(
                "[queued_splice] Splice attempted Try {}: refused on {} ({}), while the splice the node signed for the request ({}) is pending; that call stays pending",
                try_number(attempt),
                attempt.user_channel_id,
                error,
                current.funding_txo.as_deref().unwrap_or_default()
            );
            return;
        }
        Ok(_) => {}
        // Unreadable: the pending call is kept, which a later answer or the operator can still end.
        Err(error) => {
            audit_event("DB_READ_FAILED", json!({ "op": "load_queued_splice", "user_channel_id": attempt.user_channel_id, "error": error.to_string() }));
            return;
        }
    }
    match db.clear_queued_splice_pending(&attempt.user_channel_id, attempt.request_id, error, now) {
        Ok(_) => {}
        Err(error) => db_failed("clear_queued_splice_pending", &attempt.user_channel_id, error),
    }
}

/// Count a failed try on the row as it was read; the row waits for its next try on the schedule. The node answered, or no call was made, so nothing of the row is pending. False when the row was no longer in that state.
fn fail(db: &Database, row: &QueuedSplice, error: &str, now: i64) -> bool {
    fail_from(db, row, &row.status, error, now)
}

/// [`fail`] for the row's request while it is in `from_status` rather than the state it was read in.
fn fail_from(db: &Database, row: &QueuedSplice, from_status: &str, error: &str, now: i64) -> bool {
    record_failure(db, row, from_status, error, false, now)
}

/// [`fail_from`], counting the try as one more the node `refused` in a row, or ending such a run.
fn record_failure(db: &Database, row: &QueuedSplice, from_status: &str, error: &str, refused: bool, now: i64) -> bool {
    let failed = if refused {
        db.refuse_queued_splice(&row.user_channel_id, row.request_id, from_status, error, now)
    } else {
        db.fail_queued_splice(&row.user_channel_id, row.request_id, from_status, error, now)
    };
    match failed {
        Ok(Some(failed)) => {
            warn!(
                "[queued_splice] Splice attempted Try {} failed on {}: {}; next try in about {} min",
                failed.attempts,
                failed.user_channel_id,
                error,
                retry_gap_secs(failed.request_id, failed.attempts) / 60
            );
            // Later tries keep the row's own error current but write no more ledger rows.
            if failed.attempts <= LEDGER_FAILED_TRIES {
                audit("QUEUED_SPLICE_FAILED", &failed, &[("dedup_key", dedup_key("failed", &failed, failed.attempts))]);
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
        && row.last_attempt_at.is_some_and(|failed_at| now < failed_at + REVIVE_WINDOW_SECS)
}

/// Why a request is refused while a splice made by hand is still on the channel.
fn manual_refusal(manual: &ManualSplice) -> String {
    format!(
        "a splice-in made by hand on this channel at {} has not locked yet (the channel is still on {}); queue once the channel shows its new funding, or cancel with force once you have checked that the node holds no splice for it",
        manual.made_at, manual.channel_funding_txo
    )
}

/// Whether a splice-in call's error is the node's own answer. An internal error is the answer lost in transport, or the request never arriving: the node may hold the call.
fn node_answered(error: &LdkServerError) -> bool {
    !matches!(error.error_code, LdkServerErrorCode::InternalError)
}

/// Why a request is refused while a signing could still revive the row's last call.
fn revivable_refusal(row: &QueuedSplice, now: i64) -> String {
    format!(
        "the node reported the last splice-in call's negotiation failed {}s ago; a new request can replace it once {REVIVE_WINDOW_SECS}s have passed, in case that call's splice is signed after all",
        now - row.last_attempt_at.unwrap_or(now)
    )
}

/// A cancelled row whose call is over on the node's or the operator's word: the node refused it, or the operator cancelled after a refusal or with force, which left a text other than the failure event's. Nothing of it can still lock, so a splice the channel shows later is someone else's, the operator's own by hand included, and is not taken for the row's.
fn released_for_good(row: &QueuedSplice) -> bool {
    row.status == "cancelled" && !row.pending_on_node && row.last_error.as_deref() != Some(NEGOTIATION_FAILED_ERROR)
}

/// The node negotiated a splice on the channel. On the LSP's node LDK reports a negotiation only for a splice its own wallet took part in, so it is the request's splice (or the replaced request's), with one exception: a splice the operator made by hand after the request was released. Every row with a call pending records it, whatever its status, since a signed splice keeps the call through a later negotiation failure. A row with no call pending is armed with it: the node ran a call after all that a failure event, a refusal or a release had let go, or that it held until the phone came back, so the row's call is pending again and no second call goes out. A released row is not armed by a splice made by hand since.
pub(crate) fn on_splice_negotiated(db: &Database, user_channel_id: &str, funding_txo: &str, now: i64) {
    let Some(key) = row_key(user_channel_id) else { return };
    let Some(row) = row_for_event(db, &key) else { return };
    let (op, result) = if row.pending_on_node {
        ("note_queued_splice_negotiated", db.note_queued_splice_negotiated(&key, row.request_id, funding_txo, now))
    } else if row.status != "locked" && !made_by_hand_after_release(db, &row) {
        ("arm_queued_splice_negotiated", db.arm_queued_splice_negotiated(&key, row.request_id, funding_txo, now))
    } else {
        return;
    };
    match result {
        Ok(Some(noted)) => audit("QUEUED_SPLICE_NEGOTIATED", &noted, &[("armed", json!(!row.pending_on_node))]),
        Ok(None) => {}
        Err(error) => db_failed(op, &key, error),
    }
}

/// Whether the row was released, so a splice could then be made by hand on the channel, and one was: a negotiation now is most likely that one. An unreadable note counts as none, so the row is armed rather than left open to a second call.
fn made_by_hand_after_release(db: &Database, row: &QueuedSplice) -> bool {
    row.status == "cancelled" && !holds_channel(row) && matches!(db.load_manual_splice(&row.user_channel_id), Ok(Some(_)))
}

/// The node reports the channel ready on `funding_txo`, which is how a splice locks. When that is an outpoint the node negotiated for the channel's request, the request's splice has locked, also when a later splice moves the channel on before the job's next look.
pub(crate) fn on_channel_ready(db: &Database, user_channel_id: &str, funding_txo: Option<&str>, now: i64) {
    let (Some(key), Some(txo)) = (row_key(user_channel_id), funding_txo) else { return };
    let Some(row) = row_for_event(db, &key) else { return };
    if row.status != "locked" && row.negotiated_txos.iter().any(|negotiated| negotiated == txo) {
        record_lock(db, &row, txo, None, now);
    }
}

/// The node dropped the splice it was negotiating on the channel. A row whose call is out and whose splice was never signed goes back for a retry, and nothing of it is pending any more. A row that already holds a signed splice keeps it and its pending call: the dropped splice may be someone else's, and once a later splice has replaced the signed one on the channel, a check against the channel's current funding could not tell that a second call would fund it twice. Such a row locks when the channel shows its splice; until then its phone is pushed for on the retry schedule, and only the operator can release it.
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
    if let Err(error) = db.clear_manual_splice(&key) {
        db_failed("clear_manual_splice", &key, error);
    }
    let Some(row) = row_for_event(db, &key) else { return };
    end_for_channel(db, &row, "the channel closed", true, now);
}

/// The channel's queued splice while the job may still call for it, or a splice of its own may still land: any row but a locked one or a cancelled one, since the job retries a failed or stalled row too, plus a cancelled row whose last call may still be pending on the node or that the node's failure event released, since a signing can revive such a call inside the backoff and its splice is still looked for at the next request after it; only a forced cancel ends that. A manual splice-in then would be a second splice for the request, or be taken for its lock, so the operator route refuses one; an unreadable queue counts as open.
pub(crate) fn open_row(db: &Database, user_channel_id: &str, _now: i64) -> Result<Option<QueuedSplice>, rusqlite::Error> {
    let Some(key) = row_key(user_channel_id) else { return Ok(None) };
    Ok(db.load_queued_splice(&key)?.filter(holds_channel))
}

/// Whether the channel's queued row keeps a manual splice-in off the channel; the rule `open_row` describes.
fn holds_channel(row: &QueuedSplice) -> bool {
    match row.status.as_str() {
        "locked" => false,
        // The failure event's text is kept until a forced cancel, so the revive window is inside this.
        "cancelled" => row.pending_on_node || row.last_error.as_deref() == Some(NEGOTIATION_FAILED_ERROR),
        _ => true,
    }
}

/// Why a manual splice-in is refused while the channel's queued row holds the channel.
fn held_refusal(open: &QueuedSplice) -> String {
    let pending = if open.pending_on_node { ", its last call pending on the node" } else { "" };
    format!(
        "this channel has a queued splice-in ({}{pending}); cancel it first, with force once you have checked that the node holds no splice for it",
        open.status
    )
}

/// Before the operator's manual Splice+ call: note the channel's funding, so a queued request waits until the channel has moved past it. The check for a queued row that holds the channel and the note are one transaction, so a request saved while the channels were being listed is seen. Ok says whether an earlier note was replaced; an error is the reason the call must not go out.
pub(crate) async fn before_manual_splice(db: &Database, ldk: &dyn LdkServerCalls, user_channel_id: &str, now: i64) -> Result<bool, String> {
    let Some(key) = row_key(user_channel_id) else {
        return Err(format!("{user_channel_id:?} is not a channel id"));
    };
    let channels = match timeout(rpc_timeout(), ldk.list_channels(ListChannelsRequest {})).await {
        Ok(Ok(response)) => response.channels,
        Ok(Err(error)) => return Err(format!("list_channels failed: {error}")),
        Err(_) => return Err("list_channels timed out".into()),
    };
    let Some(channel) = channels.iter().find(|c| row_key(&c.user_channel_id).as_deref() == Some(key.as_str())) else {
        return Err("no open channel has that id".into());
    };
    let funding = channel_funding_txo(channel).unwrap_or_default();
    match db.note_manual_splice_unless(&key, &funding, now, holds_channel) {
        Ok(ManualNoteOutcome::Noted { existed }) => {
            info!("[queued_splice] a splice-in by hand on {key} noted on funding {funding} (replacing an earlier note: {existed})");
            Ok(existed)
        }
        Ok(ManualNoteOutcome::Blocked(open)) => Err(held_refusal(&open)),
        Err(error) => Err(format!("could not note the splice made by hand: {error}")),
    }
}

/// The operator's manual Splice+ call failed. The note is cleared only when the node's answer is its own and this press made the note: an answer lost in transport may leave the call on the node, and a refused second press says nothing about the earlier splice the note is for. Otherwise the note stays until the channel moves past it or the operator cancels with force.
pub(crate) fn after_manual_splice_failed(db: &Database, user_channel_id: &str, replaced_a_note: bool, error: &LdkServerError) {
    let Some(key) = row_key(user_channel_id) else { return };
    if replaced_a_note || !node_answered(error) {
        info!("[queued_splice] the splice-in by hand on {key} failed ({error}); its note stays until the channel moves past it");
        return;
    }
    if let Err(error) = db.clear_manual_splice(&key) {
        db_failed("clear_manual_splice", &key, error);
    }
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
        LOOKUP_FAILED_IN_PASS.store(false, Ordering::SeqCst);
        NODE_UNREACHABLE.store(false, Ordering::SeqCst);
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        std::mem::forget(dir);
        db
    }

    fn queued(db: &Database, now: i64) -> QueuedSplice {
        match db.queue_splice_in(USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, now, REVIVE_WINDOW_SECS, NEGOTIATION_FAILED_ERROR, None).unwrap() {
            QueueSpliceOutcome::Queued(row) => row,
            other => panic!("not queued: {other:?}"),
        }
    }

    fn row(db: &Database) -> QueuedSplice {
        db.load_queued_splice(USER_CHANNEL_ID_DECIMAL).unwrap().expect("row")
    }

    /// When the row's next try is due on the retry schedule.
    fn next_try_at(row: &QueuedSplice) -> i64 {
        row.last_attempt_at.expect("a failed try") + retry_gap_secs(row.request_id, row.attempts)
    }

    #[test]
    fn the_retry_schedule_is_3_10_20_and_40_minutes_then_hourly_each_moved_10_to_20_percent() {
        for request_id in 1..200 {
            for (retry, base) in [(1, 180), (2, 600), (3, 1_200), (4, 2_400), (5, 3_600), (9, 3_600), (500, 3_600)] {
                let gap = retry_gap_secs(request_id, retry);
                let moved = (gap - base).abs();
                assert!(moved >= base / 10 && moved <= base / 5, "request {request_id}, retry {retry}: {gap}s");
                assert_eq!(gap, retry_gap_secs(request_id, retry), "the same on every pass and after a restart");
            }
        }
        let first: Vec<i64> = (1..200).map(|request_id| retry_gap_secs(request_id, 1)).collect();
        assert!(first.iter().any(|&gap| gap < 180) && first.iter().any(|&gap| gap > 180), "moved both ways");
        assert!(first.iter().all(|&gap| gap > REVIVE_WINDOW_SECS), "no retry inside the revive window");
    }

    #[test]
    fn a_pending_call_is_pushed_for_on_the_same_schedule_counted_from_the_call() {
        let id = 7;
        let first = 1_000 + retry_gap_secs(id, 1);
        assert_eq!(next_pending_push_at(id, 1_000, None), first);
        assert_eq!(next_pending_push_at(id, 1_000, Some(900)), first, "the push before the call does not count");
        let second = first + retry_gap_secs(id, 2);
        assert_eq!(next_pending_push_at(id, 1_000, Some(first)), second);
        assert_eq!(next_pending_push_at(id, 1_000, Some(first + 1)), second, "a push held back by the cooldown moves it on by one");
        let mut tenth = 1_000;
        for retry in 1..=10 {
            tenth += retry_gap_secs(id, retry);
        }
        assert_eq!(next_pending_push_at(id, 1_000, Some(tenth - 1)), tenth, "hourly after the fourth, without end");
    }

    #[test]
    fn the_pending_alert_comes_after_the_second_push_for_the_call() {
        for request_id in 1..500 {
            let first = next_pending_push_at(request_id, 0, None);
            let second = next_pending_push_at(request_id, 0, Some(first));
            assert!(second < PENDING_ALERT_SECS, "request {request_id}: second push at {second}s");
            assert!(first >= SPLICE_PUSH_SPACING_SECS as i64, "the splice job's own spacing does not hold the first push back");
        }
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
    async fn an_offline_phone_times_the_wake_out_and_is_pushed_again_at_its_retry_gap_not_after_the_cooldown() {
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
        // The retry waits for its gap on the schedule. The splice job keeps its own spacing between pushes, which it notes without touching the error while the last push is that recent.
        let retry_at = next_try_at(&row(&db));
        pass(&db, &ldk, &push, retry_at - 1).await;
        assert_eq!(row(&db).wake_note, None);
        pass(&db, &ldk, &push, retry_at).await;
        let waiting = row(&db);
        assert_eq!((waiting.status.as_str(), waiting.wake_note.as_deref()), ("failed", Some("waiting for the phone's push spacing or backoff")));
        assert!(waiting.last_error.unwrap().contains("did not come online"));
        assert_eq!(sender.sent().len(), 1);
        // At the gap the last push is longer ago than that spacing, though inside the 10-minute cooldown of other pushes: the phone is pushed again.
        push.lock().await.age_last_push(COUNTERPARTY_HEX, (retry_at - 100) as u64);
        assert!(retry_at - 100 < 600, "inside the other pushes' cooldown");
        pass(&db, &ldk, &push, retry_at + 1).await;
        let woken = row(&db);
        assert_eq!((woken.status.as_str(), woken.wake_sent_at, woken.wake_note), ("waking", Some(retry_at + 1), None));
        assert_eq!(sender.sent().len(), 2);
        assert!(ldk.splice_ins.lock().unwrap().is_empty());
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_WAKE_SENT"]);
    }

    #[tokio::test]
    async fn a_row_that_never_gets_a_call_stalls_once_after_a_day_and_is_tried_when_the_phone_shows_up() {
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
        assert_eq!((waiting.status.as_str(), waiting.attempts), ("queued", 0));
        assert!(waiting.wake_note.as_deref().unwrap().contains("no usable push token"));
        // A day after it was queued it stalls once, with the reason, and keeps waiting.
        pass(&db, &ldk, &push, 100 + QUEUED_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.attempts, stalled.pending_on_node), ("stalled", 0, false));
        assert!(stalled.last_error.as_deref().unwrap().contains("of queueing: no usable push token"));
        let week = 7 * 24 * 3_600;
        pass(&db, &ldk, &push, 100 + week).await;
        assert_eq!(row(&db).status, "stalled");
        // The phone shows up a week later: the splice runs.
        *ldk.channels.lock().unwrap() = vec![channel(true, TX_A, 0)];
        pass(&db, &ldk, &push, 100 + week + 5).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!(row(&db).status, "splicing");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_STARTED"]);
    }
    #[tokio::test]
    async fn a_node_refusal_counts_a_try_and_the_row_is_tried_again_on_the_schedule_without_end() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::LightningError, "no such peer".into()));
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        queued(&db, 100);
        stable_channels::audit::enable_test_capture();

        let tries = LEDGER_FAILED_TRIES + 3;
        let mut now = 100;
        for attempt in 1..=tries {
            pass(&db, &ldk, &push, now).await;
            let failed = row(&db);
            assert_eq!((failed.status.as_str(), failed.attempts), ("failed", attempt), "never stalled");
            assert_eq!(failed.last_error.as_deref(), Some("splice_in: no such peer"));
            assert!(!failed.pending_on_node, "a refusal is an answer");
            // Before its next try on the schedule nothing happens.
            let next = next_try_at(&failed);
            assert_eq!(next - now, retry_gap_secs(failed.request_id, attempt));
            pass(&db, &ldk, &push, next - 1).await;
            assert_eq!(row(&db).attempts, attempt);
            now = next;
        }
        let events = capture();
        assert!(!names(&events).contains(&"QUEUED_SPLICE_STALLED"));
        assert_eq!(
            events.iter().filter(|(name, _)| name == "QUEUED_SPLICE_FAILED").count(),
            LEDGER_FAILED_TRIES as usize,
            "later tries go to the log only"
        );
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

        let retry_at = 101 + REVIVE_WINDOW_SECS;
        pass(&db, &ldk, &push, retry_at).await;
        let held = row(&db);
        assert_eq!((held.status.as_str(), held.attempts), ("failed", 1));
        assert!(held.wake_note.as_deref().unwrap().contains("could not say"));
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no new call while the node cannot say");
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, retry_at + 1).await;
        assert!(!ok && status.contains("could not say"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).amount_sats), ("failed", AMOUNT_SATS), "the row is not replaced");
        // Kept from a call for as long as an unanswered call may wait, the row stalls so the operator hears of it.
        pass(&db, &ldk, &push, retry_at + LOOKUP_ALERT_SECS - 1).await;
        assert_eq!(row(&db).status, "failed");
        pass(&db, &ldk, &push, retry_at + LOOKUP_ALERT_SECS).await;
        assert_eq!(row(&db).status, "stalled");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        // Once the node answers, the splice turns out to be ours: locked, still one call.
        ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, retry_at + LOOKUP_ALERT_SECS + REVIVE_WINDOW_SECS).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_LOCKED"]);
        assert!(events[0].1.get("status").is_none());
        assert!(events[0].1["error"].as_str().unwrap().contains("not answered"));
    }

    #[tokio::test]
    async fn a_pending_row_with_a_failing_lookup_keeps_its_call_alerts_once_and_locks_once_the_node_answers() {
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
        let day = 24 * 3_600;
        pass(&db, &ldk, &push, 100 + day).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("stalled", true), "still waiting for its call");
        ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 100 + day + 5).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
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
        let pushed_at = next_try_at(&row(&db));
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
        pass(&db, &ldk, &push, held_from + LOOKUP_ALERT_SECS - 1).await;
        assert_eq!(row(&db).status, "waking");
        pass(&db, &ldk, &push, held_from + LOOKUP_ALERT_SECS).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("stalled", false), "no silent wait");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no new call");
        // Once the node answers, the splice turns out to be ours.
        ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, held_from + LOOKUP_ALERT_SECS + 1).await;
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
        assert!(!ok && status.contains("has just locked"), "{status}");
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
    async fn a_refusal_while_a_splice_the_node_signed_for_the_request_is_pending_keeps_that_call() {
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
        // While the call is out, the node reports a splice of its own signed for the request (an earlier call it ran after all), and then refuses the call for it.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 102);
        assert_eq!(row(&db).status, "negotiated");
        stable_channels::audit::enable_test_capture();
        record_refusal(&db, &attempt, "splice_in: a splice is already in progress", 103);
        let held = row(&db);
        assert_eq!((held.status.as_str(), held.attempts, held.pending_on_node, held.funding_txo), ("negotiated", 0, true, Some(txo(TX_B, 1))));
        // No retry goes out behind it; it locks once the channel shows it.
        pass(&db, &ldk, &push, 103 + retry_gap_secs(held.request_id, 1)).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty());
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        pass(&db, &ldk, &push, 104 + retry_gap_secs(held.request_id, 1)).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);

        // With nothing signed, a refusal that lands after a cancel only ends the pending call (see the cancel test).
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

        // No second call, whatever the phone does; the phone is pushed again on the schedule, without end; a new request is refused.
        let request_id = row(&db).request_id;
        let first_push = next_pending_push_at(request_id, called_at, Some(100));
        pass(&db, &ldk, &push, first_push - 1).await;
        assert_eq!(sender.sent().len(), 1);
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, first_push).await;
        assert_eq!(sender.sent().len(), 2);
        assert_eq!((row(&db).status.as_str(), row(&db).attempts, row(&db).wake_sent_at), ("splicing", 0, Some(first_push)));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, first_push + 1).await;
        assert!(!ok && status.contains("already splicing"), "{status}");
        // A day later the call is still pending, never followed by another, and the phone is still pushed for. The row has stalled, which alerted the operator.
        let day_later = called_at + 24 * 3_600;
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, day_later).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty(), "a pending call is never followed by another");
        assert_eq!(sender.sent().len(), 3);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("stalled", true));
        // The channel then shows the splice locked: new funding the node's wallet took part in, and the value up by the amount.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS - 300)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, day_later + 1).await;
        let locked = row(&db);
        assert_eq!((locked.status.as_str(), locked.funding_txo, locked.pending_on_node), ("locked", Some(txo(TX_B, 1)), false));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, day_later + 2).await;
        assert!(ok, "{status}");
        let events = capture();
        assert_eq!(
            names(&events),
            ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_LOCKED", "QUEUED_SPLICE_CREATED"]
        );
        assert_eq!(events[3].1["status"], "completed");
        assert_eq!(events[3].1["lsp_sats_after"], LSP_SATS + AMOUNT_SATS - 300);
    }

    #[tokio::test]
    async fn a_pending_call_alerts_the_operator_once_and_the_phone_is_still_pushed_for() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::InternalError, "gRPC request failed: connection reset".into()));
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        let called_at = 100 + WAKE_GRACE_SECS;
        pass(&db, &ldk, &push, called_at).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("splicing", true));
        stable_channels::audit::enable_test_capture();

        // Unanswered: one alert at PENDING_ALERT_SECS, none before it and none after.
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS - 1).await;
        assert_eq!(row(&db).status, "splicing");
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.pending_on_node), ("stalled", true));
        assert!(stalled.last_error.as_deref().unwrap().contains("has not reported the splice-in call"), "{:?}", stalled.last_error);
        let pushes = sender.sent().len();
        let day_later = called_at + 24 * 3_600;
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, day_later).await;
        assert_eq!(sender.sent().len(), pushes + 1, "the phone is still pushed for a stalled call");
        assert_eq!((row(&db).status.as_str(), row(&db).wake_sent_at), ("stalled", Some(day_later)));
        let events = capture();
        assert_eq!(events.iter().filter(|(name, _)| name == "QUEUED_SPLICE_STALLED").count(), 1);

        // Signed but not locked: one alert NEGOTIATED_ALERT_SECS after the signing.
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        let request_id = row(&db).request_id;
        db.note_queued_splice_negotiated(USER_CHANNEL_ID_DECIMAL, request_id, &txo(TX_B, 1), 200).unwrap();
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, 200 + NEGOTIATED_ALERT_SECS - 1).await;
        assert_eq!(row(&db).status, "negotiated", "a signed call has until NEGOTIATED_ALERT_SECS");
        pass(&db, &ldk, &push, 200 + NEGOTIATED_ALERT_SECS).await;
        pass(&db, &ldk, &push, 200 + 2 * NEGOTIATED_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.pending_on_node), ("stalled", true));
        assert!(stalled.last_error.as_deref().unwrap().contains("has not locked"), "{:?}", stalled.last_error);
        assert_eq!(names(&capture()).iter().filter(|name| **name == "QUEUED_SPLICE_STALLED").count(), 1);
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

        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 101).await;
        assert!(ok && status.contains("cannot be taken back"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", true));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 1, 102).await;
        assert!(!ok && status.contains("cancelled it with force"), "{status}");
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 103).await;
        assert!(ok && status.contains("released"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", false));
        assert!(db.list_open_queued_splices().unwrap().is_empty());
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
        cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 101).await;
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

        // The operator queued from a card that did not show the lock yet: this request is refused, and the next goes out.
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 200).await;
        assert!(!ok && status.contains("has just locked"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).amount_sats), ("locked", AMOUNT_SATS));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 201).await;
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
        stable_channels::audit::enable_test_capture();

        // The user's splice-in locks: the funding moved and the value grew, but the node's wallet had no part in it, and the LSP's node reports no negotiation for a splice it took no part in.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_B, 1, LSP_SATS, USER_SATS + 80_000)];
        pass(&db, &ldk, &push, 102).await;
        let waiting = row(&db);
        assert_eq!((waiting.status.as_str(), waiting.attempts, waiting.pending_on_node), ("splicing", 0, true));
        assert!(ldk.splice_ins.lock().unwrap().len() == 1, "no second call: the node may still run this row's splice after the user's");
        // Then the node runs this row's splice, which its wallet funds.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_C, 2), 103);
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_C, 2, LSP_SATS + AMOUNT_SATS, USER_SATS + 80_000)];
        wallet_funded_tx(&ldk, TX_C);
        pass(&db, &ldk, &push, 103).await;
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo), ("locked", Some(txo(TX_C, 2))));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_NEGOTIATED", "QUEUED_SPLICE_LOCKED"]);
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
        pass(&db, &ldk, &push, 101 + REVIVE_WINDOW_SECS).await;
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

        let first_push = next_pending_push_at(row(&db).request_id, called_at, Some(100));
        pass(&db, &ldk, &push, first_push - 1).await;
        assert_eq!(sender.sent().len(), 1);
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, first_push).await;
        let woken = row(&db);
        assert_eq!((woken.status.as_str(), woken.attempts, woken.wake_sent_at), ("negotiated", 0, Some(first_push)));
        assert_eq!(sender.sent().len(), 2);
        // A signed splice waiting for confirmations is not an alert at the negotiation timeout.
        pass(&db, &ldk, &push, called_at + PENDING_ALERT_SECS).await;
        assert_eq!(row(&db).status, "negotiated");
        // Hours after the signing it has alerted the operator once, and is still pushed for, with no second call.
        let hours_later = signed_at + 5 * 3_600;
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, hours_later).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.pending_on_node), ("stalled", true));
        assert!(stalled.last_error.unwrap().contains("has not locked within"));
        assert_eq!(sender.sent().len(), 3);
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second call");
        // Back online and confirmed, the node finishes the splice.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, hours_later + 30).await;
        assert_eq!(row(&db).status, "locked");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_STALLED", "QUEUED_SPLICE_WAKE_SENT", "QUEUED_SPLICE_LOCKED"]);
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
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 101).await;
        assert!(ok && status.contains("cannot be taken back"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", true));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, 1, 102).await;
        assert!(!ok && status.contains("may still be pending"), "{status}");

        // The phone is away for over a week; the node finishes the call when it is back, and the job is still looking.
        pass(&db, &ldk, &push, 103 + 8 * 86_400).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", true));
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 104 + 8 * 86_400).await;
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
    async fn a_short_wallet_is_not_pushed_for_and_the_phone_is_pushed_again_once_it_is_funded() {
        let _guard = guard();
        let db = database();
        let ldk = node(false);
        ldk.spendable_onchain_sats.store(AMOUNT_SATS, Ordering::SeqCst);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);

        // Every try on the schedule fails at once while the wallet is short, and none pushes the phone.
        let mut now = 100;
        for attempt in 1..=LEDGER_FAILED_TRIES + 2 {
            pass(&db, &ldk, &push, now).await;
            let failed = row(&db);
            assert_eq!((failed.status.as_str(), failed.attempts), ("failed", attempt));
            let error = failed.last_error.clone().unwrap();
            assert!(error.contains(&format!("{FEE_HEADROOM_SATS} sats short")) && error.ends_with("the phone is not pushed until it is funded"), "{error}");
            now = next_try_at(&failed);
        }
        assert!(sender.sent().is_empty(), "no push while only the LSP can make the try go through");
        assert!(ldk.splice_ins.lock().unwrap().is_empty());

        // The operator funds the wallet: the next try on the schedule pushes the phone again.
        ldk.spendable_onchain_sats.store(FUNDED_SATS, Ordering::SeqCst);
        pass(&db, &ldk, &push, now - 1).await;
        assert!(sender.sent().is_empty(), "not before the try is due");
        pass(&db, &ldk, &push, now).await;
        assert_eq!(sender.sent(), vec![(COUNTERPARTY_HEX.to_string(), LSP_SPLICE_IN_DIRECTION.to_string())]);
        assert_eq!(row(&db).status, "waking");
    }

    #[tokio::test]
    async fn one_refusal_after_other_failures_does_not_stop_the_pushes() {
        let _guard = guard();
        let db = database();
        let ldk = node(false);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);

        // The first tries push the phone, which never comes online.
        let mut now = 100;
        for attempt in 1..LEDGER_FAILED_TRIES {
            push.lock().await.forget_cooldowns();
            pass(&db, &ldk, &push, now).await;
            assert_eq!(row(&db).status, "waking");
            pass(&db, &ldk, &push, now + WAKE_TIMEOUT_SECS).await;
            let failed = row(&db);
            assert_eq!((failed.status.as_str(), failed.attempts, failed.refusals), ("failed", attempt, 0));
            now = next_try_at(&failed);
        }
        // Then it comes online, and the node refuses the call once.
        *ldk.channels.lock().unwrap() = vec![channel(true, TX_A, 0)];
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::LightningError, "a splice is already in progress".into()));
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, now).await;
        pass(&db, &ldk, &push, now + WAKE_GRACE_SECS).await;
        let refused = row(&db);
        assert_eq!((refused.status.as_str(), refused.attempts, refused.refusals), ("failed", LEDGER_FAILED_TRIES, 1));
        assert!(refused.last_error.as_deref().unwrap().starts_with(REFUSAL_PREFIX));

        // The next try is pushed for as before.
        *ldk.channels.lock().unwrap() = vec![channel(false, TX_A, 0)];
        let sent = sender.sent().len();
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, next_try_at(&refused)).await;
        assert_eq!(sender.sent().len(), sent + 1);
        assert_eq!(row(&db).status, "waking");
    }

    #[tokio::test]
    async fn a_node_that_keeps_refusing_stops_the_pushes_but_not_the_tries() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::LightningError, "insufficient funds".into()));
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);

        // Each of the first tries pushes the phone, which comes online, and the node refuses the call.
        let mut now = 100;
        for attempt in 1..=LEDGER_FAILED_TRIES {
            push.lock().await.forget_cooldowns();
            pass(&db, &ldk, &push, now).await;
            assert_eq!(row(&db).status, "waking");
            pass(&db, &ldk, &push, now + WAKE_GRACE_SECS).await;
            let refused = row(&db);
            assert_eq!((refused.status.as_str(), refused.attempts, refused.last_attempt_at), ("failed", attempt, Some(now + WAKE_GRACE_SECS)));
            assert!(refused.last_error.as_deref().unwrap().starts_with(REFUSAL_PREFIX));
            now = next_try_at(&refused);
        }
        assert_eq!(sender.sent().len(), LEDGER_FAILED_TRIES as usize);

        // From then on the phone is not pushed, but each try still calls while it is online.
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, now).await;
        let refused = row(&db);
        assert_eq!((refused.status.as_str(), refused.attempts, refused.last_attempt_at), ("failed", LEDGER_FAILED_TRIES + 1, Some(now)), "called and refused");
        assert_eq!(sender.sent().len(), LEDGER_FAILED_TRIES as usize);

        // While the phone is away, the due try waits for it with the reason on the card; a day after the last try the row stalls once, with that reason.
        *ldk.channels.lock().unwrap() = vec![channel(false, TX_A, 0)];
        now = next_try_at(&refused);
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, now).await;
        assert_eq!(row(&db).status, "failed");
        pass(&db, &ldk, &push, now + 86_400).await;
        let waiting = row(&db);
        assert_eq!(
            (waiting.status.as_str(), waiting.attempts, waiting.wake_note.as_deref()),
            ("stalled", LEDGER_FAILED_TRIES + 1, Some(REFUSED_NOTE))
        );
        assert!(waiting.last_error.as_deref().unwrap().contains("of the last try: the node refused the last call"));
        assert_eq!(sender.sent().len(), LEDGER_FAILED_TRIES as usize, "no push for a node that keeps refusing");

        // The phone comes online by itself, and the node now takes the call.
        *ldk.splice_in_error.lock().unwrap() = None;
        *ldk.channels.lock().unwrap() = vec![channel(true, TX_A, 0)];
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, now + 86_405).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("splicing", true));
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_STARTED"]);
        assert_eq!(events[0].1["pushed"], false);

        // The negotiation then fails, which is not the node refusing, so the next try is pushed for again.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, now + 86_406);
        let failed = row(&db);
        assert_eq!(failed.last_error.as_deref(), Some(NEGOTIATION_FAILED_ERROR));
        push.lock().await.forget_cooldowns();
        pass(&db, &ldk, &push, next_try_at(&failed)).await;
        assert_eq!(sender.sent().len(), LEDGER_FAILED_TRIES as usize + 1);
        assert_eq!(row(&db).status, "waking");
    }

    #[tokio::test]
    async fn a_request_replaced_during_the_pass_is_not_spliced_for_the_old_amount() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let stale = queued(&db, 100);
        // The operator cancels and queues a different amount while a pass holds the old row.
        cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 101).await;
        db.queue_splice_in(USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 3, 102, REVIVE_WINDOW_SECS, NEGOTIATION_FAILED_ERROR, None).unwrap();
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
        assert_eq!(db.list_open_queued_splices().unwrap().len(), 1);
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
        assert!(db.list_open_queued_splices().unwrap().is_empty());
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CANCELLED", "QUEUED_SPLICE_LOCKED", "QUEUED_SPLICE_CANCELLED"]);
    }

    #[tokio::test]
    async fn node_events_move_the_row() {
        let _guard = guard();
        let db = database();
        let request_id = queued(&db, 100).request_id;
        stable_channels::audit::enable_test_capture();

        // A failure before this row's call is out is not this row's.
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

        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 102).await;
        assert_eq!((ok, status.as_str()), (true, "cancelled"));
        assert_eq!(row(&db).status, "cancelled");
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 103).await;
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
            refusals: 0,
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
            negotiated_txos: Vec::new(),
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
        pass(&db, &ldk, &push, 102 + REVIVE_WINDOW_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "a signed splice is never made again");
        assert_eq!(row(&db).status, "negotiated");
        // Days later it still waits with the call pending, has alerted the operator once, and still makes no second call.
        pass(&db, &ldk, &push, signed_at + NEGOTIATED_ALERT_SECS).await;
        pass(&db, &ldk, &push, signed_at + 3 * 24 * 3_600).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.pending_on_node), ("stalled", true));
        assert!(stalled.last_error.as_deref().unwrap().contains("has not locked within"));
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_STALLED"]);
    }

    #[tokio::test]
    async fn a_failed_row_that_cannot_be_pushed_for_waits_with_the_reason_noted_and_stalls_once_a_day_after_its_last_try() {
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

        // No push can go out after that (here the push spacing, since no real time passes in the test; a token that expires does the same), so the row waits with the reason noted.
        pass(&db, &ldk, &push, next_try_at(&row(&db))).await;
        pass(&db, &ldk, &push, failed_at + QUEUED_ALERT_SECS - 1).await;
        let waiting = row(&db);
        assert_eq!((waiting.status.as_str(), waiting.attempts), ("failed", 1));
        assert!(waiting.wake_note.as_deref().unwrap().contains("spacing"));
        assert!(names(&capture()).is_empty());
        stable_channels::audit::enable_test_capture();
        // A day after its last try it stalls once, with that reason, and still waits.
        pass(&db, &ldk, &push, failed_at + QUEUED_ALERT_SECS).await;
        pass(&db, &ldk, &push, failed_at + 3 * 24 * 3_600).await;
        let stalled = row(&db);
        assert_eq!((stalled.status.as_str(), stalled.attempts), ("stalled", 1));
        assert!(stalled.last_error.as_deref().unwrap().contains("of the last try: waiting for the phone's push spacing"));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_STALLED"]);
    }
    #[tokio::test]
    async fn a_call_stalled_before_the_node_reported_it_gets_its_own_ledger_row_when_it_stalls_again_signed() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        stable_channels::audit::enable_test_capture();

        pass(&db, &ldk, &push, 100 + PENDING_ALERT_SECS).await;
        assert_eq!(row(&db).status, "stalled");
        let signed_at = 100 + PENDING_ALERT_SECS + 1;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), signed_at);
        assert_eq!(row(&db).status, "negotiated");
        pass(&db, &ldk, &push, signed_at + NEGOTIATED_ALERT_SECS).await;
        let stalled = row(&db);
        assert_eq!(stalled.status, "stalled");
        assert!(stalled.last_error.as_deref().unwrap().starts_with("a splice was signed"));
        let keys: Vec<Value> =
            capture().into_iter().filter(|(name, _)| name == "QUEUED_SPLICE_STALLED").map(|(_, event)| event["dedup_key"].clone()).collect();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1], "the ledger keeps both alerts");
    }

    #[tokio::test]
    async fn a_negotiation_is_recorded_on_a_row_stalled_or_cancelled_with_its_call_pending_so_a_later_failure_releases_nothing() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        // A row an earlier version of the job stalled with its call pending, ten minutes after the call.
        let stalled_at = 700;
        db.stall_queued_splice_pending(USER_CHANNEL_ID_DECIMAL, row(&db).request_id, "the node has not reported the splice-in call", stalled_at).unwrap();
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).funding_txo), ("stalled", true, None));
        // The splice is signed after all, confirms, and a splice of the user's own replaces it before any pass sees it; then a negotiation on the channel fails.
        let signed_at = stalled_at + 1;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), signed_at);
        // The signing ends the stall: its note told the operator the node had not reported the call.
        let signed = row(&db);
        assert_eq!((signed.status.as_str(), signed.pending_on_node, signed.funding_txo, signed.negotiated_at), ("negotiated", true, Some(txo(TX_B, 1)), Some(signed_at)));
        assert_eq!(signed.last_error, None);
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_C, 0, LSP_SATS + AMOUNT_SATS, USER_SATS + 80_000)];
        wallet_funded_tx(&ldk, TX_B);
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, signed_at + 1);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("negotiated", true));
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, signed_at + 1 + REVIVE_WINDOW_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "a signed splice is never made again");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("negotiated", true));

        // Cancelled with the call pending, the row records a signing too, and holds the channel until the operator releases it.
        let cancelled_at = signed_at + 2 + REVIVE_WINDOW_SECS;
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
    async fn a_negotiation_soon_after_a_failure_event_arms_the_call_again_so_no_retry_goes_out() {
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
        let signed_at = 101 + REVIVE_WINDOW_SECS - 1;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), signed_at);
        let revived = row(&db);
        assert_eq!(
            (revived.status.as_str(), revived.pending_on_node, revived.funding_txo, revived.attempts, revived.last_error),
            ("negotiated", true, Some(txo(TX_B, 1)), 1, None)
        );
        // The retry that was due makes no call; the splice locks once the channel shows it.
        pass(&db, &ldk, &push, 101 + REVIVE_WINDOW_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second call");
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 102 + REVIVE_WINDOW_SECS).await;
        assert_eq!(row(&db).status, "locked");
        let events = capture();
        assert_eq!(names(&events), ["QUEUED_SPLICE_NEGOTIATED", "QUEUED_SPLICE_LOCKED"]);
        assert_eq!(events[0].1["armed"], true);
    }

    #[tokio::test]
    async fn a_negotiation_after_the_backoff_or_after_a_refusal_arms_the_call_too() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        // On the LSP's node a negotiation is a splice of its own: the call ran after all, whenever it is reported.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 101 + REVIVE_WINDOW_SECS);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).funding_txo), ("negotiated", true, Some(txo(TX_B, 1))));
        pass(&db, &ldk, &push, 101 + REVIVE_WINDOW_SECS + retry_gap_secs(row(&db).request_id, 1)).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no retry behind it");

        // So is one reported after the node refused a later call: the earlier call it held.
        let db = database();
        let ldk = node(true);
        queued(&db, 200);
        *ldk.splice_in_error.lock().unwrap() = Some((LdkServerErrorCode::LightningError, "a splice is already in progress".into()));
        pass(&db, &ldk, &push, 200).await;
        let refused = row(&db);
        assert_eq!((refused.status.as_str(), refused.attempts, refused.pending_on_node), ("failed", 1, false));
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 201);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("negotiated", true));
        *ldk.splice_in_error.lock().unwrap() = None;
        let calls = ldk.splice_ins.lock().unwrap().len();
        pass(&db, &ldk, &push, next_try_at(&refused)).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), calls, "no new call behind the signed splice");
    }

    #[tokio::test]
    async fn the_phones_window_starts_once_the_provider_has_taken_the_push() {
        let _guard = guard();
        let db = database();
        let ldk = node(false);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        // A clock that moves a second per reading: the pass reads it once for the row, and once more when the provider has answered.
        let readings = AtomicI64::new(100);
        let clock = move || readings.fetch_add(1, Ordering::SeqCst);
        run_once(&db, &ldk, &push, &clock).await;
        let woken = row(&db);
        assert_eq!((woken.status.as_str(), woken.wake_sent_at), ("waking", Some(101)));

        // The same for a pending call pushed again once its next push is due: the pass reads one time for the row, and the next once the push is out.
        let request_id = woken.request_id;
        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some(&txo(TX_A, 0)), LSP_SATS + USER_SATS, LSP_SATS, 103).unwrap();
        push.lock().await.forget_cooldowns();
        let due = next_pending_push_at(request_id, 103, Some(101));
        let readings = AtomicI64::new(due + 7);
        let clock = move || readings.fetch_add(1, Ordering::SeqCst);
        run_once(&db, &ldk, &push, &clock).await;
        assert_eq!((row(&db).status.as_str(), row(&db).wake_sent_at), ("splicing", Some(due + 8)));
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
        assert_eq!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 101 + 2 * REVIVE_WINDOW_SECS).unwrap().map(|row| row.status), Some("failed".into()));
        db.stall_queued_splice(USER_CHANNEL_ID_DECIMAL, request_id, "no phone", 102).unwrap();
        assert_eq!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 102 + 2 * REVIVE_WINDOW_SECS).unwrap().map(|row| row.status), Some("stalled".into()));
        // A cancelled row holds it only while its call is pending or could be revived.
        db.mark_queued_splice_splicing(USER_CHANNEL_ID_DECIMAL, request_id, Some(&txo(TX_A, 0)), LSP_SATS + USER_SATS, LSP_SATS, 103).unwrap();
        db.cancel_queued_splice(USER_CHANNEL_ID_DECIMAL, None, false, NEGOTIATION_FAILED_ERROR, 104).unwrap();
        assert_eq!(
            open_row(&db, USER_CHANNEL_ID_DECIMAL, 104).unwrap().map(|row| (row.status, row.pending_on_node)),
            Some(("cancelled".into(), true)),
            "a cancelled row with its call pending still holds the channel"
        );
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 105);
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 105 + REVIVE_WINDOW_SECS - 1).unwrap().is_some(), "a signing could still revive the call");
        // Past the backoff its splice is still looked for at the next request, so the channel stays held until the operator releases it with force.
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 105 + 10 * REVIVE_WINDOW_SECS).unwrap().is_some(), "released by the node's event only");
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
        // A row an earlier version of the job stalled with its call pending.
        let stalled_at = 700;
        db.stall_queued_splice_pending(USER_CHANNEL_ID_DECIMAL, row(&db).request_id, "the node has not reported the splice-in call", stalled_at).unwrap();
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).funding_txo), ("stalled", true, None));
        stable_channels::audit::enable_test_capture();
        // The phone reconnects and the node resumes the splice; a stale failure event lands while it is being signed.
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, stalled_at + 1);
        let failed = row(&db);
        assert_eq!(
            (failed.status.as_str(), failed.attempts, failed.pending_on_node, failed.last_error.as_deref(), failed.last_attempt_at),
            ("failed", 1, false, Some(NEGOTIATION_FAILED_ERROR), Some(stalled_at + 1))
        );
        // No retry before its first gap, so the signing that follows revives the call instead of racing a second one.
        pass(&db, &ldk, &push, stalled_at + 5).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), stalled_at + 6);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node, row(&db).funding_txo), ("negotiated", true, Some(txo(TX_B, 1))));
        pass(&db, &ldk, &push, stalled_at + 1 + REVIVE_WINDOW_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "no second call for the request");
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, stalled_at + 2 + REVIVE_WINDOW_SECS).await;
        assert_eq!(row(&db).status, "locked");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_NEGOTIATED", "QUEUED_SPLICE_LOCKED"]);
    }

    #[tokio::test]
    async fn no_request_replaces_a_row_inside_the_backoff_and_a_replacement_takes_the_old_calls_late_signing() {
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
        // After the backoff the request is replaced.
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS * 2, 101 + REVIVE_WINDOW_SECS).await;
        assert!(ok, "{status}");
        assert_ne!(row(&db).request_id, first.request_id);
        // The old call's splice is signed after all: the replacement takes it, so no call of its own goes out.
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 102 + REVIVE_WINDOW_SECS);
        let armed = row(&db);
        assert_eq!((armed.status.as_str(), armed.pending_on_node, armed.negotiated_txos.clone()), ("negotiated", true, vec![txo(TX_B, 1)]));
        pass(&db, &ldk, &push, 103 + REVIVE_WINDOW_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_failure_event_on_a_cancelled_pending_call_holds_the_channel_through_the_backoff_and_a_signing_keeps_it_cancelled_but_pending() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 101).await;
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
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 102 + REVIVE_WINDOW_SECS).await;
        assert!(!ok && status.contains("pending"), "{status}");
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, 102 + REVIVE_WINDOW_SECS).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "a cancelled row makes no call");
        // It locks when the channel shows the splice, as any pending call does.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        pass(&db, &ldk, &push, 103 + REVIVE_WINDOW_SECS).await;
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
        // Call, the node's failure event, the retry's call, another failure event, and a third call.
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        let second_call_at = next_try_at(&row(&db));
        pass(&db, &ldk, &push, second_call_at).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, second_call_at + 1);
        pass(&db, &ldk, &push, next_try_at(&row(&db))).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 3);
        let events = capture();
        assert_eq!(
            names(&events),
            ["QUEUED_SPLICE_STARTED", "QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_STARTED", "QUEUED_SPLICE_FAILED", "QUEUED_SPLICE_STARTED"]
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
        pass(&db, &ldk, &push, next_try_at(&row(&db))).await;
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
        pass(&db, &ldk, &push, next_try_at(&row(&db))).await;
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
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 110).await;
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
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 201 + REVIVE_WINDOW_SECS).await;
        assert!(ok);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", false));
        // The row still carries the node's answer, so its splice is still looked for: the manual route stays shut until a forced cancel.
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 201 + REVIVE_WINDOW_SECS).unwrap().is_some());
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 202 + REVIVE_WINDOW_SECS).await;
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
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 101).await;
        assert!(ok);
        let dead = row(&db);
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 102).unwrap().is_none(), "the manual route is open");
        // The route notes the splice made by hand before its call.
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 102).await, Ok(false));
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
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 105).await;
        assert!(ok);
        assert_eq!(row(&db).last_error.as_deref(), Some("cancelled by the operator"));
        assert!(open_row(&db, USER_CHANNEL_ID_DECIMAL, 106).unwrap().is_none());
        *ldk.splice_in_error.lock().unwrap() = None;
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 106).await, Ok(false));
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_C, 1, LSP_SATS + AMOUNT_SATS * 3)];
        wallet_funded_tx(&ldk, TX_C);
        stable_channels::audit::enable_test_capture();
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 107).await;
        assert!(ok, "{status}");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CREATED"]);
    }

    #[tokio::test]
    async fn a_request_waits_while_a_splice_made_by_hand_is_still_on_the_channel_so_it_is_not_taken_for_its_lock() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        // The operator splices by hand; the channel still shows the funding that call was made on.
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 100).await, Ok(false));
        assert_eq!(db.load_manual_splice(USER_CHANNEL_ID_DECIMAL).unwrap().map(|note| note.channel_funding_txo), Some(txo(TX_A, 0)));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 101).await;
        assert!(!ok && status.contains("made by hand"), "{status}");
        assert!(db.load_queued_splice(USER_CHANNEL_ID_DECIMAL).unwrap().is_none());
        // It locks: the channel moved on, so the request goes out, and that lock is nobody's in the queue.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        stable_channels::audit::enable_test_capture();
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 102).await;
        assert!(ok, "{status}");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_CREATED"]);
        assert!(db.load_manual_splice(USER_CHANNEL_ID_DECIMAL).unwrap().is_none());
        assert_eq!(row(&db).channel_funding_txo, None, "no call yet");
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 103).await;
        assert!(ok);
        // A hand-made call whose answer is lost may be on the node: its note stays.
        let lost = LdkServerError::new(LdkServerErrorCode::InternalError, "gRPC request failed");
        let refused = LdkServerError::new(LdkServerErrorCode::LightningError, "no such peer");
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 104).await, Ok(false));
        after_manual_splice_failed(&db, USER_CHANNEL_ID_DECIMAL, false, &lost);
        assert_eq!(db.load_manual_splice(USER_CHANNEL_ID_DECIMAL).unwrap().map(|note| note.made_at), Some(104));
        // A second press the node refuses says nothing about the first splice: the note stays too, and a request still waits.
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 105).await, Ok(true));
        after_manual_splice_failed(&db, USER_CHANNEL_ID_DECIMAL, true, &refused);
        assert_eq!(db.load_manual_splice(USER_CHANNEL_ID_DECIMAL).unwrap().map(|note| note.made_at), Some(105));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 106).await;
        assert!(!ok && status.contains("made by hand"), "{status}");
        // Only a first press the node refuses outright leaves nothing on the channel.
        assert!(db.clear_manual_splice(USER_CHANNEL_ID_DECIMAL).unwrap());
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 104).await, Ok(false));
        after_manual_splice_failed(&db, USER_CHANNEL_ID_DECIMAL, false, &refused);
        assert!(db.load_manual_splice(USER_CHANNEL_ID_DECIMAL).unwrap().is_none());
        // One that never moves the funding is released by the operator's forced cancel, row or no row.
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 105).await, Ok(false));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 106).await;
        assert!(!ok && status.contains("made by hand"), "{status}");
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 107).await;
        assert!(ok && status.contains("made by hand"), "{status}");
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 108).await;
        assert!(ok, "{status}");
        assert!(before_manual_splice(&db, &ldk, "not a channel", 109).await.is_err());
        // The channel closing takes the note with it.
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 110).await;
        assert!(ok);
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 111).await, Ok(false));
        on_channel_closed(&db, USER_CHANNEL_ID_DECIMAL, 112);
        assert!(db.load_manual_splice(USER_CHANNEL_ID_DECIMAL).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_splice_by_hand_and_a_request_cannot_pass_each_other_while_one_is_away_from_the_table() {
        let _guard = guard();
        let db = Arc::new(database());
        let ldk = node(true);
        // A request saved while the manual route lists the channels is seen when the note is made.
        queued(&db, 100);
        let held = before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 101).await;
        assert!(held.as_ref().is_err_and(|message| message.contains("queued splice-in (queued)")), "{held:?}");
        assert!(db.load_manual_splice(USER_CHANNEL_ID_DECIMAL).unwrap().is_none());
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 102).await;
        assert!(ok);
        // A splice by hand noted while a request reads the wallet is seen when the request is saved.
        let noted = Arc::clone(&db);
        *ldk.balances_hook.lock().unwrap() = Some(Box::new(move || {
            noted.note_manual_splice_unless(USER_CHANNEL_ID_DECIMAL, &txo(TX_A, 0), 103, |_| false).unwrap();
        }));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 103).await;
        assert!(!ok && status.contains("made by hand"), "{status}");
        assert_eq!(row(&db).status, "cancelled", "nothing saved");
    }

    #[tokio::test]
    async fn a_lookup_that_failed_for_a_request_or_an_ended_row_does_not_stall_the_next_row_at_its_first_failure() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        ldk.payment_lookup_failures.store(1_000, Ordering::SeqCst);
        // The operator's request fails its lookup; then the row is released and the lookups recover with nothing left to look up.
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 102).await;
        assert!(!ok && status.contains("could not say"), "{status}");
        ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 103).await;
        assert!(ok);
        pass(&db, &ldk, &push, 104).await;
        // An hour later a new request's call fails and the lookup fails once: held, not stalled, since no outage has run for 600 s.
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 4_000).await;
        assert!(ok, "{status}");
        pass(&db, &ldk, &push, 4_001).await;
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("splicing", true));
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 4_002);
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_C, 1, LSP_SATS + 2 * AMOUNT_SATS)];
        ldk.payment_lookup_failures.store(1_000, Ordering::SeqCst);
        stable_channels::audit::enable_test_capture();
        let retry_at = 4_002 + REVIVE_WINDOW_SECS;
        pass(&db, &ldk, &push, retry_at).await;
        assert_eq!((row(&db).status.as_str(), row(&db).attempts), ("failed", 1));
        assert!(row(&db).wake_note.as_deref().unwrap().contains("could not say"));
        // The outage is timed from this failure, not the one for the request long gone.
        pass(&db, &ldk, &push, retry_at + LOOKUP_ALERT_SECS - 1).await;
        assert_eq!(row(&db).status, "failed");
        pass(&db, &ldk, &push, retry_at + LOOKUP_ALERT_SECS).await;
        assert_eq!(row(&db).status, "stalled");
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_STALLED"]);
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

    #[tokio::test]
    async fn a_try_that_failed_because_the_phone_was_away_goes_out_as_soon_as_the_phone_is_online() {
        let _guard = guard();
        let db = database();
        let ldk = node(false);
        let dir = tempfile::tempdir().unwrap();
        let sender = FakeSender::new(true);
        let push = service(sender.clone(), dir.path(), &[COUNTERPARTY_HEX]);
        queued(&db, 100);
        pass(&db, &ldk, &push, 100).await;
        pass(&db, &ldk, &push, 100 + WAKE_TIMEOUT_SECS).await;
        let away = row(&db);
        assert!(phone_was_away(&away), "{:?}", away.last_error);
        // The user opens the app between two pushes, long before the next try on the schedule.
        let online_at = 100 + WAKE_TIMEOUT_SECS + 30;
        assert!(online_at < next_try_at(&away));
        pass(&db, &ldk, &push, online_at - 1).await;
        assert!(ldk.splice_ins.lock().unwrap().is_empty(), "nothing while it is away");
        *ldk.channels.lock().unwrap() = vec![channel(true, TX_A, 0)];
        pass(&db, &ldk, &push, online_at).await;
        // The last push is too recent to push again, so the call goes out while the phone is here.
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("splicing", true));
        assert_eq!(sender.sent().len(), 1);
    }

    #[tokio::test]
    async fn a_try_that_failed_with_the_phone_online_still_waits_for_its_gap() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        let failed = row(&db);
        assert!(!phone_was_away(&failed));
        pass(&db, &ldk, &push, next_try_at(&failed) - 1).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1, "an online phone that failed a negotiation is not called again at once");
        pass(&db, &ldk, &push, next_try_at(&failed)).await;
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_request_locks_on_its_negotiated_outpoint_even_when_the_user_took_funds_out_in_the_same_splice_and_no_lookup_works() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 101);
        // The user's node added a splice-out of 20,000 sats to the LSP's transaction: the channel grew by less than most of the amount.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_B, 1, LSP_SATS + AMOUNT_SATS, USER_SATS - 20_000)];
        ldk.payment_lookup_failures.store(1_000, Ordering::SeqCst);
        stable_channels::audit::enable_test_capture();
        pass(&db, &ldk, &push, 102).await;
        let locked = row(&db);
        assert_eq!((locked.status.as_str(), locked.funding_txo, locked.pending_on_node), ("locked", Some(txo(TX_B, 1)), false));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_fee_bumped_splice_locks_the_request_on_whichever_candidate_confirms() {
        let _guard = guard();
        for confirmed in [TX_B, TX_C] {
            let db = database();
            let ldk = node(true);
            let dir = tempfile::tempdir().unwrap();
            let push = service(FakeSender::new(true), dir.path(), &[]);
            called(&db, &ldk, &push, 100).await;
            on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 101);
            on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_C, 1), 102);
            assert_eq!(row(&db).negotiated_txos, [txo(TX_B, 1), txo(TX_C, 1)]);
            // ldk-node keys the wallet's record by the first candidate, which a lookup by the confirmed one would not find.
            wallet_funded_tx(&ldk, TX_B);
            *ldk.channels.lock().unwrap() = vec![channel_with(true, confirmed, 1, LSP_SATS + AMOUNT_SATS)];
            pass(&db, &ldk, &push, 103).await;
            assert_eq!((row(&db).status.as_str(), row(&db).funding_txo), ("locked", Some(txo(confirmed, 1))), "{confirmed}");
        }
    }

    #[tokio::test]
    async fn the_nodes_ready_event_locks_the_request_even_once_a_later_splice_has_moved_the_channel_on() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 101);
        // A ready event for another outpoint, such as the channel's first funding, is not the request's.
        on_channel_ready(&db, USER_CHANNEL_ID_DECIMAL, Some(&txo(TX_A, 0)), 102);
        assert_eq!(row(&db).status, "negotiated");
        stable_channels::audit::enable_test_capture();
        on_channel_ready(&db, USER_CHANNEL_ID_DECIMAL, Some(&txo(TX_B, 1)), 103);
        // By the job's next look the user's own splice has locked on top of it.
        *ldk.channels.lock().unwrap() = vec![channel_sides(true, TX_C, 0, LSP_SATS + AMOUNT_SATS, USER_SATS + 30_000)];
        pass(&db, &ldk, &push, 104).await;
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo), ("locked", Some(txo(TX_B, 1))));
        assert_eq!(names(&capture()), ["QUEUED_SPLICE_LOCKED"]);
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_release_is_refused_for_a_signed_splice_until_it_has_had_its_time_to_lock() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 101);
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 101 + NEGOTIATED_ALERT_SECS - 1).await;
        assert!(!ok && status.contains("can be released only"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("negotiated", true));
        // Past that time, and the channel does not show it, the operator may release it.
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 101 + NEGOTIATED_ALERT_SECS).await;
        assert!(ok, "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).pending_on_node), ("cancelled", false));
    }

    #[tokio::test]
    async fn a_release_first_checks_the_channel_for_the_calls_lock() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        // The call's splice is on the channel, but the node's event was lost and its wallet cannot be asked.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        ldk.payment_lookup_failures.store(1, Ordering::SeqCst);
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 200).await;
        assert!(!ok && status.contains("could not say"), "{status}");
        assert!(row(&db).pending_on_node, "nothing released");
        // Once it can, the request is locked instead of released.
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 201).await;
        assert!(!ok && status.contains("nothing to release"), "{status}");
        assert_eq!((row(&db).status.as_str(), row(&db).funding_txo), ("locked", Some(txo(TX_B, 1))));
        // A channel the node cannot list is not released either.
        let db = database();
        let ldk = node(true);
        called(&db, &ldk, &push, 300).await;
        ldk.list_channels_fails.store(true, Ordering::SeqCst);
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 301).await;
        assert!(!ok && status.contains("could not check the channel"), "{status}");
        assert!(row(&db).pending_on_node);
    }

    #[tokio::test]
    async fn a_request_cancelled_plainly_after_its_text_was_replaced_is_still_checked_before_a_new_one() {
        let _guard = guard();
        let db = database();
        let ldk = node(true);
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        called(&db, &ldk, &push, 100).await;
        on_splice_negotiation_failed(&db, USER_CHANNEL_ID_DECIMAL, 101);
        // The call's splice lands after all, with its event lost, while the node's wallet cannot be asked: the row stalls, which replaces the failure's text.
        *ldk.channels.lock().unwrap() = vec![channel_with(true, TX_B, 1, LSP_SATS + AMOUNT_SATS)];
        wallet_funded_tx(&ldk, TX_B);
        ldk.payment_lookup_failures.store(1_000, Ordering::SeqCst);
        pass(&db, &ldk, &push, 101 + REVIVE_WINDOW_SECS).await;
        pass(&db, &ldk, &push, 101 + REVIVE_WINDOW_SECS + LOOKUP_ALERT_SECS).await;
        assert_eq!(row(&db).status, "stalled");
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, false, 1_000).await;
        assert!(ok);
        assert!(released_for_good(&row(&db)));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 1_001).await;
        assert!(!ok && status.contains("could not say"), "{status}");
        ldk.payment_lookup_failures.store(0, Ordering::SeqCst);
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 1_002).await;
        assert!(!ok && status.contains("has just locked"), "{status}");
        assert_eq!(row(&db).status, "locked");
        assert_eq!(ldk.splice_ins.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_released_request_is_armed_by_its_late_signing_but_not_by_a_splice_made_by_hand_since() {
        let _guard = guard();
        let dir = tempfile::tempdir().unwrap();
        let push = service(FakeSender::new(true), dir.path(), &[]);
        // Released with nothing on the channel, the call then signs after all: the row holds the channel again.
        let db = database();
        let ldk = node(true);
        called(&db, &ldk, &push, 100).await;
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 101).await;
        assert!(ok, "{status}");
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_B, 1), 102);
        let armed = row(&db);
        assert_eq!((armed.status.as_str(), armed.pending_on_node), ("cancelled", true));
        let (ok, status) = queue(&db, &ldk, USER_CHANNEL_ID_DECIMAL, COUNTERPARTY_HEX, AMOUNT_SATS, 103).await;
        assert!(!ok && status.contains("may still be pending"), "{status}");
        let (ok, status) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 104).await;
        assert!(!ok && status.contains("can be released only"), "a signed splice is held for its time to lock, cancelled or not: {status}");
        // Released, then spliced by hand: that splice's negotiation is not taken for the released request's.
        let db = database();
        let ldk = node(true);
        called(&db, &ldk, &push, 200).await;
        let (ok, _) = cancel(&db, &ldk, USER_CHANNEL_ID_DECIMAL, true, 201).await;
        assert!(ok);
        assert_eq!(before_manual_splice(&db, &ldk, USER_CHANNEL_ID_DECIMAL, 202).await, Ok(false));
        on_splice_negotiated(&db, USER_CHANNEL_ID_DECIMAL, &txo(TX_C, 1), 203);
        let released = row(&db);
        assert_eq!((released.status.as_str(), released.pending_on_node, released.negotiated_txos.len()), ("cancelled", false, 0));
    }
}
