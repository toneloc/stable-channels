use crate::audit::audit_event;
use crate::db::{Database, StabilityPaymentRedebit, StabilityPaymentRollback};
use rusqlite::Result as SqliteResult;
use crate::constants::{
    MAX_RISK_LEVEL, SATS_IN_BTC, STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS,
    STABILITY_PAYMENT_AUTH_TTL_SECS, STABILITY_PAYMENT_CLOCK_SKEW_SECS,
    STABILITY_PAYMENT_COOLDOWN_SECS, STABILITY_PAYMENT_MESSAGE_TYPE,
    STABILITY_THRESHOLD_PERCENT, STABILITY_THRESHOLD_USD,
};
use crate::price_feeds::get_fresh_cached_price_no_fetch;
use crate::types::{Bitcoin, StableChannel, USD};
use ldk_node::lightning::ln::channelmanager::PaymentId;
use ldk_node::lightning::routing::router::RouteParametersConfig;
use ldk_node::lightning::types::payment::{PaymentHash, PaymentPreimage};
use ldk_node::payment::{PaymentDetails, PaymentStatus};
use ldk_node::Node;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use ureq::Agent;

// ================================================================
// Reconciliation functions
// ================================================================

/// Reconcile an outgoing payment against the stable position.
///
/// When the user sends a payment, their channel balance decreases.
/// If `backing_sats > actual_receiver_sats`, the payment ate into
/// the stable portion. This function deducts the overflow from
/// `expected_usd` and from the persisted sat allocation.
///
/// Returns `Some(usd_deducted)` if stable was reduced, `None` otherwise.
pub fn reconcile_outgoing(sc: &mut StableChannel, price: f64) -> Option<f64> {
    if sc.backing_sats == 0 || !price.is_finite() || price <= 0.0 {
        return None;
    }

    let user_sats = sc.stable_receiver_btc.sats;
    if sc.backing_sats <= user_sats {
        return None; // Payment was covered by native BTC
    }

    let overflow_sats = sc.backing_sats - user_sats;
    let usd_to_deduct = overflow_sats as f64 / SATS_IN_BTC as f64 * price;
    let old_expected = sc.expected_usd.0;
    let new_expected = (old_expected - usd_to_deduct).max(0.0);

    sc.expected_usd = USD::from_f64(new_expected);
    // Native sats have already been exhausted when live balance drops below backing. Every
    // remaining sat therefore stays in the stable allocation. Deriving backing from
    // new_expected/current_price would silently reclassify sats whenever price moved since the
    // last trade or stability settlement.
    sc.backing_sats = user_sats;
    sc.native_sats = 0;
    recompute_native(sc);

    // Set cooldown so stability check doesn't immediately re-fire
    sc.last_stability_payment = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    Some(usd_to_deduct)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverbackedRepair {
    pub backing_sats_before: u64,
    pub backing_sats_after: u64,
    pub expected_usd_before: f64,
    pub expected_usd_after: f64,
    pub usd_deducted: f64,
}

/// Repair a persisted allocation whose stable backing exceeds the wallet's live channel balance.
/// This is the durable-startup counterpart of outgoing-payment reconciliation: native sats are
/// exhausted first, then the missing stable sats reduce the USD claim at the current trusted price.
pub fn repair_overbacked_allocation(
    sc: &mut StableChannel,
    price: f64,
) -> Option<OverbackedRepair> {
    let live_receiver_sats = sc.stable_receiver_btc.sats;
    if sc.backing_sats <= live_receiver_sats || !price.is_finite() || price <= 0.0 {
        return None;
    }

    let backing_sats_before = sc.backing_sats;
    let expected_usd_before = sc.expected_usd.0.max(0.0);
    let missing_backing_sats = backing_sats_before - live_receiver_sats;
    let usd_deducted =
        (missing_backing_sats as f64 / SATS_IN_BTC as f64 * price).min(expected_usd_before);
    let expected_usd_after = (expected_usd_before - usd_deducted).max(0.0);

    sc.expected_usd = USD::from_f64(expected_usd_after);
    // A cleared USD claim may still leave sats owed to the LSP. Keep them allocated until paid.
    sc.backing_sats = live_receiver_sats;
    sc.native_sats = live_receiver_sats.saturating_sub(sc.backing_sats);
    recompute_native(sc);
    sc.last_stability_payment = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let repair = OverbackedRepair {
        backing_sats_before,
        backing_sats_after: sc.backing_sats,
        expected_usd_before,
        expected_usd_after,
        usd_deducted,
    };
    audit_event(
        "OVERBACKED_ALLOCATION_REPAIRED",
        json!({
            "user_channel_id": format!("{}", sc.user_channel_id),
            "live_receiver_sats": live_receiver_sats,
            "backing_sats_before": repair.backing_sats_before,
            "backing_sats_after": repair.backing_sats_after,
            "expected_usd_before": repair.expected_usd_before,
            "expected_usd_after": repair.expected_usd_after,
            "usd_deducted": repair.usd_deducted,
            "btc_price": price,
        }),
    );
    Some(repair)
}

/// Return whether an outbound Lightning payment is still unresolved.
///
/// LDK temporarily removes an in-flight HTLC from `outbound_capacity_msat`. Treating that lower
/// capacity as a settled spend would permanently reduce the stable claim if the payment later
/// fails. On-chain payments are excluded because they remain pending until confirmation without
/// affecting a channel's Lightning capacity.
fn is_pending_outbound_lightning(
    direction: ldk_node::payment::PaymentDirection,
    status: ldk_node::payment::PaymentStatus,
    is_onchain: bool,
) -> bool {
    direction == ldk_node::payment::PaymentDirection::Outbound
        && status == ldk_node::payment::PaymentStatus::Pending
        && !is_onchain
}

pub fn has_pending_outbound_lightning_payment(node: &Node) -> bool {
    node.list_payments().iter().any(|payment| {
        is_pending_outbound_lightning(
            payment.direction,
            payment.status,
            matches!(payment.kind, ldk_node::payment::PaymentKind::Onchain { .. }),
        )
    })
}

/// Mirror `Database::settle_rolled_back_claims_from_balance` into the live channel. Every call is
/// the claim's one evaluation against the balance, drop or no drop. Returns the sats debited; the
/// caller then judges any remaining overflow as an ordinary overspend.
pub fn settle_lost_claims_from_balance(db: &Database, sc: &mut StableChannel) -> u64 {
    if sc.user_channel_id == 0 {
        return 0;
    }
    let uid = format!("{}", sc.user_channel_id);
    match db.settle_rolled_back_claims_from_balance(&uid, sc.stable_receiver_btc.sats) {
        Ok(0) => 0,
        Ok(debited) => {
            sc.backing_sats = sc.backing_sats.saturating_sub(debited);
            sc.native_sats = sc.stable_receiver_btc.sats.saturating_sub(sc.backing_sats);
            recompute_native(sc);
            audit_event(
                "STABILITY_CLAIM_SETTLED_FROM_BALANCE",
                json!({ "user_channel_id": uid, "debited_sats": debited, "backing_sats": sc.backing_sats }),
            );
            debited
        }
        Err(e) => {
            audit_event(
                "STABILITY_CLAIM_BALANCE_SETTLEMENT_FAILED",
                json!({ "user_channel_id": uid, "error": e.to_string() }),
            );
            0
        }
    }
}

/// Repair an over-backed allocation only when the observed capacity cannot be explained by an
/// unresolved outbound HTLC or by a released stability payment that did leave after all.
pub fn repair_overbacked_allocation_if_safe(
    node: &Node,
    db: &Database,
    sc: &mut StableChannel,
    price: f64,
) -> Option<OverbackedRepair> {
    if has_pending_outbound_lightning_payment(node) {
        // An in-flight HTLC also fakes the drop the balance settlement below would attribute, so
        // the whole evaluation defers, consuming nothing.
        if sc.backing_sats > sc.stable_receiver_btc.sats {
            audit_event(
                "OVERBACKED_REPAIR_SKIPPED_PENDING_HTLC",
                json!({
                    "user_channel_id": format!("{}", sc.user_channel_id),
                    "live_receiver_sats": sc.stable_receiver_btc.sats,
                    "backing_sats": sc.backing_sats,
                    "reason": "outbound Lightning payment is still pending",
                }),
            );
        }
        return None;
    }
    settle_lost_claims_from_balance(db, sc);
    repair_overbacked_allocation(sc, price)
}

/// Reconcile an outgoing forwarded payment on the LSP side.
///
/// The LSP knows the total sats forwarded and the user's current balance.
/// Native BTC is spent first; any overflow eats into the stable position.
///
/// `user_sats` MUST be the user's balance BEFORE the spend. Callers reading a
/// live channel balance after the forward settled (e.g. from `list_channels()`
/// in a PaymentForwarded handler) must add `total_forwarded_sats` back first —
/// passing the post-spend balance understates native and over-deducts stable.
///
/// Returns `Some(usd_deducted)` if stable was reduced, `None` otherwise.
pub fn reconcile_forwarded(
    sc: &mut StableChannel,
    user_sats: u64,
    total_forwarded_sats: u64,
    price: f64,
) -> Option<f64> {
    if sc.backing_sats == 0 || !price.is_finite() || price <= 0.0 {
        return None;
    }

    let native_sats = user_sats.saturating_sub(sc.backing_sats);
    let overflow_sats = total_forwarded_sats.saturating_sub(native_sats);

    if overflow_sats == 0 {
        return None; // Fully covered by native BTC
    }

    let usd_to_deduct = overflow_sats as f64 / SATS_IN_BTC as f64 * price;
    let old_expected = sc.expected_usd.0;
    let new_expected = (old_expected - usd_to_deduct).max(0.0);

    sc.expected_usd = USD::from_f64(new_expected);
    // After forwarding: user's actual remaining balance is user_sats - total_forwarded_sats
    let remaining_user_sats = user_sats.saturating_sub(total_forwarded_sats);
    // An overflow means native was fully consumed, so all remaining sats are stable. Preserve
    // that exact allocation instead of deriving a new one from a potentially newer BTC price.
    sc.backing_sats = remaining_user_sats;
    sc.native_sats = 0;
    recompute_native(sc);

    // Set cooldown so stability check doesn't immediately re-fire on a price micro-tick
    let cooldown_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    sc.last_stability_payment = cooldown_ts;

    audit_event(
        "RECONCILE_FORWARDED_COOLDOWN_SET",
        json!({
            "user_channel_id": format!("{}", sc.user_channel_id),
            "last_stability_payment": cooldown_ts,
            "new_expected_usd": sc.expected_usd.0,
            "new_backing_sats": sc.backing_sats,
            "new_native_sats": sc.native_sats,
        }),
    );

    Some(usd_to_deduct)
}

/// Pre-deduct stable balance for a known outgoing amount (e.g. splice-out).
///
/// Unlike `reconcile_outgoing` which infers the overflow from post-payment balances,
/// this takes the explicit `amount_sats` being withdrawn and compares it against
/// `native_channel_btc` to compute overflow immediately — before on-chain confirmation.
///
/// Returns `Some(usd_deducted)` if stable was reduced, `None` if fully covered by native.
pub fn deduct_outgoing(sc: &mut StableChannel, amount_sats: u64, price: f64) -> Option<f64> {
    let receiver_sats_before = sc.stable_receiver_btc.sats;
    let backing_sats_before = sc.backing_sats;
    deduct_outgoing_from_snapshot(
        sc,
        receiver_sats_before,
        backing_sats_before,
        amount_sats,
        price,
    )
}

/// Deduct a known outgoing amount using the allocation immediately before the spend.
/// This remains correct when the live channel balance has already advanced to its post-splice
/// state by the time the asynchronous funding-output lookup completes.
pub fn deduct_outgoing_from_snapshot(
    sc: &mut StableChannel,
    receiver_sats_before: u64,
    backing_sats_before: u64,
    amount_sats: u64,
    price: f64,
) -> Option<f64> {
    if !price.is_finite() || price <= 0.0 {
        return None;
    }

    let native_sats = receiver_sats_before.saturating_sub(backing_sats_before);
    if amount_sats <= native_sats {
        return None; // Fully covered by native BTC
    }

    let overflow_sats = amount_sats - native_sats;
    let usd_to_deduct = (overflow_sats as f64 / SATS_IN_BTC as f64 * price)
        .min(sc.expected_usd.0.max(0.0));
    let new_expected = (sc.expected_usd.0 - usd_to_deduct).max(0.0);

    sc.expected_usd = USD::from_f64(new_expected);
    sc.backing_sats = backing_sats_before.saturating_sub(overflow_sats);
    sc.native_sats = sc.stable_receiver_btc.sats.saturating_sub(sc.backing_sats);
    recompute_native(sc);

    Some(usd_to_deduct)
}

/// Recompute native_channel_btc from receiver sats and backing_sats.
/// Call this after any mutation to backing_sats to keep native in sync.
pub fn recompute_native(sc: &mut StableChannel) {
    let native_sats = sc.stable_receiver_btc.sats.saturating_sub(sc.backing_sats);
    sc.native_channel_btc = Bitcoin::from_sats(native_sats);
}

/// Reconcile an incoming payment — derive backing_sats from channel balance.
///
/// After receiving a payment, the user's balance increased but
/// `native_sats` hasn't changed. Derive `backing_sats` from the
/// actual balance so the extra sats are attributed correctly.
pub fn reconcile_incoming(sc: &mut StableChannel) {
    // backingSats stays the same on incoming — native absorbs the increase.
    recompute_native(sc);
}

/// Stable/native allocation residue smaller than one cent is not useful to the user and cannot be
/// entered precisely in the two-decimal trade UI. Absorb it into the stable side so a full
/// BTC-to-USD trade produces an exact all-stable allocation.
pub fn normalize_backing_sats(
    receiver_sats: u64,
    backing_sats: u64,
    expected_usd: f64,
    price: f64,
) -> u64 {
    if receiver_sats == 0
        || backing_sats > receiver_sats
        || expected_usd <= 0.0
        || !price.is_finite()
        || price <= 0.0
    {
        return backing_sats;
    }

    let native_sats = receiver_sats - backing_sats;
    let native_usd = native_sats as f64 / SATS_IN_BTC as f64 * price;
    if native_usd < 0.01 {
        receiver_sats
    } else {
        backing_sats
    }
}

/// Derive this peer's stable backing allocation for a trade at its local price.
///
/// The result is clamped to the receiver's post-settlement balance. Sub-cent native residue is
/// absorbed into the stable side so a full BTC-to-USD trade has no floating native remainder.
pub fn trade_backing_sats(
    receiver_sats: u64,
    new_expected_usd: f64,
    quote_price: f64,
) -> u64 {
    if receiver_sats == 0
        || !new_expected_usd.is_finite()
        || new_expected_usd <= 0.0
        || !quote_price.is_finite()
        || quote_price <= 0.0
    {
        return 0;
    }

    let derived_backing = (new_expected_usd / quote_price * SATS_IN_BTC as f64) as u64;
    normalize_backing_sats(
        receiver_sats,
        derived_backing.min(receiver_sats),
        new_expected_usd,
        quote_price,
    )
}

/// Treat sub-cent stable targets as a full exit throughout trade processing.
///
/// The UI cannot action less than one cent, so retaining a fractional-cent target would only
/// bypass the full-exit drift guard and leave an unusable residual allocation.
pub fn normalize_trade_expected_usd(expected_usd: f64) -> f64 {
    if expected_usd.is_finite() && expected_usd >= 0.0 && expected_usd < 0.01 {
        0.0
    } else {
        expected_usd
    }
}

/// Apply only the change in the stable target at this peer's local price.
///
/// Repricing the complete target would erase stability drift accumulated before the trade. A full
/// exit is allowed only while that drift is inside the normal stability deadband; an actionable
/// adjustment must settle first because a trade exit releases the stable allocation.
pub fn trade_backing_after_delta(
    receiver_sats: u64,
    current_backing_sats: u64,
    current_expected_usd: f64,
    new_expected_usd: f64,
    price: f64,
) -> Option<u64> {
    let new_expected_usd = normalize_trade_expected_usd(new_expected_usd);
    if !current_expected_usd.is_finite()
        || current_expected_usd < 0.0
        || !new_expected_usd.is_finite()
        || new_expected_usd < 0.0
        || !price.is_finite()
        || price <= 0.0
    {
        return None;
    }

    let receiver_usd = receiver_sats as f64 / SATS_IN_BTC as f64 * price;
    if new_expected_usd > receiver_usd {
        return None;
    }

    if new_expected_usd == 0.0 {
        return (!allocation_drift_is_actionable(
            current_backing_sats,
            current_expected_usd,
            price,
        ))
        .then_some(0);
    }

    // Floor cumulative targets and subtract them instead of flooring each standalone delta. At a
    // fixed price the differences telescope, so repeated sub-sat trades eventually apply exactly
    // the same backing as one cumulative trade while preserving all pre-trade drift.
    let current_target_sats_f = current_expected_usd / price * SATS_IN_BTC as f64;
    let new_target_sats_f = new_expected_usd / price * SATS_IN_BTC as f64;
    if !current_target_sats_f.is_finite()
        || !new_target_sats_f.is_finite()
        || current_target_sats_f >= u64::MAX as f64
        || new_target_sats_f >= u64::MAX as f64
    {
        return None;
    }
    let current_target_sats = current_target_sats_f.floor() as u64;
    let new_target_sats = new_target_sats_f.floor() as u64;
    let mut backing_sats = if new_expected_usd >= current_expected_usd {
        current_backing_sats.checked_add(new_target_sats.checked_sub(current_target_sats)?)?
    } else {
        current_backing_sats.checked_sub(current_target_sats.checked_sub(new_target_sats)?)?
    };

    // Native dust is absorbed only for a fresh allocation. normalize_backing_sats deliberately
    // leaves an already-over-capacity value unchanged, and the final check rejects it.
    if current_expected_usd < 0.01 && current_backing_sats == 0 {
        backing_sats = normalize_backing_sats(receiver_sats, backing_sats, new_expected_usd, price);
    }
    // Zero is also used by stability checks to mean "legacy/uninitialized backing". Persisting
    // that sentinel for a live target would make the next check rebuild the full target at the
    // latest price and erase the drift this delta calculation is designed to preserve.
    (backing_sats > 0 && backing_sats <= receiver_sats).then_some(backing_sats)
}

fn allocation_drift_is_actionable(
    backing_sats: u64,
    expected_usd: f64,
    price: f64,
) -> bool {
    let current_value = backing_sats as f64 / SATS_IN_BTC as f64 * price;
    let drift_usd = (current_value - expected_usd).abs();
    if expected_usd < 0.01 {
        return drift_usd >= STABILITY_THRESHOLD_USD;
    }
    let drift_percent = drift_usd / expected_usd * 100.0;
    drift_usd >= STABILITY_THRESHOLD_USD && drift_percent >= STABILITY_THRESHOLD_PERCENT
}

/// A sent row LDK has no record of after this long never left the node and is failed.
pub const LOST_LDK_RECORD_TIMEOUT_SECS: i64 = 600;

/// Resolve `pending` sent rows against LDK's payment store. A failure delivered to a process
/// that never sent the payment, or a record LDK has lost, must not block channel spends forever.
/// A fresh success waits for its event, which carries the reconciliation; a stale one lost that
/// event (older builds acknowledged it unsaved) and is completed so it cannot block either.
pub fn reconcile_pending_sent_lightning(
    db: &Database,
    payments: &[PaymentDetails],
    now_unix: i64,
) -> SqliteResult<usize> {
    let mut resolved = 0;
    for (payment_id, created_at) in db.pending_sent_payments("lightning")? {
        let stale = now_unix - created_at > LOST_LDK_RECORD_TIMEOUT_SECS;
        let status = payments.iter().find(|p| format!("{}", p.id) == payment_id).map(|p| p.status);
        let (outcome, reason) = match status {
            Some(PaymentStatus::Failed) => ("failed", "ldk_failed"),
            Some(PaymentStatus::Succeeded) if stale => ("completed", "ldk_succeeded_event_lost"),
            None if stale => ("failed", "no_ldk_record"),
            _ => continue,
        };
        if db.update_payment_status_by_pid(&payment_id, outcome, None)? > 0 {
            resolved += 1;
            audit_event(
                "PENDING_SEND_RESOLVED_ON_RECOVERY",
                json!({ "payment_id": payment_id, "outcome": outcome, "reason": reason }),
            );
        }
    }
    Ok(resolved)
}

fn payment_hash_hex(payment: &PaymentDetails) -> String {
    match &payment.kind {
        ldk_node::payment::PaymentKind::Spontaneous { hash, .. } => format!("{hash}"),
        _ => String::new(),
    }
}

/// A stability claim LDK fails, or never heard of because the process stopped between the claim
/// and the send, is rolled back in the saved books and in `sc` together: taking the live channel
/// by `&mut` makes the caller hold its lock, so a late success cannot re-debit in between. A
/// success event that still arrives re-applies the debit (`settle_rolled_back_stability_payment`).
pub fn reconcile_lost_stability_claims(
    db: &Database,
    sc: &mut StableChannel,
    payments: &[PaymentDetails],
    now_unix: i64,
) -> SqliteResult<usize> {
    let mut rolled_back = 0;
    for (payment_id, created_at) in db.pending_sent_payments("stability")? {
        let stale = now_unix - created_at > LOST_LDK_RECORD_TIMEOUT_SECS;
        let payment = payments.iter().find(|p| format!("{}", p.id) == payment_id);
        let reason = match payment.map(|p| p.status) {
            Some(PaymentStatus::Failed) => "ldk_failed",
            Some(PaymentStatus::Succeeded) if stale => {
                // Its event is long gone; the optimistic debit is already the right books.
                let hash = payment.map(payment_hash_hex).unwrap_or_default();
                let fee = payment.and_then(|p| p.fee_paid_msat);
                if db.complete_pending_stability_payment(&payment_id, &hash, fee)? {
                    audit_event(
                        "STABILITY_CLAIM_COMPLETED_ON_RECOVERY",
                        json!({ "payment_id": payment_id }),
                    );
                }
                continue;
            }
            None if stale => "no_ldk_record",
            _ => continue,
        };
        if let Some(rollback) =
            db.fail_pending_stability_payment(&payment_id, reason == "no_ldk_record")?
        {
            audit_event(
                "STABILITY_CLAIM_ROLLED_BACK_ON_RECOVERY",
                json!({ "payment_id": payment_id, "reason": reason, "restored": rollback.restored }),
            );
            apply_stability_rollback(sc, &rollback);
            rolled_back += 1;
        }
    }
    Ok(rolled_back)
}

/// Undo a failed stability payment's optimistic debit in memory, only while it is still in place.
pub fn apply_stability_rollback(sc: &mut StableChannel, rollback: &StabilityPaymentRollback) -> bool {
    let (Some(uid), Some(before), Some(after)) = (
        rollback.user_channel_id.as_deref(),
        rollback.backing_sats_before,
        rollback.backing_sats_after,
    ) else {
        return false;
    };
    if !rollback.restored || format!("{}", sc.user_channel_id) != uid || sc.backing_sats != after {
        return false;
    }
    sc.backing_sats = before;
    sc.native_sats = sc.stable_receiver_btc.sats.saturating_sub(before);
    recompute_native(sc);
    sc.last_stability_payment = 0;
    sc.payment_made = false;
    true
}

/// Mirror a late success's re-applied debit into the live channel.
pub fn apply_stability_redebit(sc: &mut StableChannel, redebit: &StabilityPaymentRedebit) -> bool {
    if format!("{}", sc.user_channel_id) != redebit.user_channel_id {
        return false;
    }
    sc.backing_sats = sc.backing_sats.saturating_sub(redebit.debit_sats);
    sc.native_sats = sc.stable_receiver_btc.sats.saturating_sub(sc.backing_sats);
    recompute_native(sc);
    true
}

/// LDK derives a spontaneous payment's id from its preimage, so choosing the preimage fixes the id.
pub fn stability_payment_id(preimage: &PaymentPreimage) -> PaymentId {
    PaymentId(PaymentHash::from(*preimage).0)
}

/// Why LDK returned no payment id for a claimed stability payment.
#[derive(Debug)]
pub enum StabilitySendError {
    /// LDK rejected the payment before dispatching it.
    NotSent(String),
    /// LDK writes its payment store after dispatching, so the HTLC may already be in flight.
    OutcomeUnknown(String),
}

/// Persist the claim, then send. LDK can emit a terminal event before `send` returns, so the row
/// that classifies this payment and rolls it back must already be durable; without it nothing is sent.
pub fn send_claimed_stability_payment(
    db: &Database,
    sc: &mut StableChannel,
    mut info: StabilityPaymentInfo,
    now: i64,
    send: impl FnOnce() -> Result<String, StabilitySendError>,
) -> Option<StabilityPaymentInfo> {
    let amount_usd = info.amount_msat as f64 / 1000.0 / SATS_IN_BTC as f64 * info.btc_price;
    if let Err(e) = db.record_pending_stability_payment(
        &info.payment_id,
        info.amount_msat,
        Some(amount_usd),
        info.btc_price,
        &info.counterparty,
        &sc.channel_id.to_string(),
        &format!("{}", sc.user_channel_id),
        sc.expected_usd.0,
        info.backing_sats_before,
        info.backing_sats_after,
        sc.native_sats,
        sc.note.as_deref(),
    ) {
        audit_event(
            "STABILITY_PAYMENT_CLAIM_FAILED",
            json!({
                "payment_id": info.payment_id,
                "user_channel_id": format!("{}", sc.user_channel_id),
                "error": e.to_string(),
            }),
        );
        return None;
    }
    match send() {
        Ok(ldk_id) if ldk_id != info.payment_id => {
            // The claim is keyed by an id derived the way LDK derives it; never let that drift silently.
            audit_event(
                "STABILITY_PAYMENT_ID_MISMATCH",
                json!({ "claimed_payment_id": info.payment_id, "ldk_payment_id": ldk_id }),
            );
            match db.rekey_pending_stability_payment(&info.payment_id, &ldk_id) {
                Ok(_) => info.payment_id = ldk_id,
                Err(e) => audit_event(
                    "STABILITY_PAYMENT_REKEY_FAILED",
                    json!({ "ldk_payment_id": ldk_id, "error": e.to_string() }),
                ),
            }
        }
        Ok(_) => {}
        Err(StabilitySendError::OutcomeUnknown(error)) => {
            // Keep the claim: a terminal event resolves it, and the lost-claim recovery covers no event.
            audit_event(
                "STABILITY_PAYMENT_OUTCOME_UNKNOWN",
                json!({ "payment_id": info.payment_id, "error": error }),
            );
        }
        Err(StabilitySendError::NotSent(error)) => {
            audit_event(
                "STABILITY_PAYMENT_FAILED",
                json!({
                    "amount_msats": info.amount_msat,
                    "error": error,
                    "counterparty": info.counterparty,
                }),
            );
            if let Err(e) = db.fail_pending_stability_payment(&info.payment_id, false) {
                audit_event(
                    "STABILITY_PAYMENT_FAILURE_PERSIST_FAILED",
                    json!({ "payment_id": info.payment_id, "error": e.to_string() }),
                );
            }
            return None;
        }
    }
    sc.payment_made = true;
    sc.last_stability_payment = now;
    // Native is recomputed on the next balance refresh: the HTLC is still in flight.
    sc.backing_sats = info.backing_sats_after;
    Some(info)
}

/// Same fee budget as LDK's default, 1% + 50 sats, passed explicitly so routing enforces it.
pub fn lightning_fee_limit_msat(amount_msat: u64) -> u64 {
    amount_msat / 100 + 50_000
}

/// Route with the fee budget the allocation guard admitted the payment under.
pub fn route_parameters(amount_msat: u64) -> RouteParametersConfig {
    RouteParametersConfig {
        max_total_routing_fee_msat: Some(lightning_fee_limit_msat(amount_msat)),
        ..RouteParametersConfig::default()
    }
}

/// The most sats a Lightning send can remove from the channel, bounded fees included.
pub fn maximum_lightning_debit_sats(amount_msat: u64) -> Option<u64> {
    if amount_msat == 0 {
        return None; // unknown amount: backing remains reachable
    }
    let total = amount_msat.checked_add(lightning_fee_limit_msat(amount_msat))?;
    Some(total.div_ceil(1000))
}

/// A shortfall is owed BY the LSP. It is not unpaid LSP surplus at risk of withdrawal.
fn payable_surplus_is_actionable(backing_sats: u64, expected_usd: f64, price: f64) -> bool {
    let surplus_usd = backing_sats as f64 / SATS_IN_BTC as f64 * price - expected_usd;
    if expected_usd < 0.01 {
        return surplus_usd >= STABILITY_THRESHOLD_USD;
    }
    surplus_usd >= STABILITY_THRESHOLD_USD
        && surplus_usd / expected_usd * 100.0 >= STABILITY_THRESHOLD_PERCENT
}

/// Admit a channel spend only after payable surplus has settled. Native is a sat allocation, so
/// neither drift nor a price outage can block a spend whose amount AND bounded fees fit inside
/// it. Without a bound (splices), the complete allocation is checked because fees also consume
/// channel funds. The caller must serialize this check with settlement submission and reject
/// pending payments.
pub fn check_outgoing_allocation(
    sc: &StableChannel,
    price: f64,
    maximum_debit_sats: Option<u64>,
) -> Result<(), &'static str> {
    if sc.expected_usd.0 == 0.0 && sc.backing_sats == 0 {
        return Ok(());
    }
    if !sc.expected_usd.0.is_finite()
        || sc.expected_usd.0 < 0.0
        || sc.backing_sats > sc.stable_receiver_btc.sats
    {
        return Err("Waiting for the channel balance to update. Please try again shortly.");
    }
    let native_sats = sc.stable_receiver_btc.sats - sc.backing_sats;
    if maximum_debit_sats.is_some_and(|debit| debit <= native_sats) {
        return Ok(());
    }
    if sc.backing_sats == 0 {
        return Ok(());
    }
    if !price.is_finite() || price <= 0.0 {
        return Err("Waiting for a fresh price before sending. Please try again shortly.");
    }
    if payable_surplus_is_actionable(sc.backing_sats, sc.expected_usd.0, price) {
        return Err("A stability payment must settle before sending. Please try again shortly.");
    }
    Ok(())
}

/// Apply an allocation already derived by this peer.
///
/// Callers must not pass a counterparty-supplied `backing_sats` value here. The shared contract is
/// `expected_usd`; each peer derives and persists its own backing using its own price.
pub fn apply_trade_allocation(sc: &mut StableChannel, new_expected_usd: f64, backing_sats: u64) {
    sc.expected_usd = USD::from_f64(new_expected_usd);
    sc.backing_sats = backing_sats;
    sc.native_sats = sc.stable_receiver_btc.sats.saturating_sub(sc.backing_sats);
    recompute_native(sc);
}

/// Apply a trade without repricing its existing backing.
///
/// Only the target delta is converted at the current price. Returns `false` and leaves the
/// allocation untouched if the delta cannot be represented safely or fit the live balance.
#[must_use]
pub fn apply_trade(sc: &mut StableChannel, new_expected_usd: f64, price: f64) -> bool {
    let new_expected_usd = normalize_trade_expected_usd(new_expected_usd);
    let Some(backing_sats) = trade_backing_after_delta(
        sc.stable_receiver_btc.sats,
        sc.backing_sats,
        sc.expected_usd.0,
        new_expected_usd,
        price,
    ) else {
        return false;
    };
    apply_trade_allocation(sc, new_expected_usd, backing_sats);
    true
}

/// Get the current BTC/USD price, preferring cached value when available
pub fn get_current_price(agent: &Agent) -> f64 {
    // Accounting callers may only consume a fresh, non-quarantined cache value.
    let cached_price = get_fresh_cached_price_no_fetch();

    // Use the cached price if valid
    if cached_price > 0.0 {
        return cached_price;
    }

    crate::price_feeds::get_latest_price(agent).unwrap_or(0.0)
}

/// The sats effectively backing the stable position. If `backing_sats` was left unset (0) while a
/// peg is active, derive it from the target so the stable value is the peg — never the full channel
/// balance (which would count native BTC and fire a spurious PAY that drains it to the LSP).
fn effective_backing_sats(
    backing_sats: u64,
    expected_usd: f64,
    price: f64,
    receiver_sats: u64,
) -> u64 {
    if backing_sats > 0 || expected_usd < 0.01 || price <= 0.0 {
        backing_sats
    } else {
        ((expected_usd / price * 100_000_000.0) as u64).min(receiver_sats)
    }
}

pub fn channel_exists(node: &Node, user_channel_id: u128) -> bool {
    let channels = node.list_channels();
    channels
        .iter()
        .any(|c| c.user_channel_id.0 == user_channel_id)
}

// Can run in backgound
pub fn update_balances<'update_balance_lifetime>(
    node: &Node,
    sc: &'update_balance_lifetime mut StableChannel,
) -> (bool, &'update_balance_lifetime mut StableChannel) {
    // Cache-only so no caller (incl. the UI thread) blocks on the network; the background loop owns refreshes.
    let cached = get_fresh_cached_price_no_fetch();
    if cached > 0.0 {
        sc.latest_price = cached;
    }

    // --- Update On-chain ---
    let balances = node.list_balances();
    sc.onchain_btc = Bitcoin::from_sats(balances.total_onchain_balance_sats);
    sc.onchain_usd = USD::from_bitcoin(sc.onchain_btc, sc.latest_price);

    let channels = node.list_channels();
    let matching_channel = if sc.user_channel_id == 0 {
        channels.first()
    } else {
        channels
            .iter()
            .find(|c| c.user_channel_id.0 == sc.user_channel_id)
    };

    if let Some(channel) = matching_channel {
        if sc.user_channel_id == 0 {
            sc.user_channel_id = channel.user_channel_id.0;
            sc.channel_id = channel.channel_id;
            println!(
                "Set active channel: user_channel_id={}, channel_id={}",
                sc.user_channel_id, sc.channel_id
            );
        }
        // Always keep channel_id current (it changes on splice)
        sc.channel_id = channel.channel_id;

        // Skip balance update if channel is not ready yet — during ChannelPending,
        // outbound_capacity_msat is 0, which produces a misleading near-zero balance.
        if !channel.is_channel_ready {
            return (true, sc);
        }

        let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
        let our_balance_sats =
            (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
        let their_balance_sats = channel.channel_value_sats.saturating_sub(our_balance_sats);

        if sc.is_stable_receiver {
            sc.stable_receiver_btc = Bitcoin::from_sats(our_balance_sats);
            sc.stable_provider_btc = Bitcoin::from_sats(their_balance_sats);
        } else {
            sc.stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
            sc.stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
        }

        sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
        sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);

        // Native BTC is the portion not backing the stable position
        let native_sats = sc.stable_receiver_btc.sats.saturating_sub(sc.backing_sats);
        sc.native_sats = native_sats;
        sc.native_channel_btc = Bitcoin::from_sats(native_sats);

        audit_event(
            "BALANCE_UPDATE",
            json!({
                "user_channel_id": format!("{}", sc.user_channel_id),
                "stable_receiver_btc": sc.stable_receiver_btc.to_string(),
                "stable_provider_btc": sc.stable_provider_btc.to_string(),
                "stable_receiver_usd": sc.stable_receiver_usd.to_string(),
                "stable_provider_usd": sc.stable_provider_usd.to_string(),
                "native_channel_btc": sc.native_channel_btc.to_string(),
                "btc_price": sc.latest_price
            }),
        );

        return (true, sc);
    }

    println!(
        "No matching channel found for user_channel_id: {}",
        sc.user_channel_id
    );
    (true, sc)
}

/// Information about a stability payment that was sent
#[derive(Debug, Clone)]
pub struct StabilityPaymentInfo {
    pub settlement_id: String,
    pub payment_id: String,
    pub amount_msat: u64,
    pub counterparty: String,
    pub btc_price: f64,
    pub backing_sats_before: u64,
    pub backing_sats_after: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StabilityPaymentDirection {
    UserToLsp,
    LspToUser,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StabilityPaymentPayload {
    #[serde(rename = "type")]
    pub kind: String,
    pub settlement_id: String,
    pub channel_id: String,
    pub amount_msat: u64,
    pub direction: StabilityPaymentDirection,
    pub expected_usd: f64,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StabilitySignedEnvelope {
    pub payload: String,
    pub signature: String,
}

pub fn new_stability_settlement_id() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn is_lower_hex_32(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[allow(clippy::too_many_arguments)]
pub fn build_stability_payment_payload(
    settlement_id: &str,
    channel_id: &str,
    amount_msat: u64,
    direction: StabilityPaymentDirection,
    expected_usd: f64,
    created_at: u64,
    expires_at: u64,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&StabilityPaymentPayload {
        kind: STABILITY_PAYMENT_MESSAGE_TYPE.to_owned(),
        settlement_id: settlement_id.to_owned(),
        channel_id: channel_id.to_owned(),
        amount_msat,
        direction,
        expected_usd,
        created_at,
        expires_at,
    })
}

pub fn build_stability_signed_envelope(
    payload: String,
    signature: String,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&StabilitySignedEnvelope { payload, signature })
}

pub fn parse_stability_signed_envelope(raw: &str) -> Option<StabilitySignedEnvelope> {
    serde_json::from_str(raw).ok()
}

pub fn parse_stability_payment_payload(payload: &str) -> Option<StabilityPaymentPayload> {
    let payment: StabilityPaymentPayload = serde_json::from_str(payload).ok()?;
    if payment.kind != STABILITY_PAYMENT_MESSAGE_TYPE
        || !is_lower_hex_32(&payment.settlement_id)
        || !is_lower_hex_32(&payment.channel_id)
        || payment.amount_msat == 0
        || !payment.amount_msat.is_multiple_of(1000)
        || !payment.expected_usd.is_finite()
        || payment.expected_usd < 0.0
        || payment.created_at > payment.expires_at
        || payment.expires_at.saturating_sub(payment.created_at)
            > STABILITY_PAYMENT_AUTH_TTL_SECS
    {
        return None;
    }
    Some(payment)
}

pub fn stability_payment_is_fresh(payment: &StabilityPaymentPayload, now: u64) -> bool {
    payment.created_at <= now.saturating_add(STABILITY_PAYMENT_CLOCK_SKEW_SECS)
        && now <= payment.expires_at.saturating_add(STABILITY_PAYMENT_CLOCK_SKEW_SECS)
}

/// Apply a wallet-to-LSP stability payment to the LSP's local allocation.
///
/// The paid sats are authoritative, but PR #231's local-equilibrium floor remains in force: a
/// payment may settle an above-par surplus, never manufacture a below-par claim at the LSP's
/// price. The final live-balance clamp preserves the allocation invariant.
pub fn backing_after_user_to_lsp_stability(
    current_backing_sats: u64,
    expected_usd: f64,
    price: f64,
    amount_sats: u64,
    live_receiver_sats: u64,
) -> Option<u64> {
    if !expected_usd.is_finite()
        || expected_usd < 0.0
        || !price.is_finite()
        || price <= 0.0
        || amount_sats == 0
    {
        return None;
    }
    let equilibrium_f = expected_usd / price * SATS_IN_BTC as f64;
    if !equilibrium_f.is_finite() || equilibrium_f >= u64::MAX as f64 {
        return None;
    }
    let equilibrium = equilibrium_f.floor() as u64;
    let settled = if current_backing_sats > equilibrium {
        current_backing_sats
            .saturating_sub(amount_sats)
            .max(equilibrium)
    } else {
        current_backing_sats
    };
    Some(settled.min(live_receiver_sats))
}

/// Apply an LSP-to-wallet stability payment to the wallet's local allocation.
///
/// The received sats move backing only toward the wallet's equilibrium at its own price. Any
/// overpayment remains native, and an already above-par allocation is never reduced by an incoming
/// payment. This lets peers safely retain independent price feeds without comparing `f64` state
/// bit-for-bit.
pub fn backing_after_lsp_to_user_stability(
    current_backing_sats: u64,
    expected_usd: f64,
    price: f64,
    amount_sats: u64,
    live_receiver_sats: u64,
) -> Option<u64> {
    if !expected_usd.is_finite()
        || expected_usd < 0.0
        || !price.is_finite()
        || price <= 0.0
        || amount_sats == 0
        || current_backing_sats > live_receiver_sats
    {
        return None;
    }
    let equilibrium_f = expected_usd / price * SATS_IN_BTC as f64;
    if !equilibrium_f.is_finite() || equilibrium_f >= u64::MAX as f64 {
        return None;
    }
    let equilibrium = (equilibrium_f.floor() as u64).min(live_receiver_sats);
    if current_backing_sats >= equilibrium {
        return Some(current_backing_sats);
    }
    current_backing_sats
        .checked_add(amount_sats)
        .map(|backing| backing.min(equilibrium))
}

/// Whether the Lightning wallet's last successful chain sync is recent enough to send a
/// stability payment. A missing timestamp (never synced), a timestamp in the future (clock
/// skew), or one older than `max_age_secs` all block the send: paying on a stale chain tip
/// understates outbound HTLC expiry, which LDK later force-closes on. Exactly `max_age_secs`
/// old is accepted.
pub fn lightning_sync_is_fresh(
    latest_sync_timestamp: Option<u64>,
    now_secs: u64,
    max_age_secs: u64,
) -> bool {
    match latest_sync_timestamp {
        Some(ts) => ts <= now_secs && now_secs - ts <= max_age_secs,
        None => false,
    }
}

/// Check and enforce stability for a channel.
///
/// The stability logic keeps the user's expected_usd amount stable:
/// - expected_usd is the USD amount to keep stable
/// - The rest of the channel balance floats with BTC price
///
/// Returns Some(StabilityPaymentInfo) if a payment was sent, None otherwise.
pub fn check_stability(
    node: &Node,
    db: &Database,
    sc: &mut StableChannel,
    price: f64,
) -> Option<StabilityPaymentInfo> {
    if !price.is_finite() || price <= 0.0 {
        audit_event(
            "STABILITY_SKIP",
            json!({
                "reason": "caller supplied no valid current price",
                "price": price,
            }),
        );
        return None;
    }
    let current_price = price;

    sc.latest_price = current_price;
    let (success, _) = update_balances(node, sc);

    if !success {
        audit_event(
            "BALANCE_UPDATE_FAILED",
            json!({
                "user_channel_id": format!("{}", sc.user_channel_id)
            }),
        );
        return None;
    }

    // Do NOT recalculate backing_sats here.
    // backing_sats is set at trade time (expected_usd / price * 1e8) and stays fixed.
    // As BTC price moves, stable_usd_value = backing_sats * new_price will drift
    // from expected_usd, triggering a stability payment to rebalance.

    // A zero/sub-cent claim can still carry a final payment owed to the LSP.
    if sc.expected_usd.0 < 0.01 && sc.backing_sats == 0 {
        audit_event(
            "STABILITY_SKIP",
            json!({
                "user_channel_id": format!("{}", sc.user_channel_id),
                "reason": "expected_usd is too small",
                "expected_usd": sc.expected_usd.0
            }),
        );
        return None;
    }

    // The target is expected_usd
    let target_usd = sc.expected_usd.0;

    // Repair an unset backing (0 with a live peg + price) by deriving it from the target, so the
    // stable portion is valued at the peg — NOT the full channel balance, which would count native
    // BTC and trigger a spurious PAY that drains it to the LSP.
    sc.backing_sats = effective_backing_sats(
        sc.backing_sats,
        target_usd,
        current_price,
        sc.stable_receiver_btc.sats,
    );
    if sc.backing_sats > sc.stable_receiver_btc.sats
        && has_pending_outbound_lightning_payment(node)
    {
        // Emit the common safety audit and stop the whole stability decision. Continuing with the
        // temporarily reduced capacity could initiate another payment from a false drift signal.
        let _ = repair_overbacked_allocation_if_safe(node, db, sc, current_price);
        return None;
    }
    if repair_overbacked_allocation_if_safe(node, db, sc, current_price).is_some() {
        return None;
    }
    sc.native_sats = sc
        .stable_receiver_btc
        .sats
        .saturating_sub(sc.backing_sats);
    recompute_native(sc);

    // Value of the stable portion only (excludes native BTC).
    let stable_usd_value = if sc.backing_sats > 0 {
        (sc.backing_sats as f64 / 100_000_000.0) * current_price
    } else {
        // Backing still unset (degenerate: no price / sub-cent peg) — value at the peg, never the
        // full channel balance.
        target_usd
    };

    // Calculate deviation: how much the stable portion has drifted from target
    // Due to price changes, the BTC backing the stable portion may be worth more or less
    let dollars_from_par = USD::from_f64(stable_usd_value - target_usd);
    let percent_from_par = (dollars_from_par.0 / target_usd.max(0.01) * 100.0).abs();
    let is_receiver_below_expected = stable_usd_value < target_usd;

    let action = if percent_from_par < STABILITY_THRESHOLD_PERCENT
        || dollars_from_par.0.abs() < STABILITY_THRESHOLD_USD
    {
        "STABLE"
    } else if sc.risk_level > MAX_RISK_LEVEL {
        "HIGH_RISK_NO_ACTION"
    } else if (sc.is_stable_receiver && is_receiver_below_expected)
        || (!sc.is_stable_receiver && !is_receiver_below_expected)
    {
        "CHECK_ONLY"
    } else {
        "PAY"
    };

    audit_event(
        "STABILITY_CHECK",
        json!({
            "expected_usd": target_usd,
            "stable_usd_value": stable_usd_value,
            "backing_sats": sc.backing_sats,
            "native_sats": sc.native_sats,
            "total_receiver_usd": sc.stable_receiver_usd.0,
            "percent_from_par": percent_from_par,
            "btc_price": sc.latest_price,
            "action": action,
            "is_stable_receiver": sc.is_stable_receiver,
            "risk_level": sc.risk_level
        }),
    );

    if action != "PAY" {
        return None;
    }

    // Safety check: if an in-flight HTLC is temporarily inflating the receiver balance
    // above backing + native, skip — the drift is transient and will resolve when the
    // HTLC settles. Only relevant on the LSP side (!is_stable_receiver) when the
    // receiver appears above par (price rose, LSP should be paid).
    if !sc.is_stable_receiver && !is_receiver_below_expected {
        let expected_sats = sc.backing_sats + sc.native_sats;
        if sc.stable_receiver_btc.sats > expected_sats + expected_sats / 100 {
            audit_event(
                "STABILITY_SKIP_HTLC_SAFETY",
                json!({
                    "user_channel_id": format!("{}", sc.user_channel_id),
                    "receiver_sats": sc.stable_receiver_btc.sats,
                    "expected_sats": expected_sats,
                    "backing_sats": sc.backing_sats,
                    "native_sats": sc.native_sats,
                    "reason": "receiver balance >1% above expected — likely in-flight HTLC"
                }),
            );
            return None;
        }
    }

    // Enforce cooldown between stability payments
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    audit_event(
        "STABILITY_PAY_COOLDOWN_CHECK",
        json!({
            "user_channel_id": format!("{}", sc.user_channel_id),
            "now": now,
            "last_stability_payment": sc.last_stability_payment,
            "seconds_since": now - sc.last_stability_payment,
            "cooldown_secs": STABILITY_PAYMENT_COOLDOWN_SECS,
            "will_block": sc.last_stability_payment > 0 && (now - sc.last_stability_payment) < STABILITY_PAYMENT_COOLDOWN_SECS as i64,
        }),
    );

    if sc.last_stability_payment > 0
        && (now - sc.last_stability_payment) < STABILITY_PAYMENT_COOLDOWN_SECS as i64
    {
        audit_event(
            "STABILITY_COOLDOWN",
            json!({
                "user_channel_id": format!("{}", sc.user_channel_id),
                "seconds_since_last": now - sc.last_stability_payment,
                "cooldown_secs": STABILITY_PAYMENT_COOLDOWN_SECS,
            }),
        );
        return None;
    }

    // Stable allocations are sat-denominated. Send and sign the same exact whole-sat value that
    // both peers can apply to backing without fractional-sat ambiguity.
    let amt = (USD::to_msats(dollars_from_par, sc.latest_price) / 1000) * 1000;
    if amt == 0 {
        return None;
    }

    // Send-boundary chain-freshness gate (see #243): never pay on a stale chain tip.
    // Skipping here leaves no durable state — the cooldown is only set after a real send —
    // so the next stability tick retries once LDK's background sync catches up.
    let latest_sync = node.status().latest_lightning_wallet_sync_timestamp;
    if !lightning_sync_is_fresh(latest_sync, now.max(0) as u64, STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS)
    {
        audit_event(
            "STABILITY_SKIP",
            json!({
                "user_channel_id": format!("{}", sc.user_channel_id),
                "reason": "stale_lightning_sync",
                "latest_lightning_wallet_sync_timestamp": latest_sync,
                "sync_age_secs": latest_sync.map(|ts| (now.max(0) as u64).saturating_sub(ts)),
                "max_age_secs": STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS,
            }),
        );
        return None;
    }

    let settlement_id = new_stability_settlement_id();
    let created_at = now.max(0) as u64;
    let expires_at = created_at.saturating_add(STABILITY_PAYMENT_AUTH_TTL_SECS);
    let payload = match build_stability_payment_payload(
        &settlement_id,
        &sc.channel_id.to_string(),
        amt,
        StabilityPaymentDirection::UserToLsp,
        sc.expected_usd.0,
        created_at,
        expires_at,
    ) {
        Ok(payload) => payload,
        Err(error) => {
            audit_event(
                "STABILITY_PAYMENT_SERIALIZE_FAILED",
                json!({
                    "user_channel_id": format!("{}", sc.user_channel_id),
                    "settlement_id": settlement_id,
                    "amount_msat": amt,
                    "error": error.to_string(),
                }),
            );
            return None;
        }
    };
    let signature = node.sign_message(payload.as_bytes());
    let signed_envelope = match build_stability_signed_envelope(payload, signature) {
        Ok(envelope) => envelope,
        Err(error) => {
            audit_event(
                "STABILITY_PAYMENT_SERIALIZE_FAILED",
                json!({
                    "user_channel_id": format!("{}", sc.user_channel_id),
                    "settlement_id": settlement_id,
                    "amount_msat": amt,
                    "stage": "envelope",
                    "error": error.to_string(),
                }),
            );
            return None;
        }
    };
    // Signed record only: the unauthenticated [1] marker is no longer sent by any client.
    let signed_record = ldk_node::CustomTlvRecord {
        type_num: crate::constants::SIGNED_STABILITY_TLV_TYPE,
        value: signed_envelope.into_bytes(),
    };
    // The id comes from a preimage chosen here, so the durable claim carries it before LDK
    // sees the payment and could emit a terminal event for it.
    let mut preimage = [0u8; 32];
    rand::rng().fill_bytes(&mut preimage);
    let preimage = PaymentPreimage(preimage);
    let counterparty = sc.counterparty;
    let previous_backing = sc.backing_sats;
    let info = StabilityPaymentInfo {
        settlement_id,
        payment_id: stability_payment_id(&preimage).to_string(),
        amount_msat: amt,
        counterparty: counterparty.to_string(),
        btc_price: sc.latest_price,
        backing_sats_before: previous_backing,
        // Debit only the whole sats actually sent, preserving any rounding residue.
        backing_sats_after: previous_backing.saturating_sub(amt / 1000),
    };
    send_claimed_stability_payment(db, sc, info, now, || {
        node.spontaneous_payment()
            .send_with_preimage_and_custom_tlvs(amt, counterparty, vec![signed_record], preimage, None)
            .map(|payment_id| payment_id.to_string())
            .map_err(|e| match e {
                ldk_node::NodeError::PersistenceFailed => StabilitySendError::OutcomeUnknown(e.to_string()),
                _ => StabilitySendError::NotSent(e.to_string()),
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ldk_node::lightning::ln::channelmanager::PaymentId;
    use ldk_node::lightning::types::payment::PaymentPreimage;

    #[test]
    fn final_claim_spends_preserve_lsp_surplus_in_every_reconciliation_path() {
        for native_before in [0, 5_000] {
            for path in 0..4 {
                let mut sc = test_sc(10.0, 50_000.0, 10_000);
                let result = match path {
                    0 => reconcile_outgoing(&mut sc, 100_000.0),
                    1 => deduct_outgoing_from_snapshot(&mut sc, 20_000 + native_before, 20_000,
                        10_000 + native_before, 100_000.0),
                    2 => repair_overbacked_allocation(&mut sc, 100_000.0).map(|r| r.usd_deducted),
                    _ => reconcile_forwarded(&mut sc, 20_000 + native_before,
                        10_000 + native_before, 100_000.0),
                };
                assert_eq!(result, Some(10.0));
                assert_eq!(sc.expected_usd.0, 0.0);
                assert_eq!(sc.backing_sats, 10_000);
                assert_eq!(sc.native_channel_btc.sats, 0);
                assert!(reconcile_outgoing(&mut sc, 100_000.0).is_none());
                assert!(repair_overbacked_allocation(&mut sc, 100_000.0).is_none());
                assert!(check_outgoing_allocation(&sc, 100_000.0, None).is_err());
                assert_eq!(backing_after_user_to_lsp_stability(10_000, 0.0, 100_000.0, 10_000, 0), Some(0));
            }
        }
    }

    #[test]
    fn channel_spends_wait_for_payable_surplus_to_settle_and_a_trusted_price() {
        let mut sc = test_sc(10.0, 100_000.0, 100_000);
        for price in [110_000.0, 0.0, f64::NAN, f64::INFINITY] {
            assert!(check_outgoing_allocation(&sc, price, None).is_err());
        }
        assert!(check_outgoing_allocation(&sc, 90_000.0, None).is_ok(), "the LSP owes the shortfall");
        assert!(check_outgoing_allocation(&sc, 100_000.0, None).is_ok());
        assert!(check_outgoing_allocation(&sc, 102_000.0, None).is_ok(), "sub-$0.25 deadband remains allowed");
        sc.expected_usd = USD(0.00085);
        sc.backing_sats = 477;
        assert!(check_outgoing_allocation(&sc, 105_000.0, None).is_err());
        sc.expected_usd = USD(0.0);
        sc.backing_sats = 249;
        assert!(check_outgoing_allocation(&sc, 100_000.0, None).is_ok());
        sc.backing_sats = 250;
        assert!(check_outgoing_allocation(&sc, 100_000.0, None).is_err());
        sc.backing_sats = 0;
        assert!(check_outgoing_allocation(&sc, 0.0, None).is_ok(), "native-only wallet needs no settlement price");
    }

    #[test]
    fn native_spends_including_fees_bypass_surplus_and_price_checks() {
        // 10,000 backing sats and 90,000 native sats.
        let sc = test_sc(10.0, 100_000.0, 100_000);
        for price in [110_000.0, 90_000.0, 0.0, f64::NAN, f64::INFINITY] {
            assert!(check_outgoing_allocation(&sc, price, Some(90_000)).is_ok());
        }
        assert!(check_outgoing_allocation(&sc, 110_000.0, Some(90_001)).is_err());
        assert!(check_outgoing_allocation(&sc, 0.0, Some(90_001)).is_err());
        assert_eq!(maximum_lightning_debit_sats(0), None);
        assert_eq!(maximum_lightning_debit_sats(1_000_000), Some(1_060));
        assert_eq!(maximum_lightning_debit_sats(1_000_001), Some(1_061));
        assert_eq!(route_parameters(1_000_000).max_total_routing_fee_msat, Some(60_000));
    }

    #[test]
    fn zero_target_surplus_is_protected_but_native_sats_remain_spendable() {
        let mut sc = test_sc(0.0, 100_000.0, 3_000);
        sc.backing_sats = 1_000;
        assert!(check_outgoing_allocation(&sc, 0.0, Some(2_000)).is_ok());
        assert!(check_outgoing_allocation(&sc, 100_000.0, Some(2_001)).is_err());
    }

    fn details(id: u8, status: PaymentStatus) -> PaymentDetails {
        use ldk_node::lightning::types::payment::PaymentHash;
        use ldk_node::payment::{PaymentDirection, PaymentKind};
        PaymentDetails {
            id: PaymentId([id; 32]),
            kind: PaymentKind::Spontaneous { hash: PaymentHash([0; 32]), preimage: None },
            amount_msat: Some(1_000_000),
            fee_paid_msat: None,
            direction: PaymentDirection::Outbound,
            status,
            latest_update_timestamp: 0,
        }
    }

    /// Books worth $11 against a $10 target on channel 7, saved, with a $1 claim prepared.
    fn claimed_channel(db: &Database, payment_id: &str) -> (StableChannel, StabilityPaymentInfo) {
        let mut sc = test_sc(10.0, 100_000.0, 20_000);
        sc.user_channel_id = 7;
        sc.backing_sats = 11_000;
        sc.native_sats = 9_000;
        db.save_channel(&sc.channel_id.to_string(), "7", 10.0, 11_000, 9_000, None).unwrap();
        let info = StabilityPaymentInfo {
            settlement_id: "11".repeat(32),
            payment_id: payment_id.to_string(),
            amount_msat: 1_000_000,
            counterparty: sc.counterparty.to_string(),
            btc_price: 100_000.0,
            backing_sats_before: 11_000,
            backing_sats_after: 10_000,
        };
        (sc, info)
    }

    #[test]
    fn stability_payment_id_is_known_before_the_send() {
        // LDK derives the id the same way: sha256 of the preimage.
        assert_eq!(
            stability_payment_id(&PaymentPreimage([0u8; 32])).to_string(),
            "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925"
        );
    }

    #[test]
    fn stability_claim_is_durable_before_the_send_and_rolls_back_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let (mut sc, info) = claimed_channel(&db, &"aa".repeat(32));

        // A claim that cannot be persisted never reaches LDK.
        let unpersistable = StabilityPaymentInfo { backing_sats_before: u64::MAX, ..info.clone() };
        let reached_ldk = std::cell::Cell::new(false);
        assert!(send_claimed_stability_payment(&db, &mut sc, unpersistable, 1, || {
            reached_ldk.set(true);
            Ok("aa".repeat(32))
        })
        .is_none());
        assert!(!reached_ldk.get());
        assert_eq!(sc.backing_sats, 11_000);

        // LDK can emit a terminal event before `send` returns: the row must already classify it.
        assert!(send_claimed_stability_payment(&db, &mut sc, info.clone(), 1, || {
            assert!(db.is_stability_payment(&info.payment_id).unwrap());
            Err(StabilitySendError::NotSent("route not found".into()))
        })
        .is_none());
        assert_eq!(sc.backing_sats, 11_000);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 11_000);
        assert!(!db.has_pending_channel_send().unwrap(), "a synchronous failure rolls the claim back");

        let accepted = StabilityPaymentInfo { payment_id: "bb".repeat(32), ..info };
        let sent = send_claimed_stability_payment(&db, &mut sc, accepted, 42, || Ok("bb".repeat(32)));
        assert_eq!(sent.unwrap().payment_id, "bb".repeat(32));
        assert_eq!(sc.backing_sats, 10_000);
        assert_eq!(sc.last_stability_payment, 42);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 10_000);
        assert!(db.is_stability_payment(&"bb".repeat(32)).unwrap());
        assert!(db.has_pending_channel_send().unwrap());
    }

    #[test]
    fn a_send_error_after_dispatch_keeps_the_claim_and_the_debit() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let (mut sc, info) = claimed_channel(&db, &"cc".repeat(32));
        // LDK writes its payment store after dispatching, so this error does not mean "not sent".
        let sent = send_claimed_stability_payment(&db, &mut sc, info, 7, || {
            Err(StabilitySendError::OutcomeUnknown("Failed to persist data.".into()))
        });
        assert_eq!(sent.unwrap().payment_id, "cc".repeat(32));
        assert_eq!(sc.backing_sats, 10_000);
        assert_eq!(sc.last_stability_payment, 7);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 10_000);
        assert!(db.has_pending_channel_send().unwrap(), "the terminal event must still find its row");
        assert!(db.complete_pending_stability_payment(&"cc".repeat(32), "", None).unwrap());
    }

    #[test]
    fn a_payment_id_ldk_did_not_derive_our_way_rekeys_the_claim() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let (mut sc, info) = claimed_channel(&db, &"dd".repeat(32));
        let sent = send_claimed_stability_payment(&db, &mut sc, info, 1, || Ok("ee".repeat(32)));
        // Events carry LDK's id, so the row that classifies them must carry it too.
        assert_eq!(sent.unwrap().payment_id, "ee".repeat(32));
        assert!(db.is_stability_payment(&"ee".repeat(32)).unwrap());
        assert!(!db.is_stability_payment(&"dd".repeat(32)).unwrap());
        assert_eq!(sc.backing_sats, 10_000);
    }

    #[test]
    fn lost_stability_claims_roll_back_after_the_grace_period() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let id = format!("{}", PaymentId([9; 32]));
        let (mut sc, info) = claimed_channel(&db, &id);
        assert!(send_claimed_stability_payment(&db, &mut sc, info, 1, || Ok(id.clone())).is_some());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let later = now + LOST_LDK_RECORD_TIMEOUT_SECS + 1;

        // A crash between the claim and the send leaves a row LDK never heard of.
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &[], now).unwrap(), 0);
        let pending = [details(9, PaymentStatus::Pending)];
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &pending, later).unwrap(), 0);
        let succeeded = [details(9, PaymentStatus::Succeeded)];
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &succeeded, now).unwrap(), 0);
        assert!(db.has_pending_channel_send().unwrap());
        assert_eq!(sc.backing_sats, 10_000);

        // The rollback reaches the saved books and the live channel in one step.
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &[], later).unwrap(), 1);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 11_000);
        assert!(!db.has_pending_channel_send().unwrap());
        assert_eq!(sc.backing_sats, 11_000);
        assert_eq!(sc.last_stability_payment, 0);
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &[], later).unwrap(), 0);
        assert_eq!(sc.backing_sats, 11_000, "only the optimistic allocation is undone");
    }

    #[test]
    fn a_success_after_a_lost_record_rollback_reapplies_the_debit_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let id = format!("{}", PaymentId([9; 32]));
        let (mut sc, info) = claimed_channel(&db, &id);
        assert!(send_claimed_stability_payment(&db, &mut sc, info, 1, || Ok(id.clone())).is_some());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let later = now + LOST_LDK_RECORD_TIMEOUT_SECS + 1;
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &[], later).unwrap(), 1);
        assert_eq!(sc.backing_sats, 11_000);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 11_000);

        // The payment had left after all: its success event arrives after the rollback.
        assert!(!db.complete_pending_stability_payment(&id, "hash", Some(5)).unwrap());
        let redebit = db.settle_rolled_back_stability_payment(&id, "hash", Some(5)).unwrap().unwrap();
        assert_eq!(redebit.debit_sats, 1_000);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 10_000);
        assert_eq!(db.get_recent_payments(1).unwrap()[0].status, "completed");
        assert!(apply_stability_redebit(&mut sc, &redebit));
        assert_eq!(sc.backing_sats, 10_000);

        // A replayed event debits nothing more.
        assert!(db.settle_rolled_back_stability_payment(&id, "hash", Some(5)).unwrap().is_none());
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 10_000);
    }

    #[test]
    fn a_rolled_back_claim_is_settled_from_the_balance_drop_not_repaired_as_an_overspend() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let id = format!("{}", PaymentId([9; 32]));
        let (mut sc, info) = claimed_channel(&db, &id);
        sc.stable_receiver_btc = Bitcoin::from_sats(11_000); // fully stable: no native sats
        sc.native_sats = 0;
        assert!(send_claimed_stability_payment(&db, &mut sc, info, 1, || Ok(id.clone())).is_some());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let later = now + LOST_LDK_RECORD_TIMEOUT_SECS + 1;
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &[], later).unwrap(), 1);
        assert_eq!(sc.backing_sats, 11_000);

        // The payment had left: the live balance is down by its amount.
        sc.stable_receiver_btc = Bitcoin::from_sats(10_000);
        assert_eq!(settle_lost_claims_from_balance(&db, &mut sc), 1_000);
        assert_eq!(sc.backing_sats, 10_000);
        assert_eq!(sc.expected_usd.0, 10.0);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 10_000);
        assert!(repair_overbacked_allocation(&mut sc, 100_000.0).is_none(), "nothing is over-backed");
        assert_eq!(sc.expected_usd.0, 10.0);

        // The late success only completes history; the debit is not applied twice.
        assert!(db.settle_rolled_back_stability_payment(&id, "hash", None).unwrap().is_none());
        assert_eq!(db.get_recent_payments(1).unwrap()[0].status, "completed");
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 10_000);
        assert_eq!(settle_lost_claims_from_balance(&db, &mut sc), 0);
    }

    #[test]
    fn a_lost_record_claim_the_balance_never_dropped_for_is_expired_not_later_attributed() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let id = format!("{}", PaymentId([9; 32]));
        let (mut sc, info) = claimed_channel(&db, &id);
        sc.stable_receiver_btc = Bitcoin::from_sats(11_000); // fully stable: no native sats
        sc.native_sats = 0;
        assert!(send_claimed_stability_payment(&db, &mut sc, info, 1, || Ok(id.clone())).is_some());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let later = now + LOST_LDK_RECORD_TIMEOUT_SECS + 1;
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &[], later).unwrap(), 1);

        // The payment never left: the balance still covers the books. That one evaluation consumes
        // the claim, so a drop that appears later is an ordinary overspend, never this claim.
        assert_eq!(settle_lost_claims_from_balance(&db, &mut sc), 0);
        sc.stable_receiver_btc = Bitcoin::from_sats(9_500);
        assert_eq!(settle_lost_claims_from_balance(&db, &mut sc), 0);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 11_000);

        // Its late success still re-applies the debit once.
        let redebit = db.settle_rolled_back_stability_payment(&id, "hash", None).unwrap().unwrap();
        assert_eq!(redebit.debit_sats, 1_000);
        assert!(apply_stability_redebit(&mut sc, &redebit));
        assert_eq!(sc.backing_sats, 10_000);
    }

    #[test]
    fn a_proven_failed_claim_is_never_settled_from_the_balance() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let id = format!("{}", PaymentId([9; 32]));
        let (mut sc, info) = claimed_channel(&db, &id);
        sc.stable_receiver_btc = Bitcoin::from_sats(11_000);
        sc.native_sats = 0;
        assert!(send_claimed_stability_payment(&db, &mut sc, info, 1, || Ok(id.clone())).is_some());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        // LDK itself reports the payment failed: the outcome is proven, so only a success event
        // could ever re-apply the debit — the balance rule must not touch a later drop.
        let ldk = [details(9, PaymentStatus::Failed)];
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &ldk, now).unwrap(), 1);
        sc.stable_receiver_btc = Bitcoin::from_sats(9_500);
        assert_eq!(settle_lost_claims_from_balance(&db, &mut sc), 0);
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 11_000);
        assert_eq!(sc.backing_sats, 11_000);
    }

    #[test]
    fn pending_sent_rows_resolve_against_ldk_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::Database::open(dir.path()).unwrap();
        for id in [1u8, 2, 3] {
            db.record_payment(Some(&format!("{}", PaymentId([id; 32]))), "lightning", "sent",
                1_000_000, None, None, None, "pending", None, None).unwrap();
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let ldk = [details(1, PaymentStatus::Failed), details(2, PaymentStatus::Pending)];
        // The failure LDK reports is applied even though this process never sent the payment.
        assert_eq!(reconcile_pending_sent_lightning(&db, &ldk, now).unwrap(), 1);
        assert!(db.has_pending_channel_send().unwrap(), "in-flight and unknown rows still wait");
        // A row LDK has no record of is failed only once the grace period has passed.
        let later = now + LOST_LDK_RECORD_TIMEOUT_SECS + 1;
        assert_eq!(reconcile_pending_sent_lightning(&db, &ldk, later).unwrap(), 1);
        assert!(db.has_pending_channel_send().unwrap(), "the in-flight payment keeps blocking");
        // A fresh success waits for its event, which carries the reconciliation.
        let ldk = [details(2, PaymentStatus::Succeeded)];
        assert_eq!(reconcile_pending_sent_lightning(&db, &ldk, now).unwrap(), 0);
        assert!(db.has_pending_channel_send().unwrap());
        // A stale one lost its event long ago (older builds acked it unsaved): complete the row.
        assert_eq!(reconcile_pending_sent_lightning(&db, &ldk, later).unwrap(), 1);
        assert!(!db.has_pending_channel_send().unwrap());
        assert!(db.get_recent_payments(3).unwrap().iter().any(|p| p.status == "completed"));
    }

    #[test]
    fn a_stale_stability_claim_ldk_reports_succeeded_completes_and_keeps_its_debit() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let id = format!("{}", PaymentId([9; 32]));
        let (mut sc, info) = claimed_channel(&db, &id);
        assert!(send_claimed_stability_payment(&db, &mut sc, info, 1, || Ok(id.clone())).is_some());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let later = now + LOST_LDK_RECORD_TIMEOUT_SECS + 1;
        let ldk = [details(9, PaymentStatus::Succeeded)];

        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &ldk, now).unwrap(), 0);
        assert!(db.has_pending_channel_send().unwrap(), "a fresh success waits for its event");
        assert_eq!(reconcile_lost_stability_claims(&db, &mut sc, &ldk, later).unwrap(), 0);
        assert!(!db.has_pending_channel_send().unwrap(), "a stale one must not block spends forever");
        assert_eq!(db.get_recent_payments(1).unwrap()[0].status, "completed");
        assert_eq!(db.load_channel("7").unwrap().unwrap().backing_sats, 10_000);
        assert_eq!(sc.backing_sats, 10_000);
    }

    #[test]
    fn residual_backing_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut sc = test_sc(10.0, 50_000.0, 10_000);
        repair_overbacked_allocation(&mut sc, 100_000.0).unwrap();
        {
            let db = crate::db::Database::open(dir.path()).unwrap();
            db.save_channel("channel", "7", sc.expected_usd.0, sc.backing_sats, sc.native_sats, None).unwrap();
        }
        let db = crate::db::Database::open(dir.path()).unwrap();
        let row = db.load_channel("7").unwrap().unwrap();
        assert_eq!((row.expected_usd, row.backing_sats, row.native_sats), (0.0, 10_000, 0));
    }

    #[test]
    fn missing_future_or_old_lightning_sync_blocks_stability_send() {
        let max_age = STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS;
        // Never synced.
        assert!(!lightning_sync_is_fresh(None, 1_000_000, max_age));
        // Timestamp in the future (clock skew).
        assert!(!lightning_sync_is_fresh(Some(1_000_001), 1_000_000, max_age));
        // One second past the freshness window.
        assert!(!lightning_sync_is_fresh(
            Some(1_000_000 - max_age - 1),
            1_000_000,
            max_age
        ));
    }

    #[test]
    fn fresh_lightning_sync_allows_stability_send() {
        let max_age = STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS;
        // Exactly max_age old is accepted.
        assert!(lightning_sync_is_fresh(
            Some(1_000_000 - max_age),
            1_000_000,
            max_age
        ));
        // Just synced.
        assert!(lightning_sync_is_fresh(Some(1_000_000), 1_000_000, max_age));
    }

    #[test]
    fn signed_stability_payload_round_trips_and_enforces_expiry() {
        let settlement_id = "11".repeat(32);
        let channel_id = "22".repeat(32);
        let payload = build_stability_payment_payload(
            &settlement_id,
            &channel_id,
            909_000,
            StabilityPaymentDirection::UserToLsp,
            10.0,
            1_000,
            1_000 + STABILITY_PAYMENT_AUTH_TTL_SECS,
        )
        .unwrap();
        let parsed = parse_stability_payment_payload(&payload).unwrap();
        assert_eq!(parsed.settlement_id, settlement_id);
        assert_eq!(parsed.channel_id, channel_id);
        assert_eq!(parsed.amount_msat, 909_000);
        assert_eq!(parsed.direction, StabilityPaymentDirection::UserToLsp);
        assert!(stability_payment_is_fresh(&parsed, 1_300));
        assert!(!stability_payment_is_fresh(
            &parsed,
            parsed.expires_at + STABILITY_PAYMENT_CLOCK_SKEW_SECS + 1
        ));

        let envelope = build_stability_signed_envelope(payload.clone(), "signature".to_owned())
            .unwrap();
        let parsed_envelope = parse_stability_signed_envelope(&envelope).unwrap();
        assert_eq!(parsed_envelope.payload, payload);
        assert_eq!(parsed_envelope.signature, "signature");
    }

    #[test]
    fn signed_stability_payload_rejects_non_sat_and_invalid_identifiers() {
        let valid_id = "aa".repeat(32);
        let valid_channel = "22".repeat(32);
        let non_sat = build_stability_payment_payload(
            &valid_id,
            &valid_channel,
            1_001,
            StabilityPaymentDirection::LspToUser,
            10.0,
            1_000,
            1_100,
        )
        .unwrap();
        assert!(parse_stability_payment_payload(&non_sat).is_none());

        let uppercase_id = build_stability_payment_payload(
            &valid_id.to_uppercase(),
            &valid_channel,
            1_000,
            StabilityPaymentDirection::LspToUser,
            10.0,
            1_000,
            1_100,
        )
        .unwrap();
        assert!(parse_stability_payment_payload(&uppercase_id).is_none());

        let excessive_lifetime = build_stability_payment_payload(
            &valid_id,
            &valid_channel,
            1_000,
            StabilityPaymentDirection::LspToUser,
            10.0,
            1_000,
            1_001 + STABILITY_PAYMENT_AUTH_TTL_SECS,
        )
        .unwrap();
        assert!(parse_stability_payment_payload(&excessive_lifetime).is_none());
    }

    #[test]
    fn user_to_lsp_settlement_is_amount_bound_and_keeps_pr231_floor() {
        assert_eq!(
            backing_after_user_to_lsp_stability(10_000, 10.0, 110_000.0, 1, 49_999),
            Some(9_999)
        );
        assert_eq!(
            backing_after_user_to_lsp_stability(10_000, 10.0, 110_000.0, 909, 49_091),
            Some(9_091)
        );
        // An oversized marker/payment cannot push backing below the LSP's local equilibrium.
        assert_eq!(
            backing_after_user_to_lsp_stability(10_000, 10.0, 110_000.0, 5_000, 45_000),
            Some(9_090)
        );
        // If already below the local equilibrium, receiving money does not erase that drift.
        assert_eq!(
            backing_after_user_to_lsp_stability(9_000, 10.0, 110_000.0, 909, 49_091),
            Some(9_000)
        );
    }

    #[test]
    fn lsp_to_user_settlement_uses_local_equilibrium_and_keeps_excess_native() {
        assert_eq!(
            backing_after_lsp_to_user_stability(9_000, 10.0, 100_000.0, 1_000, 50_000),
            Some(10_000),
        );
        assert_eq!(
            backing_after_lsp_to_user_stability(9_000, 10.0, 100_000.0, 5_000, 50_000),
            Some(10_000),
            "an overpayment cannot create backing above the local target",
        );
        assert_eq!(
            backing_after_lsp_to_user_stability(10_000, 10.0, 110_000.0, 1_000, 50_000),
            Some(10_000),
            "an incoming payment cannot reduce an existing above-par allocation",
        );
        assert_eq!(
            backing_after_lsp_to_user_stability(10_001, 10.0, 100_000.0, 1_000, 10_000),
            None,
            "a stale over-capacity allocation must be reconciled before settlement",
        );
    }

    #[test]
    fn only_pending_outbound_lightning_blocks_capacity_repair() {
        use ldk_node::payment::{PaymentDirection, PaymentStatus};

        assert!(is_pending_outbound_lightning(
            PaymentDirection::Outbound,
            PaymentStatus::Pending,
            false,
        ));
        assert!(!is_pending_outbound_lightning(
            PaymentDirection::Outbound,
            PaymentStatus::Succeeded,
            false,
        ));
        assert!(!is_pending_outbound_lightning(
            PaymentDirection::Inbound,
            PaymentStatus::Pending,
            false,
        ));
        assert!(!is_pending_outbound_lightning(
            PaymentDirection::Outbound,
            PaymentStatus::Pending,
            true,
        ));
    }

    #[test]
    fn test_get_current_price_returns_non_negative() {
        let agent = Agent::new();
        let price = get_current_price(&agent);
        assert!(price >= 0.0);
    }

    #[test]
    fn effective_backing_recomputes_when_unset() {
        // Unset backing (0) with a live peg + price → derived from the peg, not left at 0
        // (so the stable value can't balloon to the full channel balance and fire a bad PAY).
        assert_eq!(
            effective_backing_sats(0, 100.0, 50_000.0, 300_000),
            200_000
        );
        // Already-set backing is returned unchanged.
        assert_eq!(
            effective_backing_sats(123_456, 100.0, 50_000.0, 300_000),
            123_456
        );
        // A legacy peg cannot repair to more backing than the wallet actually owns.
        assert_eq!(
            effective_backing_sats(0, 100.0, 100_000.0, 95_000),
            95_000
        );
        // No price / no peg → nothing to derive from; left as-is.
        assert_eq!(effective_backing_sats(0, 100.0, 0.0, 300_000), 0);
        assert_eq!(effective_backing_sats(0, 0.0, 50_000.0, 300_000), 0);
    }

    #[test]
    fn test_usd_from_bitcoin_conversion() {
        let btc = Bitcoin::from_sats(100_000_000); // 1 BTC
        let price = 50_000.0;
        let usd = USD::from_bitcoin(btc, price);
        assert_eq!(usd.0, 50_000.0);
    }

    #[test]
    fn test_usd_to_msats_conversion() {
        let usd = USD::from_f64(50.0);
        let price = 50_000.0;
        // $50 at $50k/BTC = 0.001 BTC = 100,000 sats = 100,000,000 msats
        let msats = USD::to_msats(usd, price);
        assert_eq!(msats, 100_000_000);
    }

    #[test]
    fn test_percent_from_par_calculation() {
        let target_usd: f64 = 100.0;
        let current_stable_usd: f64 = 99.0;
        let dollars_from_par = current_stable_usd - target_usd;
        let percent_from_par = ((dollars_from_par / target_usd) * 100.0).abs();
        assert_eq!(percent_from_par, 1.0);
    }

    #[test]
    fn test_stability_action_determination() {
        // Test that small deviations result in STABLE action
        let percent_from_par = 0.05; // 0.05% deviation
        let action = if percent_from_par < STABILITY_THRESHOLD_PERCENT {
            "STABLE"
        } else {
            "CHECK"
        };
        assert_eq!(action, "STABLE");
    }

    #[test]
    fn test_stability_action_above_threshold() {
        // Test that large deviations don't result in STABLE action
        let percent_from_par = 0.5; // 0.5% deviation
        let action = if percent_from_par < STABILITY_THRESHOLD_PERCENT {
            "STABLE"
        } else {
            "CHECK"
        };
        assert_eq!(action, "CHECK");
    }

    // ================================================================
    // Helper: build a StableChannel for unit tests (no node needed)
    // ================================================================
    fn test_sc(expected_usd: f64, price: f64, receiver_sats: u64) -> StableChannel {
        let backing = if price > 0.0 {
            (expected_usd / price * 100_000_000.0) as u64
        } else {
            0
        };
        let native = receiver_sats.saturating_sub(backing);
        StableChannel {
            expected_usd: USD::from_f64(expected_usd),
            backing_sats: backing,
            native_sats: native,
            latest_price: price,
            stable_receiver_btc: Bitcoin::from_sats(receiver_sats),
            is_stable_receiver: true,
            ..StableChannel::default()
        }
    }

    // ================================================================
    // reconcile_outgoing
    // ================================================================

    #[test]
    fn outgoing_no_stable_position() {
        // No stable position → nothing to reconcile
        let mut sc = test_sc(0.0, 100_000.0, 500_000);
        assert!(reconcile_outgoing(&mut sc, 100_000.0).is_none());
    }

    #[test]
    fn outgoing_payment_covered_by_native() {
        // $500 stable out of 1M sats ($1000) → 500k backing, 500k native
        // Spend 200k sats → remaining 800k > backing 500k → native absorbed it
        let mut sc = test_sc(500.0, 100_000.0, 800_000);
        assert!(reconcile_outgoing(&mut sc, 100_000.0).is_none());
        assert_eq!(sc.expected_usd.0, 500.0); // unchanged
    }

    #[test]
    fn outgoing_payment_eats_into_stable() {
        // $1000 stable at $100k → backing = 1M sats, all stable, no native
        // Spend 100k sats → receiver now has 900k < backing 1M
        let mut sc = test_sc(1000.0, 100_000.0, 900_000);
        let deducted = reconcile_outgoing(&mut sc, 100_000.0);
        assert!(deducted.is_some());
        let d = deducted.unwrap();
        assert!(
            (d - 100.0).abs() < 0.01,
            "should deduct ~$100, got ${:.2}",
            d
        );
        assert!((sc.expected_usd.0 - 900.0).abs() < 0.01);
        // All remaining sats stay in the stable allocation exactly.
        assert_eq!(sc.backing_sats, 900_000);
    }

    #[test]
    fn outgoing_partial_stable_deduction() {
        // $500 stable out of 1M sats → backing 500k, native 500k
        // Spend 700k → remaining 300k < backing 500k → overflow 200k
        let mut sc = test_sc(500.0, 100_000.0, 300_000);
        let deducted = reconcile_outgoing(&mut sc, 100_000.0).unwrap();
        assert!((deducted - 200.0).abs() < 0.01, "overflow 200k sats = $200");
        assert!((sc.expected_usd.0 - 300.0).abs() < 0.01);
    }

    #[test]
    fn outgoing_spends_entire_stable() {
        // $500 stable, backing 500k, receiver has 0 sats left
        let mut sc = test_sc(500.0, 100_000.0, 0);
        let deducted = reconcile_outgoing(&mut sc, 100_000.0).unwrap();
        assert!((deducted - 500.0).abs() < 0.01);
        assert!(sc.expected_usd.0 < 0.01); // clamped to 0
        assert_eq!(sc.backing_sats, 0);
    }

    #[test]
    fn outgoing_zero_price_returns_none() {
        let mut sc = test_sc(500.0, 100_000.0, 300_000);
        assert!(reconcile_outgoing(&mut sc, 0.0).is_none());
        assert_eq!(sc.expected_usd.0, 500.0); // unchanged
    }

    #[test]
    fn outgoing_zero_backing_returns_none() {
        let mut sc = test_sc(500.0, 100_000.0, 300_000);
        sc.backing_sats = 0;
        assert!(reconcile_outgoing(&mut sc, 100_000.0).is_none());
    }

    #[test]
    fn overbacked_allocation_is_repaired_from_live_balance() {
        let mut sc = test_sc(100.0, 100_000.0, 80_000);
        sc.backing_sats = 100_000;
        sc.native_sats = 0;

        let repair = repair_overbacked_allocation(&mut sc, 100_000.0).unwrap();

        assert_eq!(repair.backing_sats_before, 100_000);
        assert_eq!(repair.backing_sats_after, 80_000);
        assert!((repair.usd_deducted - 20.0).abs() < 1e-6);
        assert!((sc.expected_usd.0 - 80.0).abs() < 1e-6);
        assert_eq!(sc.backing_sats, 80_000);
        assert_eq!(sc.native_sats, 0);
    }

    #[test]
    fn overbacked_repair_waits_for_a_valid_price() {
        let mut sc = test_sc(100.0, 100_000.0, 80_000);
        sc.backing_sats = 100_000;

        assert!(repair_overbacked_allocation(&mut sc, 0.0).is_none());
        assert_eq!(sc.expected_usd.0, 100.0);
        assert_eq!(sc.backing_sats, 100_000);
    }

    #[test]
    fn outgoing_at_different_prices() {
        // Same sats overflow at higher price → larger USD deduction
        // $500 stable at $100k → backing 500k. Receiver has 400k. Overflow 100k.
        let mut sc1 = test_sc(500.0, 100_000.0, 400_000);
        let d1 = reconcile_outgoing(&mut sc1, 100_000.0).unwrap();

        // Same scenario but reconcile at $200k price
        let mut sc2 = test_sc(500.0, 100_000.0, 400_000);
        let d2 = reconcile_outgoing(&mut sc2, 200_000.0).unwrap();

        // 100k sats at $100k = $100, at $200k = $200
        assert!((d1 - 100.0).abs() < 0.01);
        assert!((d2 - 200.0).abs() < 0.01);
    }

    #[test]
    fn outgoing_after_price_move_preserves_remaining_stable_sats() {
        // Allocated at $100k: $100 is 100k stable sats. Price falls to $80k before a 10k-sat
        // stable overflow is reconciled. The quote changes the USD deduction, not the allocation.
        let mut sc = test_sc(100.0, 100_000.0, 90_000);
        let deducted = reconcile_outgoing(&mut sc, 80_000.0).unwrap();

        assert!((deducted - 8.0).abs() < 1e-9);
        assert!((sc.expected_usd.0 - 92.0).abs() < 1e-9);
        assert_eq!(sc.backing_sats, 90_000);
        assert_eq!(sc.native_sats, 0);
    }

    #[test]
    fn splice_out_snapshot_is_stable_after_live_balance_advances() {
        // Production regression: ChannelReady exposed the post-splice 1,742 sats before the
        // funding-output lookup returned. The deduction must still use the pre-splice allocation.
        let price = 65_872.5;
        let mut sc = test_sc(31.4424, price, 1_742);
        sc.backing_sats = 47_615;
        sc.native_sats = 0;

        let deducted =
            deduct_outgoing_from_snapshot(&mut sc, 92_022, 47_615, 90_280, price).unwrap();

        assert!((deducted - 30.217691925).abs() < 1e-9);
        assert!((sc.expected_usd.0 - 1.224708075).abs() < 1e-9);
        assert_eq!(sc.backing_sats, 1_742);
        assert_eq!(sc.native_sats, 0);
        assert_eq!(sc.native_channel_btc.sats, 0);
    }

    #[test]
    fn splice_out_preserves_remaining_backing_when_expected_usd_reaches_zero() {
        let mut sc = test_sc(0.005, 65_000.0, 250);
        sc.backing_sats = 500;

        let deducted =
            deduct_outgoing_from_snapshot(&mut sc, 500, 500, 250, 65_000.0).unwrap();

        assert!((deducted - 0.005).abs() < f64::EPSILON);
        assert_eq!(sc.expected_usd.0, 0.0);
        assert_eq!(sc.backing_sats, 250);
        assert_eq!(sc.native_sats, 0);
    }

    // ================================================================
    // reconcile_forwarded (LSP side)
    // ================================================================

    #[test]
    fn forwarded_covered_by_native() {
        // User has 1M sats, backing 500k → native 500k
        // Forwarded 200k sats → all covered by native
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        sc.is_stable_receiver = false; // LSP perspective
        let result = reconcile_forwarded(&mut sc, 1_000_000, 200_000, 100_000.0);
        assert!(result.is_none());
        assert_eq!(sc.expected_usd.0, 500.0);
    }

    #[test]
    fn forwarded_eats_into_stable() {
        // User has 1M sats, backing 500k → native 500k
        // Forwarded 700k → 500k native + 200k from stable
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        let deducted = reconcile_forwarded(&mut sc, 1_000_000, 700_000, 100_000.0).unwrap();
        assert!((deducted - 200.0).abs() < 0.01);
        assert!((sc.expected_usd.0 - 300.0).abs() < 0.01);
    }

    #[test]
    fn forwarded_all_stable_no_native() {
        // User has 500k sats, backing 500k → 0 native
        // Forwarded 100k → all from stable
        let mut sc = test_sc(500.0, 100_000.0, 500_000);
        let deducted = reconcile_forwarded(&mut sc, 500_000, 100_000, 100_000.0).unwrap();
        assert!((deducted - 100.0).abs() < 0.01);
        assert!((sc.expected_usd.0 - 400.0).abs() < 0.01);
    }

    #[test]
    fn forwarded_after_price_move_preserves_remaining_stable_sats() {
        // 100k stable + 50k native were allocated at $100k. At an $80k execution quote, a 60k
        // payment consumes all native and 10k stable, leaving exactly 90k stable sats.
        let mut sc = test_sc(100.0, 100_000.0, 150_000);
        let deducted = reconcile_forwarded(&mut sc, 150_000, 60_000, 80_000.0).unwrap();

        assert!((deducted - 8.0).abs() < 1e-9);
        assert!((sc.expected_usd.0 - 92.0).abs() < 1e-9);
        assert_eq!(sc.backing_sats, 90_000);
        assert_eq!(sc.native_sats, 0);
    }

    #[test]
    fn forwarded_zero_expected_usd() {
        let mut sc = test_sc(0.0, 100_000.0, 500_000);
        assert!(reconcile_forwarded(&mut sc, 500_000, 100_000, 100_000.0).is_none());
    }

    #[test]
    fn forwarded_zero_price() {
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        assert!(reconcile_forwarded(&mut sc, 1_000_000, 700_000, 0.0).is_none());
    }

    // ================================================================
    // reconcile_incoming
    // ================================================================

    #[test]
    fn incoming_does_not_reset_backing() {
        // backing_sats stays as-is on incoming — only the sender resets it
        let mut sc = test_sc(500.0, 100_000.0, 1_200_000);
        sc.backing_sats = 600_000; // drifted
        reconcile_incoming(&mut sc);
        assert_eq!(sc.backing_sats, 600_000); // unchanged
    }

    #[test]
    fn incoming_no_change_when_already_at_equilibrium() {
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        let backing_before = sc.backing_sats;
        reconcile_incoming(&mut sc);
        assert_eq!(sc.backing_sats, backing_before);
    }

    #[test]
    fn incoming_skips_when_no_stable_position() {
        let mut sc = test_sc(0.0, 100_000.0, 500_000);
        sc.backing_sats = 12345;
        reconcile_incoming(&mut sc);
        assert_eq!(sc.backing_sats, 12345); // unchanged
    }

    #[test]
    fn incoming_derives_from_balance_not_price() {
        // With native_sats model, reconcile_incoming derives backing from balance,
        // not from price. Even with zero price, it should work correctly.
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        sc.latest_price = 0.0; // price unavailable
        let backing_before = sc.backing_sats;
        reconcile_incoming(&mut sc);
        // backing_sats = receiver_sats - native_sats = 1M - 500k = 500k
        assert_eq!(sc.backing_sats, backing_before);
    }

    #[test]
    fn incoming_preserves_expected_usd() {
        let mut sc = test_sc(500.0, 100_000.0, 1_500_000);
        reconcile_incoming(&mut sc);
        assert_eq!(sc.expected_usd.0, 500.0); // never changes
    }

    // ================================================================
    // apply_trade
    // ================================================================

    #[test]
    fn trade_buy_reduces_stable() {
        // Buy $200 BTC: expected_usd $500 → $300
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        assert!(apply_trade(&mut sc, 300.0, 100_000.0));
        assert_eq!(sc.expected_usd.0, 300.0);
        let expected_backing = (300.0 / 100_000.0 * 100_000_000.0) as u64;
        assert_eq!(sc.backing_sats, expected_backing);
    }

    #[test]
    fn trade_sell_increases_stable() {
        // Sell $200 BTC: expected_usd $500 → $700
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        assert!(apply_trade(&mut sc, 700.0, 100_000.0));
        assert_eq!(sc.expected_usd.0, 700.0);
        let expected_backing = (700.0 / 100_000.0 * 100_000_000.0) as u64;
        assert_eq!(sc.backing_sats, expected_backing);
    }

    #[test]
    fn trade_to_zero() {
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        assert!(apply_trade(&mut sc, 0.0, 100_000.0));
        assert_eq!(sc.expected_usd.0, 0.0);
        assert_eq!(sc.backing_sats, 0);
    }

    #[test]
    fn trade_zero_price_skips_backing_update() {
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        let backing_before = sc.backing_sats;
        assert!(!apply_trade(&mut sc, 700.0, 0.0));
        assert_eq!(sc.expected_usd.0, 500.0);
        assert_eq!(sc.backing_sats, backing_before);
    }

    #[test]
    fn no_op_trades_preserve_above_and_below_par_drift() {
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        assert!(apply_trade(&mut sc, 500.0, 200_000.0));
        assert_eq!(sc.backing_sats, 500_000);

        assert!(apply_trade(&mut sc, 500.0, 80_000.0));
        assert_eq!(sc.backing_sats, 500_000);
    }

    #[test]
    fn trade_full_balance_to_stable() {
        // Convert all $1000 to stable
        let mut sc = test_sc(0.0, 100_000.0, 1_000_000);
        assert!(apply_trade(&mut sc, 1000.0, 100_000.0));
        assert_eq!(sc.expected_usd.0, 1000.0);
        assert_eq!(sc.backing_sats, 1_000_000);
    }

    #[test]
    fn trade_full_balance_absorbs_sub_cent_native_dust() {
        let mut sc = test_sc(0.0, 66_250.21, 57_444);
        assert!(apply_trade(&mut sc, 38.055025828575, 66_250.21));

        assert_eq!(sc.backing_sats, 57_444);
        assert_eq!(sc.native_sats, 0);
        assert_eq!(sc.native_channel_btc.sats, 0);
    }

    #[test]
    fn trade_keeps_meaningful_native_allocation() {
        let mut sc = test_sc(0.0, 100_000.0, 100_000);
        assert!(apply_trade(&mut sc, 99.0, 100_000.0));

        assert_eq!(sc.backing_sats, 99_000);
        assert_eq!(sc.native_sats, 1_000);
        assert_eq!(sc.native_channel_btc.sats, 1_000);
    }

    #[test]
    fn trade_over_live_balance_is_not_partially_applied() {
        let mut sc = test_sc(0.0, 100_000.0, 95_000);
        assert!(!apply_trade(&mut sc, 100.0, 100_000.0));

        assert_eq!(sc.expected_usd.0, 0.0);
        assert_eq!(sc.backing_sats, 0);
        assert_eq!(sc.native_sats, 95_000);
    }

    #[test]
    fn normal_trades_apply_only_the_locally_priced_delta() {
        let mut increase = test_sc(100.0, 100_000.0, 200_000);
        assert!(apply_trade(&mut increase, 110.0, 110_000.0));
        assert_eq!(increase.backing_sats, 109_091);

        let mut decrease = test_sc(100.0, 100_000.0, 200_000);
        assert!(apply_trade(&mut decrease, 90.0, 110_000.0));
        assert_eq!(decrease.backing_sats, 90_909);
    }

    #[test]
    fn tiny_trade_preserves_preexisting_stability_drift() {
        let mut sc = test_sc(100.0, 100_000.0, 200_000);

        assert!(apply_trade(&mut sc, 100.01, 110_000.0));

        assert_eq!(sc.backing_sats, 100_009);
        let value = sc.backing_sats as f64 / SATS_IN_BTC as f64 * 110_000.0;
        assert!(value - sc.expected_usd.0 > 9.99);
    }

    #[test]
    fn repeated_sub_sat_target_changes_apply_the_cumulative_backing() {
        let price = 63_734.35;
        let mut current_expected = 0.0;
        let mut backing = 0;

        for step in 1..=10_000 {
            let new_expected =
                normalize_trade_expected_usd(step as f64 / 10_000.0);
            backing = trade_backing_after_delta(
                1_000_000,
                backing,
                current_expected,
                new_expected,
                price,
            )
            .unwrap();
            current_expected = new_expected;
        }

        assert_eq!(
            backing,
            (current_expected / price * SATS_IN_BTC as f64).floor() as u64,
        );
    }

    #[test]
    fn arithmetic_underflow_leaves_trade_state_unchanged() {
        let mut sc = test_sc(100.0, 100_000.0, 200_000);
        sc.backing_sats = 10;
        sc.native_sats = 199_990;

        assert!(!apply_trade(&mut sc, 99.0, 100_000.0));
        assert_eq!(sc.expected_usd.0, 100.0);
        assert_eq!(sc.backing_sats, 10);
        assert_eq!(sc.native_sats, 199_990);
    }

    #[test]
    fn nonzero_trade_target_cannot_use_the_uninitialized_zero_backing_sentinel() {
        // At the local price this reduction consumes the final 500 backing sats while leaving a
        // live $0.50 target. Accepting it would let check_stability rebuild 500 sats and erase the
        // existing below-par drift.
        assert_eq!(
            trade_backing_after_delta(10_000, 500, 1.0, 0.5, 100_000.0),
            None,
        );

        let mut sc = test_sc(1.0, 100_000.0, 10_000);
        sc.backing_sats = 500;
        sc.native_sats = 9_500;
        assert!(!apply_trade(&mut sc, 0.5, 100_000.0));
        assert_eq!(sc.expected_usd.0, 1.0);
        assert_eq!(sc.backing_sats, 500);
    }

    #[test]
    fn full_exit_waits_for_actionable_drift_to_settle() {
        for price in [90_000.0, 110_000.0] {
            let mut sc = test_sc(100.0, 100_000.0, 200_000);
            assert!(!apply_trade(&mut sc, 0.0, price));
            assert_eq!(sc.expected_usd.0, 100.0);
            assert_eq!(sc.backing_sats, 100_000);
        }
    }

    #[test]
    fn full_exit_allows_non_actionable_drift() {
        let mut sc = test_sc(100.0, 100_000.0, 200_000);
        assert!(apply_trade(&mut sc, 0.0, 100_001.0));
        assert_eq!(sc.expected_usd.0, 0.0);
        assert_eq!(sc.backing_sats, 0);
    }

    #[test]
    fn sub_cent_target_is_normalized_through_the_full_exit_guard() {
        let mut safe = test_sc(100.0, 100_000.0, 200_000);
        assert!(apply_trade(&mut safe, 0.009, 100_001.0));
        assert_eq!(safe.expected_usd.0, 0.0);
        assert_eq!(safe.backing_sats, 0);

        let mut actionable = test_sc(100.0, 100_000.0, 200_000);
        assert!(!apply_trade(&mut actionable, 0.009, 90_000.0));
        assert_eq!(actionable.expected_usd.0, 100.0);
        assert_eq!(actionable.backing_sats, 100_000);
    }

    #[test]
    fn full_exit_does_not_hide_large_absolute_drift_on_sub_cent_target() {
        let mut sc = test_sc(0.005, 100_000.0, 200_000);
        sc.backing_sats = 100_000;
        sc.native_sats = 100_000;

        assert!(!apply_trade(&mut sc, 0.0, 100_000.0));
        assert_eq!(sc.expected_usd.0, 0.005);
        assert_eq!(sc.backing_sats, 100_000);
    }

    #[test]
    fn capacity_overflow_is_rejected_instead_of_clamped() {
        assert_eq!(
            trade_backing_after_delta(100_000, 0, 0.0, 100.001, 100_000.0),
            None,
        );
        assert_eq!(
            trade_backing_after_delta(100_000, 0, 0.0, 100.10, 100_000.0),
            None,
        );

        // Existing backing is $0.30 above a $10 target. Clamping this increase would silently
        // erase that actionable pre-trade obligation.
        assert_eq!(
            trade_backing_after_delta(20_000, 10_300, 10.0, 20.001, 100_000.0),
            None,
        );
    }

    #[test]
    fn signed_trade_allocation_is_not_repriced_by_the_applying_peer() {
        let receiver_sats = 50_000;
        let quote_price = 100_000.0;
        let backing_sats = trade_backing_sats(receiver_sats, 49.95, quote_price);
        assert_eq!(backing_sats, 49_950);

        let mut sc = test_sc(0.0, 100_500.0, receiver_sats);
        apply_trade_allocation(&mut sc, 49.95, backing_sats);

        assert_eq!(sc.expected_usd.0, 49.95);
        assert_eq!(sc.backing_sats, 49_950);
        assert_eq!(sc.native_sats, 50);
    }

    #[test]
    fn signed_full_peg_allocation_absorbs_sub_cent_residue() {
        let backing_sats = trade_backing_sats(57_444, 38.055025828575, 66_250.21);
        assert_eq!(backing_sats, 57_444);

        let mut sc = test_sc(0.0, 66_400.0, 57_444);
        apply_trade_allocation(&mut sc, 38.055025828575, backing_sats);
        assert_eq!(sc.backing_sats, 57_444);
        assert_eq!(sc.native_sats, 0);
    }

    // ================================================================
    // recompute_native
    // ================================================================

    #[test]
    fn native_half_stable_half_native() {
        // $500 stable out of 1M sats ($1000) → backing 500k, native 500k
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        recompute_native(&mut sc);
        assert_eq!(sc.native_channel_btc.sats, 500_000);
    }

    #[test]
    fn native_fully_stabilized() {
        // $1000 stable out of 1M sats → backing 1M, native 0
        let mut sc = test_sc(1000.0, 100_000.0, 1_000_000);
        recompute_native(&mut sc);
        assert_eq!(sc.native_channel_btc.sats, 0);
    }

    #[test]
    fn native_backing_exceeds_receiver_saturates() {
        // Edge case: backing > receiver (stale backing) → native saturates to 0
        let mut sc = test_sc(1000.0, 100_000.0, 800_000);
        recompute_native(&mut sc);
        assert_eq!(sc.native_channel_btc.sats, 0);
    }

    #[test]
    fn native_updated_after_reconcile_incoming() {
        // Simulate stability payment: receiver gained sats
        // backing stays at drifted value, native = receiver - backing
        let mut sc = test_sc(500.0, 100_000.0, 1_200_000);
        sc.backing_sats = 600_000; // drifted
        reconcile_incoming(&mut sc);
        // native = 1.2M - 600k (drifted backing) = 600k
        assert_eq!(sc.native_channel_btc.sats, 1_200_000 - 600_000);
    }

    #[test]
    fn native_updated_after_apply_trade() {
        // Sell BTC: increase stable from $500 to $800
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        assert!(apply_trade(&mut sc, 800.0, 100_000.0));
        let expected_backing = (800.0 / 100_000.0 * 100_000_000.0) as u64;
        assert_eq!(sc.native_channel_btc.sats, 1_000_000 - expected_backing);
    }

    #[test]
    fn native_updated_after_reconcile_outgoing() {
        // $1000 stable, backing 1M, user spent 100k → receiver now 900k
        let mut sc = test_sc(1000.0, 100_000.0, 900_000);
        reconcile_outgoing(&mut sc, 100_000.0);
        // expected_usd reduced to ~$900, backing ~900k, native ≈ 0 (±1 sat from f64 truncation)
        assert!(
            sc.native_channel_btc.sats <= 1,
            "native should be ~0, got {}",
            sc.native_channel_btc.sats
        );
    }

    #[test]
    fn cooldown_field_default() {
        let sc = StableChannel::default();
        assert_eq!(sc.last_stability_payment, 0);
    }
}
