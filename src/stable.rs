use crate::lightning::LightningNode;
use crate::types::{Bitcoin, StableChannel, USD};
use ldk_node::{
    lightning::ln::types::ChannelId, Node,
};
use ureq::Agent;
use crate::price_feeds::get_cached_price;
use crate::audit::audit_event;
use serde_json::json;

pub fn get_current_price(agent: &Agent) -> f64 {
    let cached_price = get_cached_price();
    if cached_price > 0.0 {
        return cached_price;
    }
    match crate::price_feeds::get_latest_price(agent) {
        Ok(price) => price,
        Err(_) => 0.0 
    }
}

pub fn channel_exists(node: &dyn LightningNode, channel_id: &ChannelId) -> bool {
    let channels = node.list_channels();
    channels.iter().any(|c| c.channel_id == *channel_id)
}

pub fn update_balances<'a>(node: &dyn LightningNode, sc: &'a mut StableChannel) -> (bool, &'a mut StableChannel) {
    internal_update_balances(node.list_channels(), sc)
}

pub fn update_balances_node<'a>(node: &Node, sc: &'a mut StableChannel) -> (bool, &'a mut StableChannel) {
    internal_update_balances(node.list_channels(), sc)
}

fn internal_update_balances<'a>(
    channels: Vec<ldk_node::ChannelDetails>,
    sc: &'a mut StableChannel,
) -> (bool, &'a mut StableChannel) {
    if sc.latest_price == 0.0 {
        sc.latest_price = get_cached_price();
        if sc.latest_price == 0.0 {
            let agent = Agent::new();
            sc.latest_price = get_current_price(&agent);
        }
    }

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

        audit_event("BALANCE_UPDATE", json!({
            "channel_id": format!("{}", sc.channel_id),
            "stable_receiver_btc": sc.stable_receiver_btc.to_string(),
            "stable_provider_btc": sc.stable_provider_btc.to_string(),
            "stable_receiver_usd": sc.stable_receiver_usd.to_string(),
            "stable_provider_usd": sc.stable_provider_usd.to_string(),
            "btc_price": sc.latest_price
        }));

        return (true, sc);
    }

    println!("No matching channel found for ID: {}", sc.channel_id);
    (true, sc)
}

pub fn check_stability(node: &dyn LightningNode, sc: &mut StableChannel, price: f64) {
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
            return;
        }
    };

    sc.latest_price = current_price;
    let (success, _) = update_balances(node, sc);

    if !success {
        audit_event("BALANCE_UPDATE_FAILED", json!({
            "channel_id": format!("{}", sc.channel_id)
        }));
        return;
    }

    let dollars_from_par = sc.stable_receiver_usd - sc.expected_usd;
    let percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();
    let is_receiver_below_expected = sc.stable_receiver_usd < sc.expected_usd;

    let action = if percent_from_par < 0.1 {
        "STABLE"
    } else if sc.risk_level > 100 {
        "HIGH_RISK_NO_ACTION"
    } else if (sc.is_stable_receiver && is_receiver_below_expected)
        || (!sc.is_stable_receiver && !is_receiver_below_expected)
    {
        "CHECK_ONLY"
    } else {
        "PAY"
    };

    audit_event("STABILITY_CHECK", json!({
        "expected_usd": sc.expected_usd.0,
        "current_receiver_usd": sc.stable_receiver_usd.0,
        "percent_from_par": percent_from_par,
        "btc_price": sc.latest_price,
        "action": action,
        "is_stable_receiver": sc.is_stable_receiver,
        "risk_level": sc.risk_level
    }));

    if action != "PAY" {
        return;
    }

    let amt = USD::to_msats(dollars_from_par, sc.latest_price);
    match node.spontaneous_payment().send(amt, sc.counterparty, None) {
        Ok(payment_id) => {
            sc.payment_made = true;
            audit_event("STABILITY_PAYMENT_SENT", json!({
                "amount_msats": amt,
                "payment_id": payment_id.to_string(),
                "counterparty": sc.counterparty.to_string()
            }));
        }
        Err(e) => {
            audit_event("STABILITY_PAYMENT_FAILED", json!({
                "amount_msats": amt,
                "error": format!("{e}"),
                "counterparty": sc.counterparty.to_string()
            }));
        }
    }
}

pub fn check_stability_node(node: &Node, sc: &mut StableChannel, price: f64) {
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
            return;
        }
    };

    sc.latest_price = current_price;
    let (success, _) = update_balances_node(node, sc);

    if !success {
        audit_event("BALANCE_UPDATE_FAILED", json!({
            "channel_id": format!("{}", sc.channel_id)
        }));
        return;
    }

    let dollars_from_par = sc.stable_receiver_usd - sc.expected_usd;
    let percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();
    let is_receiver_below_expected = sc.stable_receiver_usd < sc.expected_usd;

    let action = if percent_from_par < 0.1 {
        "STABLE"
    } else if sc.risk_level > 100 {
        "HIGH_RISK_NO_ACTION"
    } else if (sc.is_stable_receiver && is_receiver_below_expected)
        || (!sc.is_stable_receiver && !is_receiver_below_expected)
    {
        "CHECK_ONLY"
    } else {
        "PAY"
    };

    audit_event("STABILITY_CHECK", json!({
        "expected_usd": sc.expected_usd.0,
        "current_receiver_usd": sc.stable_receiver_usd.0,
        "percent_from_par": percent_from_par,
        "btc_price": sc.latest_price,
        "action": action,
        "is_stable_receiver": sc.is_stable_receiver,
        "risk_level": sc.risk_level
    }));

    if action != "PAY" {
        return;
    }

    let amt = USD::to_msats(dollars_from_par, sc.latest_price);
    match node.spontaneous_payment().send(amt, sc.counterparty, None) {
        Ok(payment_id) => {
            sc.payment_made = true;
            audit_event("STABILITY_PAYMENT_SENT", json!({
                "amount_msats": amt,
                "payment_id": payment_id.to_string(),
                "counterparty": sc.counterparty.to_string()
            }));
        }
        Err(e) => {
            audit_event("STABILITY_PAYMENT_FAILED", json!({
                "amount_msats": amt,
                "error": format!("{e}"),
                "counterparty": sc.counterparty.to_string()
            }));
        }
    }
}
