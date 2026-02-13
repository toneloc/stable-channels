use crate::types::{Bitcoin, StableChannel, USD};
use ldk_node::{
    lightning::ln::types::ChannelId, Node,
};
use ureq::Agent;
use crate::price_feeds::get_cached_price;
use crate::audit::audit_event;
use crate::constants::{STABILITY_THRESHOLD_PERCENT, MAX_RISK_LEVEL, SATS_IN_BTC};
use serde_json::json;

// ================================================================
// Reconciliation functions
// ================================================================

/// Reconcile an outgoing payment against the stable position.
///
/// When the user sends a payment, their channel balance decreases.
/// If `backing_sats > actual_receiver_sats`, the payment ate into
/// the stable portion. This function deducts the overflow from
/// `expected_usd` and recalculates `backing_sats`.
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
    let btc_amount = new_expected / price;
    sc.backing_sats = (btc_amount * 100_000_000.0) as u64;

    Some(usd_to_deduct)
}

/// Reconcile an outgoing forwarded payment on the LSP side.
///
/// The LSP knows the total sats forwarded and the user's current balance.
/// Native BTC is spent first; any overflow eats into the stable position.
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
    if price > 0.0 {
        let btc_amount = new_expected / price;
        sc.backing_sats = (btc_amount * 100_000_000.0) as u64;
    }

    Some(usd_to_deduct)
}

/// Reconcile an incoming payment — reset backing_sats to equilibrium.
///
/// After receiving a payment, the user's balance increased but
/// `expected_usd` hasn't changed. Recalculate `backing_sats` so
/// the extra sats are treated as native BTC, not stable.
pub fn reconcile_incoming(sc: &mut StableChannel) {
    if sc.expected_usd.0 > 0.01 && sc.latest_price > 0.0 {
        let btc_amount = sc.expected_usd.0 / sc.latest_price;
        sc.backing_sats = (btc_amount * 100_000_000.0) as u64;
    }
}

/// Apply a trade — set new expected_usd and recalculate backing_sats.
///
/// Used after buy/sell trades and when the LSP processes a trade message.
/// Sets `expected_usd` to the new value and recalculates `backing_sats`
/// at the current price.
pub fn apply_trade(sc: &mut StableChannel, new_expected_usd: f64, price: f64) {
    sc.expected_usd = USD::from_f64(new_expected_usd);
    if price > 0.0 {
        let btc_amount = new_expected_usd / price;
        sc.backing_sats = (btc_amount * 100_000_000.0) as u64;
    }
}

/// Get the current BTC/USD price, preferring cached value when available
pub fn get_current_price(agent: &Agent) -> f64 {
    // First try the cached price
    let cached_price = get_cached_price();
    
    // Use the cached price if valid
    if cached_price > 0.0 {
        return cached_price;
    }
    
    match crate::price_feeds::get_latest_price(agent) {
        Ok(price) => price,
        Err(_) => 0.0 
    }
}

pub fn channel_exists(node: &Node, channel_id: &ChannelId) -> bool {
    let channels = node.list_channels();
    channels.iter().any(|c| c.channel_id == *channel_id)
}

// Can run in backgound
pub fn update_balances<'update_balance_lifetime>(
    node: &Node,
    sc: &'update_balance_lifetime mut StableChannel,
) -> (bool, &'update_balance_lifetime mut StableChannel) {
    if sc.latest_price == 0.0 {
        sc.latest_price = get_cached_price();
        
        if sc.latest_price == 0.0 {
            let agent = Agent::new();
            sc.latest_price = get_current_price(&agent);
        }
    }

    // --- Update On-chain ---
    let balances = node.list_balances();
    sc.onchain_btc = Bitcoin::from_sats(balances.total_onchain_balance_sats);
    sc.onchain_usd = USD::from_bitcoin(sc.onchain_btc, sc.latest_price);

    let channels = node.list_channels();
    let matching_channel = if sc.channel_id == ChannelId::from_bytes([0; 32]) {
        channels.first()
    } else {
        channels.iter().find(|c| c.channel_id == sc.channel_id)
    };
    
    if let Some(channel) = matching_channel {
        if sc.channel_id == ChannelId::from_bytes([0; 32]) {
            sc.channel_id = channel.channel_id;
            println!("Set active channel ID to: {}", sc.channel_id);
        }

        // Skip balance update if channel is not ready yet — during ChannelPending,
        // outbound_capacity_msat is 0, which produces a misleading near-zero balance.
        if !channel.is_channel_ready {
            return (true, sc);
        }

        let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
        let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
        let their_balance_sats = channel.channel_value_sats - our_balance_sats;
        
        if sc.is_stable_receiver {
            sc.stable_receiver_btc = Bitcoin::from_sats(our_balance_sats);
            sc.stable_provider_btc = Bitcoin::from_sats(their_balance_sats);
        } else {
            sc.stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
            sc.stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
        }
        
        sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
        sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);

        // Native BTC is the portion not stabilized (total - expected_usd)
        let total_receiver_usd = sc.stable_receiver_usd.0;
        let native_usd = USD::from_f64((total_receiver_usd - sc.expected_usd.0).max(0.0));
        sc.native_channel_btc = Bitcoin::from_usd(native_usd, sc.latest_price);

        audit_event("BALANCE_UPDATE", json!({
            "channel_id": format!("{}", sc.channel_id),
            "stable_receiver_btc": sc.stable_receiver_btc.to_string(),
            "stable_provider_btc": sc.stable_provider_btc.to_string(),
            "stable_receiver_usd": sc.stable_receiver_usd.to_string(),
            "stable_provider_usd": sc.stable_provider_usd.to_string(),
            "native_channel_btc": sc.native_channel_btc.to_string(),
            "btc_price": sc.latest_price
        }));

        return (true, sc);
    }
    
    println!("No matching channel found for ID: {}", sc.channel_id);
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
pub fn check_stability(node: &Node, sc: &mut StableChannel, price: f64) -> Option<StabilityPaymentInfo> {
    let current_price = if price > 0.0 {
        price
    } else {
        let cached_price = get_cached_price();
        if cached_price > 0.0 {
            cached_price
        } else {
            audit_event("STABILITY_SKIP", json!({
                "reason": "no valid price available"
            }));
            return None;
        }
    };

    sc.latest_price = current_price;
    let (success, _) = update_balances(node, sc);

    if !success {
        audit_event("BALANCE_UPDATE_FAILED", json!({
            "channel_id": format!("{}", sc.channel_id)
        }));
        return None;
    }

    // Skip if expected_usd is zero or very small (nothing to stabilize)
    if sc.expected_usd.0 < 0.01 {
        audit_event("STABILITY_SKIP", json!({
            "channel_id": format!("{}", sc.channel_id),
            "reason": "expected_usd is too small",
            "expected_usd": sc.expected_usd.0
        }));
        return None;
    }

    // The target is expected_usd
    let target_usd = sc.expected_usd.0;

    // Use backing_sats to calculate the current value of the stable portion only
    // This excludes the native BTC position from stability calculations
    let stable_usd_value = if sc.backing_sats > 0 {
        // backing_sats tracks the BTC backing the stable portion
        (sc.backing_sats as f64 / 100_000_000.0) * current_price
    } else {
        // Fallback for channels without backing_sats set yet - use total balance
        // but only if expected_usd is set (means user has a stable position)
        sc.stable_receiver_usd.0
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

    let action = if percent_from_par < STABILITY_THRESHOLD_PERCENT {
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

    audit_event("STABILITY_CHECK", json!({
        "expected_usd": target_usd,
        "stable_usd_value": stable_usd_value,
        "backing_sats": sc.backing_sats,
        "total_receiver_usd": sc.stable_receiver_usd.0,
        "percent_from_par": percent_from_par,
        "btc_price": sc.latest_price,
        "action": action,
        "is_stable_receiver": sc.is_stable_receiver,
        "risk_level": sc.risk_level
    }));

    if action != "PAY" {
        return None;
    }

    let amt = USD::to_msats(dollars_from_par, sc.latest_price);
    match node.spontaneous_payment().send(amt, sc.counterparty, None) {
        Ok(payment_id) => {
            sc.payment_made = true;

            // Recalculate backing_sats to new equilibrium after payment.
            // Without this, the same drift is detected every check cycle,
            // causing repeated one-directional payments.
            if current_price > 0.0 {
                let btc_amount = target_usd / current_price;
                sc.backing_sats = (btc_amount * 100_000_000.0) as u64;
            }

            let payment_id_str = payment_id.to_string();
            let counterparty_str = sc.counterparty.to_string();
            audit_event("STABILITY_PAYMENT_SENT", json!({
                "amount_msats": amt,
                "payment_id": payment_id_str,
                "counterparty": counterparty_str,
                "expected_usd": target_usd,
                "new_backing_sats": sc.backing_sats
            }));
            Some(StabilityPaymentInfo {
                payment_id: payment_id_str,
                amount_msat: amt,
                counterparty: counterparty_str,
                btc_price: sc.latest_price,
            })
        }
        Err(e) => {
            audit_event("STABILITY_PAYMENT_FAILED", json!({
                "amount_msats": amt,
                "error": format!("{e}"),
                "counterparty": sc.counterparty.to_string()
            }));
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
        StableChannel {
            expected_usd: USD::from_f64(expected_usd),
            backing_sats: backing,
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
        assert!((d - 100.0).abs() < 0.01, "should deduct ~$100, got ${:.2}", d);
        assert!((sc.expected_usd.0 - 900.0).abs() < 0.01);
        // backing_sats should match new expected_usd
        let expected_backing = (900.0 / 100_000.0 * 100_000_000.0) as u64;
        assert_eq!(sc.backing_sats, expected_backing);
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
    fn incoming_resets_backing_to_equilibrium() {
        // $500 at $100k → backing should be 500k
        // Simulate: backing drifted to 600k for some reason
        let mut sc = test_sc(500.0, 100_000.0, 1_200_000);
        sc.backing_sats = 600_000; // drifted
        reconcile_incoming(&mut sc);
        let expected_backing = (500.0 / 100_000.0 * 100_000_000.0) as u64;
        assert_eq!(sc.backing_sats, expected_backing);
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
    fn incoming_skips_when_no_price() {
        let mut sc = test_sc(500.0, 0.0, 500_000);
        sc.backing_sats = 12345;
        reconcile_incoming(&mut sc);
        assert_eq!(sc.backing_sats, 12345); // unchanged
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
}