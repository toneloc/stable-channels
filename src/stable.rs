use crate::audit::audit_event;
use crate::constants::{
    MAX_RISK_LEVEL, SATS_IN_BTC, STABILITY_PAYMENT_COOLDOWN_SECS, STABILITY_THRESHOLD_PERCENT,
    STABILITY_THRESHOLD_USD,
};
use crate::price_feeds::{get_cached_price, get_cached_price_no_fetch};
use crate::types::{Bitcoin, StableChannel, USD};
use ldk_node::Node;
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
    if sc.expected_usd.0 <= 0.01 || sc.backing_sats == 0 || price <= 0.0 {
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
    if sc.expected_usd.0 <= 0.0 || price <= 0.0 {
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
    if new_expected == 0.0 {
        sc.backing_sats = 0;
    }
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

/// Derive the stable backing allocation for a trade at the signed quote price.
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

/// Apply a previously agreed trade allocation without repricing it.
///
/// `backing_sats` is part of the signed trade intent. A peer may validate that intent against its
/// own price and live LDK balance first, but once accepted it must not derive a different allocation
/// from a later price snapshot.
pub fn apply_trade_allocation(sc: &mut StableChannel, new_expected_usd: f64, backing_sats: u64) {
    sc.expected_usd = USD::from_f64(new_expected_usd);
    sc.backing_sats = backing_sats;
    sc.native_sats = sc.stable_receiver_btc.sats.saturating_sub(sc.backing_sats);
    recompute_native(sc);
}

/// Apply a trade — set new expected_usd and recalculate backing_sats + native_sats.
///
/// Used after buy/sell trades and when the LSP processes a trade message.
/// Sets `expected_usd` to the new value and recalculates `backing_sats`
/// at the current price. Updates `native_sats` (the invariant that stays
/// fixed between stability payments).
pub fn apply_trade(sc: &mut StableChannel, new_expected_usd: f64, price: f64) {
    sc.expected_usd = USD::from_f64(new_expected_usd);
    if price > 0.0 {
        let receiver_sats = sc.stable_receiver_btc.sats;
        sc.backing_sats = if receiver_sats > 0 {
            trade_backing_sats(receiver_sats, new_expected_usd, price)
        } else {
            (new_expected_usd / price * SATS_IN_BTC as f64) as u64
        };
    }
    // native_sats is everything NOT backing the stable position
    sc.native_sats = sc.stable_receiver_btc.sats.saturating_sub(sc.backing_sats);
    recompute_native(sc);
}

/// Get the current BTC/USD price, preferring cached value when available
pub fn get_current_price(agent: &Agent) -> f64 {
    // First try the cached price
    let cached_price = get_cached_price();

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
    let cached = get_cached_price_no_fetch();
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
    pub payment_id: String,
    pub amount_msat: u64,
    pub counterparty: String,
    pub btc_price: f64,
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

    // Skip if expected_usd is zero or very small (nothing to stabilize)
    if sc.expected_usd.0 < 0.01 {
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
    if sc.backing_sats > sc.stable_receiver_btc.sats {
        audit_event(
            "STABILITY_SKIP",
            json!({
                "user_channel_id": format!("{}", sc.user_channel_id),
                "reason": "backing exceeds live balance; awaiting spend reconciliation",
                "backing_sats": sc.backing_sats,
                "live_receiver_sats": sc.stable_receiver_btc.sats,
            }),
        );
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
    let percent_from_par = if target_usd > 0.0 {
        ((dollars_from_par.0 / target_usd) * 100.0).abs()
    } else {
        0.0
    };
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

    let amt = USD::to_msats(dollars_from_par, sc.latest_price);
    let marker = ldk_node::CustomTlvRecord {
        type_num: crate::constants::STABLE_CHANNEL_TLV_TYPE,
        value: vec![1u8],
    };
    match node.spontaneous_payment().send_with_custom_tlvs(amt, sc.counterparty, None, vec![marker]) {
        Ok(payment_id) => {
            sc.payment_made = true;
            sc.last_stability_payment = now;

            // Reset backing_sats to equilibrium at current price.
            // This accounts the payment against the stable pool, not native BTC.
            // Don't recompute native_sats here — receiver balance hasn't updated yet
            // (HTLC still in flight). Native will be recomputed on next balance refresh.
            let new_backing = (target_usd / sc.latest_price * 100_000_000.0) as u64;
            sc.backing_sats = new_backing;

            let payment_id_str = payment_id.to_string();
            let counterparty_str = sc.counterparty.to_string();
            audit_event(
                "STABILITY_PAYMENT_SENT",
                json!({
                    "amount_msats": amt,
                    "payment_id": payment_id_str,
                    "counterparty": counterparty_str,
                    "expected_usd": target_usd,
                    "new_backing_sats": new_backing
                }),
            );
            Some(StabilityPaymentInfo {
                payment_id: payment_id_str,
                amount_msat: amt,
                counterparty: counterparty_str,
                btc_price: sc.latest_price,
            })
        }
        Err(e) => {
            audit_event(
                "STABILITY_PAYMENT_FAILED",
                json!({
                    "amount_msats": amt,
                    "error": format!("{e}"),
                    "counterparty": sc.counterparty.to_string()
                }),
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn splice_out_releases_remaining_backing_when_expected_usd_reaches_zero() {
        let mut sc = test_sc(0.005, 65_000.0, 250);
        sc.backing_sats = 500;

        let deducted =
            deduct_outgoing_from_snapshot(&mut sc, 500, 500, 250, 65_000.0).unwrap();

        assert!((deducted - 0.005).abs() < f64::EPSILON);
        assert_eq!(sc.expected_usd.0, 0.0);
        assert_eq!(sc.backing_sats, 0);
        assert_eq!(sc.native_sats, 250);
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
        apply_trade(&mut sc, 300.0, 100_000.0);
        assert_eq!(sc.expected_usd.0, 300.0);
        let expected_backing = (300.0 / 100_000.0 * 100_000_000.0) as u64;
        assert_eq!(sc.backing_sats, expected_backing);
    }

    #[test]
    fn trade_sell_increases_stable() {
        // Sell $200 BTC: expected_usd $500 → $700
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        apply_trade(&mut sc, 700.0, 100_000.0);
        assert_eq!(sc.expected_usd.0, 700.0);
        let expected_backing = (700.0 / 100_000.0 * 100_000_000.0) as u64;
        assert_eq!(sc.backing_sats, expected_backing);
    }

    #[test]
    fn trade_to_zero() {
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        apply_trade(&mut sc, 0.0, 100_000.0);
        assert_eq!(sc.expected_usd.0, 0.0);
        assert_eq!(sc.backing_sats, 0);
    }

    #[test]
    fn trade_zero_price_skips_backing_update() {
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        let backing_before = sc.backing_sats;
        apply_trade(&mut sc, 700.0, 0.0);
        assert_eq!(sc.expected_usd.0, 700.0); // usd updated
        assert_eq!(sc.backing_sats, backing_before); // backing unchanged
    }

    #[test]
    fn trade_at_different_price() {
        // Same $500 stable, but price doubled to $200k
        // backing should be half the sats
        let mut sc = test_sc(500.0, 100_000.0, 1_000_000);
        apply_trade(&mut sc, 500.0, 200_000.0);
        let expected_backing = (500.0 / 200_000.0 * 100_000_000.0) as u64; // 250k
        assert_eq!(sc.backing_sats, expected_backing);
        assert_eq!(expected_backing, 250_000);
    }

    #[test]
    fn trade_full_balance_to_stable() {
        // Convert all $1000 to stable
        let mut sc = test_sc(0.0, 100_000.0, 1_000_000);
        apply_trade(&mut sc, 1000.0, 100_000.0);
        assert_eq!(sc.expected_usd.0, 1000.0);
        assert_eq!(sc.backing_sats, 1_000_000);
    }

    #[test]
    fn trade_full_balance_absorbs_sub_cent_native_dust() {
        let mut sc = test_sc(0.0, 66_250.21, 57_444);
        apply_trade(&mut sc, 38.055025828575, 66_250.21);

        assert_eq!(sc.backing_sats, 57_444);
        assert_eq!(sc.native_sats, 0);
        assert_eq!(sc.native_channel_btc.sats, 0);
    }

    #[test]
    fn trade_keeps_meaningful_native_allocation() {
        let mut sc = test_sc(0.0, 100_000.0, 100_000);
        apply_trade(&mut sc, 99.0, 100_000.0);

        assert_eq!(sc.backing_sats, 99_000);
        assert_eq!(sc.native_sats, 1_000);
        assert_eq!(sc.native_channel_btc.sats, 1_000);
    }

    #[test]
    fn trade_backing_never_exceeds_live_balance() {
        let mut sc = test_sc(0.0, 100_000.0, 95_000);
        apply_trade(&mut sc, 100.0, 100_000.0);

        assert_eq!(sc.backing_sats, 95_000);
        assert_eq!(sc.native_sats, 0);
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
        apply_trade(&mut sc, 800.0, 100_000.0);
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
