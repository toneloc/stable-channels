use crate::types::{Bitcoin, StableChannel, USD};
use ldk_node::{
    lightning::ln::types::ChannelId, Node,
};
use ureq::Agent;
use crate::price_feeds::get_cached_price;
use crate::audit::audit_event;
use crate::constants::{STABILITY_THRESHOLD_PERCENT, MAX_RISK_LEVEL};
use serde_json::json;

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

        // Update native_channel_btc based on allocation
        // This is the portion that floats with BTC price
        let total_receiver_usd = sc.stable_receiver_usd.0;
        let native_usd = USD::from_f64(total_receiver_usd * sc.allocation.btc_weight);
        sc.native_channel_btc = Bitcoin::from_usd(native_usd, sc.latest_price);

        audit_event("BALANCE_UPDATE", json!({
            "channel_id": format!("{}", sc.channel_id),
            "stable_receiver_btc": sc.stable_receiver_btc.to_string(),
            "stable_provider_btc": sc.stable_provider_btc.to_string(),
            "stable_receiver_usd": sc.stable_receiver_usd.to_string(),
            "stable_provider_usd": sc.stable_provider_usd.to_string(),
            "native_channel_btc": sc.native_channel_btc.to_string(),
            "allocation_usd_weight": sc.allocation.usd_weight,
            "allocation_btc_weight": sc.allocation.btc_weight,
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

/// Check and enforce stability for a channel based on its allocation weights.
///
/// The stability logic now respects per-channel allocation:
/// - allocation.usd_weight determines what portion of the channel value is stabilized
/// - allocation.btc_weight portion floats with BTC price (native exposure)
///
/// For example, with allocation { usd_weight: 0.75, btc_weight: 0.25 }:
/// - 75% of the channel value is kept stable in USD terms
/// - 25% floats with the BTC market
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

    // Skip stability check if allocation is 100% BTC (no USD to stabilize)
    if sc.allocation.usd_weight < 0.001 {
        audit_event("STABILITY_SKIP", json!({
            "channel_id": format!("{}", sc.channel_id),
            "reason": "allocation is 100% BTC, no USD to stabilize",
            "allocation_usd_weight": sc.allocation.usd_weight
        }));
        return None;
    }

    // Skip if expected_usd is zero or very small
    if sc.expected_usd.0 < 0.01 {
        audit_event("STABILITY_SKIP", json!({
            "channel_id": format!("{}", sc.channel_id),
            "reason": "expected_usd is too small",
            "expected_usd": sc.expected_usd.0
        }));
        return None;
    }

    // Calculate the current USD value of the stable portion
    // The stable_receiver_usd includes the entire channel balance
    // We only stabilize the usd_weight portion
    let total_receiver_usd = sc.stable_receiver_usd.0;
    let current_stable_usd = total_receiver_usd * sc.allocation.usd_weight;

    // The target is expected_usd (which should be set based on allocation)
    let target_usd = sc.expected_usd.0;

    // Calculate deviation from the stable target
    let dollars_from_par = USD::from_f64(current_stable_usd - target_usd);
    let percent_from_par = if target_usd > 0.0 {
        ((dollars_from_par.0 / target_usd) * 100.0).abs()
    } else {
        0.0
    };
    let is_receiver_below_expected = current_stable_usd < target_usd;

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
        "current_stable_usd": current_stable_usd,
        "total_receiver_usd": total_receiver_usd,
        "allocation_usd_weight": sc.allocation.usd_weight,
        "allocation_btc_weight": sc.allocation.btc_weight,
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
            let payment_id_str = payment_id.to_string();
            let counterparty_str = sc.counterparty.to_string();
            audit_event("STABILITY_PAYMENT_SENT", json!({
                "amount_msats": amt,
                "payment_id": payment_id_str,
                "counterparty": counterparty_str,
                "allocation_usd_weight": sc.allocation.usd_weight
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
}