//! Pure channel-ledger logic: paging, labels, help text, formatting and JSONL export.

use std::collections::HashSet;

use chrono::{TimeZone, Utc};
use sc_rest_client::sc_protos::stable::{
    AccountingSnapshot, ChannelLedgerEvent, ChannelLedgerOverview, ListChannelLedgerEventsResponse,
};

use crate::format::format_sats;

pub fn merge_ledger_events(existing: &mut Vec<ChannelLedgerEvent>, incoming: Vec<ChannelLedgerEvent>) {
    let mut seen = existing
        .iter()
        .map(|event| event.id)
        .collect::<HashSet<_>>();
    existing.extend(incoming.into_iter().filter(|event| seen.insert(event.id)));
    existing.sort_by_key(|event| (event.occurred_at_ms, event.id));
}

pub fn checked_next_cursor(
    requested_cursor: &str,
    next_cursor: Option<String>,
    seen_cursors: &mut HashSet<String>,
) -> Result<Option<String>, String> {
    let Some(next_cursor) = next_cursor else {
        return Ok(None);
    };
    if next_cursor.is_empty()
        || next_cursor == requested_cursor
        || !seen_cursors.insert(next_cursor.clone())
    {
        return Err(
            "Ledger export stopped because the server repeated a pagination cursor".to_owned(),
        );
    }
    Ok(Some(next_cursor))
}

pub fn loaded_events_caption(loaded: usize, matching: u64, has_more: bool) -> Option<String> {
    has_more.then(|| format!("Showing {loaded} of {matching} matching events — Load older for more"))
}

pub fn filter_choice_label<'a>(filter: &str, value: &'a str) -> &'a str {
    if filter == "Completeness" && value == "observed" {
        "direct"
    } else {
        value
    }
}

pub fn latest_state_caption(overview: &ChannelLedgerOverview) -> String {
    let source = match overview.latest_accounting_source.as_str() {
        "channels" => "Current SQLite state",
        "ledger" => "Latest complete snapshot",
        _ => "No complete state",
    };
    match overview.latest_accounting_at_ms {
        Some(timestamp) => format!("{source} · {}", relative_timestamp(timestamp)),
        None => source.to_owned(),
    }
}

pub fn timeline_order(events: &[ChannelLedgerEvent], newest_first: bool) -> Vec<usize> {
    let mut order = (0..events.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| (events[*index].occurred_at_ms, events[*index].id));
    if newest_first {
        order.reverse();
    }
    order
}

pub fn snapshot_rows(snapshot: &AccountingSnapshot) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(value) = snapshot.expected_usd {
        rows.push(("Expected USD", format!("${value:.2}")));
    }
    if let Some(value) = snapshot.backing_sats {
        rows.push(("Backing", format_sats_with_usd(value, snapshot.btc_price)));
    }
    if let Some(value) = snapshot.native_sats {
        rows.push(("Native", format_sats_with_usd(value, snapshot.btc_price)));
    }
    if let Some(value) = snapshot.live_receiver_sats {
        rows.push((
            "Live balance",
            format_sats_with_usd(value, snapshot.btc_price),
        ));
    }
    if let Some(value) = snapshot.amount_sats {
        rows.push(("Amount", format_sats_with_usd(value, snapshot.btc_price)));
    } else if let Some(value) = snapshot.amount_msat {
        rows.push(("Amount", format_msat(value)));
    }
    if let Some(value) = snapshot.amount_usd {
        rows.push(("Recorded amount", format!("${value:.2}")));
    }
    if let Some(value) = snapshot.fee_sats {
        rows.push(("Fee", format_sats_with_usd(value, snapshot.btc_price)));
    } else if let Some(value) = snapshot.fee_msat {
        rows.push(("Fee", format_msat(value)));
    }
    rows
}

/// Signed change such as "+$1.50" or "−$1.50" (true minus sign); empty when nothing changed.
pub fn decimal_delta(before: Option<f64>, after: Option<f64>, prefix: &str) -> Option<(String, i8)> {
    let delta = after? - before?;
    let cents = (delta * 100.0).round();
    let (sign, text) = if cents > 0.0 {
        (1, format!("+{prefix}{:.2}", delta.abs()))
    } else if cents < 0.0 {
        (-1, format!("\u{2212}{prefix}{:.2}", delta.abs()))
    } else {
        (0, String::new())
    };
    Some((text, sign))
}

/// Signed change such as "+1,234 sats" or "−328 sats"; empty when nothing changed.
pub fn sats_delta(before: Option<u64>, after: Option<u64>) -> Option<(String, i8)> {
    let delta = after? as i128 - before? as i128;
    let magnitude = format_sats(delta.unsigned_abs() as u64);
    let text = match delta.signum() {
        1 => format!("+{magnitude} sats"),
        -1 => format!("\u{2212}{magnitude} sats"),
        _ => String::new(),
    };
    Some((text, delta.signum() as i8))
}

#[cfg(test)]
fn accounting_delta(
    before: Option<&AccountingSnapshot>,
    after: Option<&AccountingSnapshot>,
) -> Option<String> {
    let before = before?;
    let after = after?;
    let mut parts = Vec::new();
    if let (Some(a), Some(b)) = (before.expected_usd, after.expected_usd) {
        parts.push(format!("expected_usd {a:.2} -> {b:.2} ({:+.2})", b - a));
    }
    if let (Some(a), Some(b)) = (before.backing_sats, after.backing_sats) {
        parts.push(format!("backing {a} -> {b} ({:+})", b as i128 - a as i128));
    }
    if let (Some(a), Some(b)) = (before.native_sats, after.native_sats) {
        parts.push(format!("native {a} -> {b} ({:+})", b as i128 - a as i128));
    }
    (!parts.is_empty()).then(|| parts.join("  •  "))
}

pub fn human_summary(event: &ChannelLedgerEvent) -> String {
    match event.event_type.as_str() {
        "CHANNEL_PENDING" => "Channel opening started".to_owned(),
        "CHANNEL_READY_TRACKED" => "Channel ready".to_owned(),
        "CHANNEL_OPEN_FAILED" => "Channel opening failed".to_owned(),
        "STABLE_EDITED" | "TRADE_APPLIED" | "SYNC_V1_APPLIED" => "Stable target changed".to_owned(),
        "PAYMENT_OUTGOING_RECONCILED" | "OUTGOING_STABLE_DEDUCTED" | "STABLE_SPEND_DEDUCTED" => {
            "Outgoing payment reduced stable backing".to_owned()
        }
        "SPLICE_IN_RECONCILED" => "Splice in completed".to_owned(),
        "SPLICE_OUT_STABLE_RECONCILED" => "Splice out completed".to_owned(),
        "CHANNEL_READY_SPLICE" => match splice_direction(event).as_deref() {
            Some("in") => "Splice in completed".to_owned(),
            Some("out") => "Splice out completed".to_owned(),
            _ => "Splice completed".to_owned(),
        },
        "SPLICE_RECONCILED" => "Splice completed".to_owned(),
        "SPLICE_OUT_STABLE_DEDUCTED" => "Splice out reduced stable backing".to_owned(),
        "STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN" => "Stability payment outcome unknown".to_owned(),
        "STABILITY_TOP_UP_DEFERRED_STILL_PENDING" => "Stability payment still unclaimed".to_owned(),
        "STABILITY_TOP_UP_BOOKING_FAILED" => "Claimed stability payment not booked".to_owned(),
        "STABILITY_TOP_UP_LOOKUP_FAILED" => "Stability payment outcome could not be checked".to_owned(),
        "STABILITY_PAYMENT_SENT" => "Stability payment sent".to_owned(),
        "STABILITY_PAYMENT_SETTLED" => "Stability payment completed".to_owned(),
        "EVENT_STREAM_GAP_CLOSED" => "Channel recovered after reconnect".to_owned(),
        "CHANNEL_ACCOUNTING_STATE_COMMITTED" => "Channel accounting state recorded".to_owned(),
        "CHANNEL_CLOSED_COMMITTED" | "CHANNEL_CLOSED" => "Channel closed".to_owned(),
        "STABILITY_PAYMENT_RECORDED" => "Stability payment recorded".to_owned(),
        "MESSAGE_RECEIVED" => "Channel message received".to_owned(),
        "TRADE_SIGNATURE_VALID" => "Channel message signature verified".to_owned(),
        "SYNC_MESSAGE_SENT" => "Accounting sync delivered".to_owned(),
        "PAYMENT_SETTLED" if event_amount_msat(event) == Some(1) => {
            "Accounting sync settled".to_owned()
        }
        "PAYMENT_FAILED" => match detail_text(event, "reason") {
            Some(reason) => format!("Payment failed: {}", humanize_enum(&reason)),
            None => "Payment failed".to_owned(),
        },
        "SPLICE_NEGOTIATED" => "Splice negotiated".to_owned(),
        "SPLICE_NEGOTIATION_FAILED" => "Splice negotiation failed".to_owned(),
        "CHANNEL_SHUTDOWN_STATE_CHANGED" => match detail_text(event, "shutdown_state") {
            Some(state) => format!("Channel shutdown: {}", humanize_enum(&state)),
            None => "Channel shutdown stage changed".to_owned(),
        },
        "CHANNEL_ONCHAIN_TX" => {
            let state = if event.status == "failed" {
                "failed"
            } else if detail_text(event, "confirmation").as_deref() == Some("confirmed") {
                "confirmed"
            } else {
                "broadcast"
            };
            format!("{} {state}", onchain_tx_label(detail_text(event, "tx_type").as_deref()))
        }
        unknown => title_case_event(unknown),
    }
}

pub fn event_help(event: &ChannelLedgerEvent) -> String {
    let explanation = match event.event_type.as_str() {
        "STABLE_EDITED" => "An operator changed the channel's target stable USD amount.",
        "TRADE_APPLIED" => "A validated BTC/USD trade updated the channel's stable allocation.",
        "SYNC_V1_APPLIED" => {
            "A newer signed allocation from the wallet was accepted and applied to this channel."
        }
        "PAYMENT_OUTGOING_RECONCILED" | "OUTGOING_STABLE_DEDUCTED" | "STABLE_SPEND_DEDUCTED" => {
            "An outgoing Lightning payment used stable-backed capacity, so the recorded stable backing was reduced."
        }
        "SPLICE_RECONCILED" => {
            "LDK reported the channel ready after a splice, and the LSP reconciled its current capacity and allocation."
        }
        "SPLICE_IN_RECONCILED" => {
            "A splice in added funds to the channel. The channel became ready again and its accounting was updated."
        }
        "SPLICE_OUT_STABLE_RECONCILED" => {
            "A splice out removed funds from the channel. The channel became ready again and its stable accounting was updated."
        }
        "CHANNEL_READY_SPLICE" => return splice_help(event),
        "SPLICE_OUT_STABLE_DEDUCTED" => {
            "The splice out removed more than the channel's native balance, so the remaining amount reduced its stable backing."
        }
        "STABILITY_PUSH_QUEUED" => {
            "The wallet was offline, so the LSP queued a push notification asking it to reconnect and check stability. No stability payment was sent yet."
        }
        "STABILITY_CHECK_ONLY" => {
            "The channel was above its target, but the LSP cannot pull value from the wallet, so it recorded the check without sending a payment."
        }
        "STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN" => {
            "The LSP sent a stability payment but the node has no record of it. The payment is neither booked nor resent until its outcome is known."
        }
        "STABILITY_TOP_UP_DEFERRED_STILL_PENDING" => {
            "A stability payment has been waiting to be claimed for over an hour, usually because the wallet is offline. The channel gets no other stability payment until this one is claimed or fails."
        }
        "STABILITY_TOP_UP_LOOKUP_FAILED" => {
            "The node could not be asked whether a stability payment on its way was claimed, and another event for the LSP was handled meanwhile. If that event was a payment on this channel, its stable target may be too high. Compare the target with the wallet's balance once the payment settles."
        }
        "STABILITY_TOP_UP_BOOKING_FAILED" => {
            "A stability payment was claimed but the channel's record could not be found, so it was not added to the stable backing. It stays pending and no other stability payment is sent to this channel until the record is found."
        }
        "STABILITY_PAYMENT_SENT" => {
            "The LSP sent a Lightning payment to move the channel's stable value toward its target."
        }
        "STABILITY_PAYMENT_SETTLED" => {
            "The stability payment completed successfully and is no longer in flight."
        }
        "STABILITY_PAYMENT_RECORDED" => {
            "A stability payment was associated with this channel and stored for settlement tracking."
        }
        "EVENT_STREAM_CONNECTED" => {
            "The LSP connected to LDK Server's live event stream and resumed listening for activity."
        }
        "EVENT_STREAM_GAP_STARTED" => {
            "The LSP lost the live LDK event stream, so activity during this interval may need reconstruction."
        }
        "EVENT_STREAM_GAP_CLOSED" => {
            "The LSP reconnected to LDK Server and completed its recovery check for the missed interval."
        }
        "CHANNEL_RECONSTRUCTED" => {
            "After reconnecting, the LSP rebuilt this snapshot from current LDK channel data. The channel itself was not recreated."
        }
        "PAYMENT_RECONSTRUCTED" => {
            "After reconnecting, the LSP rebuilt this payment record from LDK's current payment history."
        }
        "PEER_RECONSTRUCTED" => {
            "After reconnecting, the LSP rebuilt this peer snapshot from LDK's current peer list."
        }
        "SWEEP_RECONSTRUCTED" => {
            "After reconnecting, the LSP rebuilt this pending sweep snapshot from LDK's current balances."
        }
        "PAYMENT_FORWARDED_BACKFILL" => {
            return with_skimmed_fee(
                event,
                "The LSP found a forwarded payment in LDK history that was not observed on the live event stream and added it to the ledger.",
            );
        }
        "PAYMENT_FORWARDED" => {
            return with_skimmed_fee(
                event,
                "The LSP routed this payment between the incoming and outgoing channels shown below.",
            );
        }
        "SPLICE_NEGOTIATED" => {
            "The splice was agreed with the peer and its new funding transaction is waiting for confirmation. The channel's accounting is reconciled when LDK reports it ready again."
        }
        "SPLICE_NEGOTIATION_FAILED" => {
            "A splice negotiation round with the peer failed. Nothing changed on-chain: the channel keeps its current funding and any splice already negotiated."
        }
        "CHANNEL_SHUTDOWN_STATE_CHANGED" => {
            "The channel moved to a new cooperative close stage. Pending HTLCs are resolved first, then the closing fee is negotiated and the close transaction is broadcast. The LSP checks every 30 seconds, so a quick close can skip stages, and a stage the channel falls back to after a disconnect is not recorded again."
        }
        "RECONCILIATION_SCOPE_FAILED" => {
            "Part of the reconnect recovery could not be queried. The affected scope and error are available in Raw JSON."
        }
        "CHANNEL_ACCOUNTING_STATE_COMMITTED" => {
            "The latest expected USD, backing, native balance, and live balance were saved as one accounting snapshot."
        }
        "CHANNEL_READY_TRACKED" => {
            "The channel opening finished. LDK marked the channel ready for Lightning payments, and the LSP began tracking its stable accounting."
        }
        "CHANNEL_PENDING" => {
            "The channel opening started. Its funding transaction was created, and it is waiting for confirmations before it can carry Lightning payments."
        }
        "CHANNEL_OPEN_FAILED" => {
            "The channel opening stopped before the channel became usable. Open Raw JSON to see the recorded reason."
        }
        "CHANNEL_CLOSED_COMMITTED" | "CHANNEL_CLOSED" => {
            "The channel was closed and can no longer carry payments. The LSP stopped tracking it as an active stable channel."
        }
        "MESSAGE_RECEIVED" => {
            "The LSP received a Stable Channels protocol message carried in a custom Lightning record."
        }
        "TRADE_PARSED_PAYLOAD_OK" => {
            "The received trade message had the expected structure and could be decoded."
        }
        "TRADE_SIGNATURE_VALID" => {
            "The cryptographic signature on the received channel message was successfully verified."
        }
        "SYNC_MESSAGE_SENT" | "TRADE_MESSAGE_SENT" => {
            "A Stable Channels protocol message was delivered to the counterparty over Lightning."
        }
        "PAYMENT_SETTLED" if event_amount_msat(event) == Some(1) => {
            "The 1-msat carrier payment used to deliver an accounting sync completed successfully."
        }
        "PAYMENT_SETTLED" | "PAYMENT_SUCCESSFUL" => {
            "The Lightning payment completed successfully."
        }
        "PAYMENT_FAILED" => return failed_payment_help(event),
        "CHANNEL_ONCHAIN_TX" => return onchain_tx_help(event),
        _ => {
            return format!(
                "This is {}. Hover the badges for classification details or open Raw JSON for the exact recorded fields.",
                category_help_phrase(&event.category)
            );
        },
    };
    explanation.to_owned()
}

fn detail_value(event: &ChannelLedgerEvent, key: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(&event.detail_json).ok()?.get(key).cloned()
}

fn detail_text(event: &ChannelLedgerEvent, key: &str) -> Option<String> {
    detail_value(event, key)?.as_str().map(str::to_owned)
}

// LDK enum names such as ROUTE_NOT_FOUND read as "route not found".
pub(crate) fn humanize_enum(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', " ").replace("htlcs", "HTLCs")
}

fn onchain_tx_label(tx_type: Option<&str>) -> &'static str {
    match tx_type {
        Some("FUNDING") => "Funding transaction",
        Some("INTERACTIVE_FUNDING") => "Interactive funding transaction",
        Some("COOPERATIVE_CLOSE") => "Cooperative close transaction",
        Some("UNILATERAL_CLOSE") => "Force-close transaction",
        Some("ANCHOR_BUMP") => "Close fee-bump transaction",
        Some("CLAIM") => "Claim transaction",
        Some("SWEEP") => "Sweep transaction",
        _ => "On-chain channel transaction",
    }
}

fn onchain_tx_help(event: &ChannelLedgerEvent) -> String {
    let what = match detail_text(event, "tx_type").as_deref() {
        Some("FUNDING") => "This transaction funds the channel.",
        Some("INTERACTIVE_FUNDING") => "This transaction was negotiated together with the peer, such as a splice, and becomes the channel's new funding.",
        Some("COOPERATIVE_CLOSE") => "Both sides agreed to close the channel, and this transaction pays out their balances.",
        Some("UNILATERAL_CLOSE") => "One side force-closed the channel by broadcasting its latest commitment transaction.",
        Some("ANCHOR_BUMP") => "LDK added fees to a closing transaction through its anchor output so it confirms in time.",
        Some("CLAIM") => "LDK claimed funds from the channel's closing transaction.",
        Some("SWEEP") => "LDK swept the channel's claimable outputs back to its on-chain wallet.",
        _ => "LDK classified this on-chain transaction as belonging to the channel.",
    };
    let height = detail_value(event, "confirmation_height").and_then(|height| height.as_u64());
    let when = match height {
        _ if event.status == "failed" => "It did not confirm and was dropped or replaced.".to_owned(),
        Some(height) => format!("It confirmed in block {height}."),
        None => "It is waiting for confirmation.".to_owned(),
    };
    format!("{what} {when}")
}

fn failed_payment_help(event: &ChannelLedgerEvent) -> String {
    let base = "The Lightning payment did not complete successfully.";
    let Some(reason) = detail_text(event, "reason") else {
        return base.to_owned();
    };
    let cause = match reason.as_str() {
        "ROUTE_NOT_FOUND" => "LDK found no route to the recipient. For a stability payment or sync this usually means the wallet was offline or the channel lacked capacity.".to_owned(),
        "RECIPIENT_REJECTED" => "The recipient's node rejected it.".to_owned(),
        "RETRIES_EXHAUSTED" => "LDK used up its retry attempts or its retry timeout.".to_owned(),
        "PAYMENT_EXPIRED" => "It expired while LDK was still retrying.".to_owned(),
        "USER_ABANDONED" => "It was abandoned before it completed.".to_owned(),
        "UNEXPECTED_ERROR" => "LDK hit an unexpected routing error.".to_owned(),
        other => format!("LDK reported the reason as {}.", humanize_enum(other)),
    };
    format!("{base} {cause}")
}

fn with_skimmed_fee(event: &ChannelLedgerEvent, explanation: &str) -> String {
    match detail_value(event, "skimmed_fee_msat").and_then(|fee| fee.as_u64()).filter(|fee| *fee > 0) {
        Some(fee) => format!(
            "{explanation} {} of the fee was withheld as the channel-open fee for a just-in-time channel.",
            format_msat(fee)
        ),
        None => explanation.to_owned(),
    }
}

fn splice_direction(event: &ChannelLedgerEvent) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&event.detail_json)
        .ok()?
        .get("direction")?
        .as_str()
        .map(str::to_owned)
}

fn splice_amount_sats(event: &ChannelLedgerEvent) -> Option<u64> {
    event
        .after
        .as_ref()
        .and_then(|snapshot| snapshot.amount_sats)
        .or_else(|| {
            serde_json::from_str::<serde_json::Value>(&event.detail_json)
                .ok()?
                .get("amount_sats")?
                .as_u64()
        })
}

fn splice_help(event: &ChannelLedgerEvent) -> String {
    let amount = splice_amount_sats(event)
        .map(|amount| format!("{} sats net", format_sats(amount)))
        .unwrap_or_else(|| "funds".to_owned());
    match splice_direction(event).as_deref() {
        Some("in") => format!(
            "A splice in added {amount} to the channel. The channel became ready again and its new balance was stored."
        ),
        Some("out") => format!(
            "A splice out removed {amount} from the channel. The channel became ready again and its stable accounting was reconciled."
        ),
        Some("unchanged") => {
            "LDK reported the channel ready after a splice, but its recorded balance was unchanged. This can be a replay or recovery event."
                .to_owned()
        },
        _ => {
            "LDK reported the channel ready after a splice, and the LSP reconciled its current balance and stable accounting."
                .to_owned()
        },
    }
}

fn category_help_phrase(category: &str) -> &'static str {
    match category {
        "channel" => "a channel lifecycle event",
        "payment" => "a Lightning payment event",
        "forwarding" => "a routed-payment event",
        "trade" => "a trade or stable-allocation event",
        "stability" => "a stabilization or accounting-sync event",
        "peer" => "a peer-connection event",
        "sweep" => "a channel-closing sweep event",
        "reconciliation" => "a recovery or backfill event",
        "operator" => "an operator action",
        "system" => "an internal system event",
        _ => "an unclassified ledger event",
    }
}

fn title_case_event(event_type: &str) -> String {
    let mut words = event_type
        .split('_')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if let Some(first) = words.first_mut() {
        if let Some(initial) = first.get_mut(0..1) {
            initial.make_ascii_uppercase();
        }
    }
    if words.is_empty() {
        "Unknown event".to_owned()
    } else {
        words.join(" ")
    }
}

fn event_amount_msat(event: &ChannelLedgerEvent) -> Option<u64> {
    event
        .after
        .as_ref()
        .and_then(|snapshot| snapshot.amount_msat)
        .or_else(|| {
            serde_json::from_str::<serde_json::Value>(&event.detail_json)
                .ok()
                .and_then(|detail| detail.get("amount_msat").and_then(|value| value.as_u64()))
        })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ForwardingLeg {
    pub channel_id: Option<String>,
    pub user_channel_id: Option<String>,
    pub node_id: Option<String>,
}

impl ForwardingLeg {
    fn is_empty(&self) -> bool {
        self.channel_id.is_none() && self.user_channel_id.is_none() && self.node_id.is_none()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ForwardingPath {
    pub incoming: ForwardingLeg,
    pub outgoing: ForwardingLeg,
}

pub fn forwarding_path(event: &ChannelLedgerEvent) -> Option<ForwardingPath> {
    if !matches!(
        event.event_type.as_str(),
        "PAYMENT_FORWARDED" | "PAYMENT_FORWARDED_BACKFILL"
    ) {
        return None;
    }
    let detail = serde_json::from_str::<serde_json::Value>(&event.detail_json).ok()?;
    let text = |key: &str| {
        detail.get(key).and_then(|value| match value {
            serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
            serde_json::Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    };
    let path = ForwardingPath {
        incoming: ForwardingLeg {
            channel_id: text("prev_channel_id"),
            user_channel_id: text("prev_user_channel_id"),
            node_id: text("prev_node_id"),
        },
        outgoing: ForwardingLeg {
            channel_id: text("next_channel_id"),
            user_channel_id: text("next_user_channel_id"),
            node_id: text("next_node_id"),
        },
    };
    (!path.incoming.is_empty() || !path.outgoing.is_empty()).then_some(path)
}

pub fn category_help(category: &str) -> String {
    let meaning = match category {
        "channel" => "Channel lifecycle, readiness, splice, or closure activity",
        "payment" => "Lightning payment activity",
        "forwarding" => "Routed payment activity",
        "trade" => "BTC/USD trade or stable-allocation activity",
        "stability" => "Stabilization payment or accounting-sync activity",
        "peer" => "Peer connection activity",
        "sweep" => "Closing-output sweep activity",
        "reconciliation" => "Recovery, backfill, or event-gap processing",
        "operator" => "Manual edit or configuration activity",
        "system" => "Internal system activity",
        _ => "Unclassified ledger activity",
    };
    format!("Category: {meaning}")
}

pub fn status_help(status: &str) -> String {
    let meaning = match status {
        "observed" => "Informational event; no workflow completion is implied",
        "pending" => "Operation is still in progress",
        "completed" => "Operation finished or was applied successfully",
        "partial" => "Only part of the operation completed successfully",
        "failed" => "Operation failed or was rejected",
        "skipped" => "Operation was intentionally not performed",
        _ => "Unrecognized event status",
    };
    format!("Status: {meaning}")
}

pub fn completeness_label(completeness: &str) -> &str {
    match completeness {
        "observed" => "direct",
        other => other,
    }
}

pub fn completeness_help(completeness: &str) -> String {
    let meaning = match completeness {
        "observed" => "Recorded directly when the event occurred",
        "reconstructed" => "Rebuilt later from other available records",
        "legacy" => "Imported from the older JSONL audit log and may lack structured state",
        "gap" => "Marks known missing or incomplete event coverage",
        _ => "Unrecognized record completeness",
    };
    format!("Completeness: {meaning}")
}

/// Visual tone (CSS class suffix) for an event status badge.
pub fn status_tone(status: &str) -> &'static str {
    match status {
        "failed" => "danger",
        "completed" => "success",
        "skipped" => "muted",
        _ => "warning",
    }
}

/// Visual tone (CSS class suffix) for an event completeness badge.
pub fn completeness_tone(completeness: &str) -> &'static str {
    match completeness {
        "observed" => "success",
        "gap" => "danger",
        "legacy" => "muted",
        _ => "warning",
    }
}

pub fn format_sats_with_usd(sats: u64, btc_price: Option<f64>) -> String {
    let display = format!("{} sats", format_sats(sats));
    match btc_price.filter(|price| price.is_finite() && *price > 0.0) {
        Some(price) => format!("{display} · ≈ ${:.2}", sats_to_usd(sats, price)),
        None => display,
    }
}

fn sats_to_usd(sats: u64, btc_price: f64) -> f64 {
    sats as f64 / 100_000_000.0 * btc_price
}

fn format_msat(msat: u64) -> String {
    if msat % 1_000 == 0 {
        format!("{} sats", format_sats(msat / 1_000))
    } else {
        format!("{} msat", format_sats(msat))
    }
}

pub fn relative_timestamp(timestamp_ms: i64) -> String {
    let seconds = (Utc::now().timestamp_millis() - timestamp_ms) / 1_000;
    if seconds < 0 {
        return "in the future".to_owned();
    }
    match seconds {
        0..=4 => "just now".to_owned(),
        5..=59 => format!("{seconds} seconds ago"),
        60..=119 => "1 minute ago".to_owned(),
        120..=3_599 => format!("{} minutes ago", seconds / 60),
        3_600..=7_199 => "1 hour ago".to_owned(),
        7_200..=86_399 => format!("{} hours ago", seconds / 3_600),
        86_400..=172_799 => "1 day ago".to_owned(),
        172_800..=2_591_999 => format!("{} days ago", seconds / 86_400),
        2_592_000..=5_183_999 => "1 month ago".to_owned(),
        5_184_000..=31_535_999 => format!("{} months ago", seconds / 2_592_000),
        31_536_000..=63_071_999 => "1 year ago".to_owned(),
        _ => format!("{} years ago", seconds / 31_536_000),
    }
}

pub fn exact_timestamp(timestamp_ms: i64) -> String {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|timestamp| timestamp.format("%d %b %Y, %H:%M:%S%.3f UTC").to_string())
        .unwrap_or_else(|| format!("{timestamp_ms} ms"))
}

fn snapshot_json(snapshot: &AccountingSnapshot) -> serde_json::Value {
    serde_json::json!({
        "expected_usd": snapshot.expected_usd,
        "backing_sats": snapshot.backing_sats,
        "native_sats": snapshot.native_sats,
        "live_receiver_sats": snapshot.live_receiver_sats,
        "btc_price": snapshot.btc_price,
        "amount_sats": snapshot.amount_sats,
        "amount_msat": snapshot.amount_msat,
        "amount_usd": snapshot.amount_usd,
        "fee_sats": snapshot.fee_sats,
        "fee_msat": snapshot.fee_msat,
    })
}

pub fn history_jsonl(history: &ListChannelLedgerEventsResponse) -> String {
    let mut events = history.events.iter().collect::<Vec<_>>();
    events.sort_by_key(|event| (event.occurred_at_ms, event.id));
    let mut seen = HashSet::new();
    events
        .into_iter()
        .filter(|event| seen.insert(event.id))
        .map(|event| {
            serde_json::json!({
                "ledger_id": event.id,
                "occurred_at_ms": event.occurred_at_ms,
                "recorded_at_ms": event.recorded_at_ms,
                "event": event.event_type,
                "category": event.category,
                "severity": event.severity,
                "status": event.status,
                "source": event.source,
                "completeness": event.completeness,
                "dedup_key": event.dedup_key,
                "before": event.before.as_ref().map(snapshot_json),
                "after": event.after.as_ref().map(snapshot_json),
                "refs": event.refs.iter().map(|reference| serde_json::json!({
                    "role": reference.role,
                    "value": reference.value,
                })).collect::<Vec<_>>(),
                "data": serde_json::from_str::<serde_json::Value>(&event.detail_json)
                    .unwrap_or_else(|_| serde_json::Value::String(event.detail_json.clone())),
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: i64, event_type: &str) -> ChannelLedgerEvent {
        ChannelLedgerEvent {
            id,
            event_type: event_type.to_owned(),
            occurred_at_ms: id * 10,
            status: "completed".to_owned(),
            severity: "info".to_owned(),
            completeness: "observed".to_owned(),
            detail_json: "{}".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn completeness_display_distinguishes_direct_records_from_status() {
        assert_eq!(completeness_label("observed"), "direct");
        assert_eq!(filter_choice_label("Completeness", "observed"), "direct");
        assert_eq!(filter_choice_label("Status", "observed"), "observed");
        assert!(completeness_help("observed").contains("Recorded directly"));
        assert!(status_help("observed").contains("Informational event"));
    }

    #[test]
    fn deltas_read_as_signed_amounts_and_stay_empty_when_nothing_changed() {
        assert_eq!(decimal_delta(Some(43.63), Some(43.63), "$"), Some((String::new(), 0)));
        assert_eq!(decimal_delta(Some(1.0), Some(2.5), "$"), Some(("+$1.50".to_owned(), 1)));
        assert_eq!(decimal_delta(Some(2.5), Some(1.0), "$"), Some(("\u{2212}$1.50".to_owned(), -1)));
        assert_eq!(sats_delta(Some(52_772), Some(52_444)), Some(("\u{2212}328 sats".to_owned(), -1)));
        assert_eq!(sats_delta(Some(1_000), Some(2_234)), Some(("+1,234 sats".to_owned(), 1)));
        assert_eq!(sats_delta(Some(16_485), Some(16_485)), Some((String::new(), 0)));
    }

    #[test]
    fn accounting_delta_reports_before_after() {
        let before = AccountingSnapshot {
            backing_sats: Some(10),
            native_sats: Some(5),
            ..Default::default()
        };
        let after = AccountingSnapshot {
            backing_sats: Some(12),
            native_sats: Some(3),
            ..Default::default()
        };
        let text = accounting_delta(Some(&before), Some(&after)).unwrap();
        assert!(text.contains("backing 10 -> 12 (+2)"));
        assert!(text.contains("native 5 -> 3 (-2)"));
        assert!(accounting_delta(None, Some(&after)).is_none());
    }

    #[test]
    fn human_summaries_and_unknown_fallback_are_readable() {
        assert_eq!(
            human_summary(&event(1, "STABLE_EDITED")),
            "Stable target changed"
        );
        assert_eq!(
            human_summary(&event(2, "PAYMENT_OUTGOING_RECONCILED")),
            "Outgoing payment reduced stable backing"
        );
        assert_eq!(
            human_summary(&event(3, "SPLICE_RECONCILED")),
            "Splice completed"
        );
        assert_eq!(human_summary(&event(4, "A_NEW_EVENT")), "A new event");
        assert_eq!(
            human_summary(&event(5, "CHANNEL_PENDING")),
            "Channel opening started"
        );
        assert_eq!(
            human_summary(&event(6, "CHANNEL_READY_TRACKED")),
            "Channel ready"
        );

        let mut splice_in = event(7, "CHANNEL_READY_SPLICE");
        splice_in.detail_json = r#"{"direction":"in","amount_sats":9769}"#.to_owned();
        assert_eq!(human_summary(&splice_in), "Splice in completed");

        let mut splice_out = event(8, "CHANNEL_READY_SPLICE");
        splice_out.detail_json = r#"{"direction":"out","amount_sats":5000}"#.to_owned();
        assert_eq!(human_summary(&splice_out), "Splice out completed");
    }

    #[test]
    fn event_help_explains_operator_facing_titles() {
        assert!(event_help(&event(1, "STABILITY_PUSH_QUEUED")).contains("No stability payment"));
        assert!(event_help(&event(2, "CHANNEL_RECONSTRUCTED")).contains("not recreated"));
        assert!(event_help(&event(3, "SPLICE_RECONCILED")).contains("reconciled"));
        assert!(
            event_help(&event(4, "CHANNEL_PENDING")).contains("funding transaction was created")
        );
        assert!(event_help(&event(5, "CHANNEL_READY_TRACKED")).contains("opening finished"));

        let mut one_msat = event(6, "PAYMENT_SETTLED");
        one_msat.detail_json = r#"{"amount_msat":1}"#.to_owned();
        assert!(event_help(&one_msat).contains("carrier payment"));

        let mut splice_in = event(7, "CHANNEL_READY_SPLICE");
        splice_in.detail_json = r#"{"direction":"in","amount_sats":9769}"#.to_owned();
        assert!(event_help(&splice_in).contains("9,769 sats net"));

        let legacy_splice = event(8, "CHANNEL_READY_SPLICE");
        assert_eq!(human_summary(&legacy_splice), "Splice completed");
        assert!(event_help(&legacy_splice).contains("reconciled"));

        let settled = event(9, "STABILITY_PAYMENT_SETTLED");
        assert_eq!(human_summary(&settled), "Stability payment completed");
        assert!(event_help(&settled).contains("no longer in flight"));
    }

    #[test]
    fn loaded_count_explains_remaining_pages() {
        assert_eq!(
            loaded_events_caption(50, 300, true).as_deref(),
            Some("Showing 50 of 300 matching events — Load older for more")
        );
        assert_eq!(loaded_events_caption(50, 50, false), None);
    }

    #[test]
    fn forwarded_payment_preserves_incoming_and_outgoing_leg_roles() {
        let mut forwarded = event(5, "PAYMENT_FORWARDED");
        forwarded.detail_json = serde_json::json!({
            "prev_channel_id": "incoming-channel",
            "prev_user_channel_id": "incoming-user-channel",
            "prev_node_id": "incoming-peer",
            "next_channel_id": "outgoing-channel",
            "next_user_channel_id": "outgoing-user-channel",
            "next_node_id": "outgoing-peer",
        })
        .to_string();

        let path = forwarding_path(&forwarded).unwrap();
        assert_eq!(
            path.incoming.channel_id.as_deref(),
            Some("incoming-channel")
        );
        assert_eq!(
            path.incoming.user_channel_id.as_deref(),
            Some("incoming-user-channel")
        );
        assert_eq!(path.incoming.node_id.as_deref(), Some("incoming-peer"));
        assert_eq!(
            path.outgoing.channel_id.as_deref(),
            Some("outgoing-channel")
        );
        assert_eq!(
            path.outgoing.user_channel_id.as_deref(),
            Some("outgoing-user-channel")
        );
        assert_eq!(path.outgoing.node_id.as_deref(), Some("outgoing-peer"));
        assert!(forwarding_path(&event(6, "CHANNEL_RECONSTRUCTED")).is_none());
    }

    #[test]
    fn timeline_keeps_every_event_in_requested_order() {
        let events = vec![
            event(1, "MESSAGE_RECEIVED"),
            event(2, "TRADE_SIGNATURE_VALID"),
            event(3, "STABLE_EDITED"),
        ];
        assert_eq!(timeline_order(&events, false), vec![0, 1, 2]);
        assert_eq!(timeline_order(&events, true), vec![2, 1, 0]);
    }

    #[test]
    fn jsonl_export_is_chronological_complete_and_deduplicated() {
        let mut newest = event(2, "E2");
        newest.detail_json = r#"{"id":2}"#.to_owned();
        newest.before = Some(AccountingSnapshot {
            backing_sats: Some(7),
            ..Default::default()
        });
        let mut oldest = event(1, "E1");
        oldest.detail_json = r#"{"id":1}"#.to_owned();
        let history = ListChannelLedgerEventsResponse {
            events: vec![newest.clone(), oldest, newest],
            next_cursor: None,
            overview: None,
        };
        let jsonl = history_jsonl(&history);
        let lines = jsonl.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"ledger_id\":1"));
        assert!(lines[1].contains("\"ledger_id\":2"));
        assert!(lines[1].contains("\"before\":{\"amount_msat\":null"));
        assert!(lines[1].contains("\"data\":{\"id\":2}"));
    }

    #[test]
    fn sats_values_include_approximate_usd_only_with_recorded_price() {
        assert_eq!(format_sats_with_usd(100_000, None), "100,000 sats");
        assert_eq!(
            format_sats_with_usd(100_000, Some(80_000.0)),
            "100,000 sats · ≈ $80.00"
        );
    }

    fn with_detail(mut event: ChannelLedgerEvent, detail: &str) -> ChannelLedgerEvent {
        event.detail_json = detail.to_owned();
        event
    }

    #[test]
    fn splice_negotiation_rows_read_as_splice_progress() {
        let negotiated = event(1, "SPLICE_NEGOTIATED");
        assert_eq!(human_summary(&negotiated), "Splice negotiated");
        assert!(event_help(&negotiated).contains("waiting for confirmation"));
        let failed = event(2, "SPLICE_NEGOTIATION_FAILED");
        assert_eq!(human_summary(&failed), "Splice negotiation failed");
        assert!(event_help(&failed).contains("Nothing changed on-chain"));
    }

    #[test]
    fn shutdown_stage_rows_name_the_new_stage() {
        let resolving = with_detail(
            event(1, "CHANNEL_SHUTDOWN_STATE_CHANGED"),
            r#"{"previous_shutdown_state":"SHUTDOWN_INITIATED","shutdown_state":"RESOLVING_HTLCS"}"#,
        );
        assert_eq!(human_summary(&resolving), "Channel shutdown: resolving HTLCs");
        assert!(event_help(&resolving).contains("cooperative close"));
        assert!(event_help(&resolving).contains("every 30 seconds"));
        assert_eq!(
            human_summary(&event(2, "CHANNEL_SHUTDOWN_STATE_CHANGED")),
            "Channel shutdown stage changed"
        );
    }

    #[test]
    fn failed_payment_rows_explain_the_ldk_reason() {
        let route = with_detail(event(1, "PAYMENT_FAILED"), r#"{"reason":"ROUTE_NOT_FOUND"}"#);
        assert_eq!(human_summary(&route), "Payment failed: route not found");
        assert!(event_help(&route).contains("wallet was offline"));
        let exhausted = with_detail(event(5, "PAYMENT_FAILED"), r#"{"reason":"RETRIES_EXHAUSTED"}"#);
        assert!(event_help(&exhausted).contains("used up its retry attempts"));
        let rejected = with_detail(event(2, "PAYMENT_FAILED"), r#"{"reason":"RECIPIENT_REJECTED"}"#);
        assert!(event_help(&rejected).contains("rejected"));
        let novel = with_detail(event(3, "PAYMENT_FAILED"), r#"{"reason":"UNKNOWN(42)"}"#);
        assert!(event_help(&novel).contains("unknown(42)"));
        let legacy = with_detail(event(4, "PAYMENT_FAILED"), r#"{"reason":null}"#);
        assert_eq!(human_summary(&legacy), "Payment failed");
        assert_eq!(event_help(&legacy), "The Lightning payment did not complete successfully.");
    }

    #[test]
    fn onchain_channel_transactions_read_as_their_type_and_state() {
        let mut funding = with_detail(
            event(1, "CHANNEL_ONCHAIN_TX"),
            r#"{"tx_type":"FUNDING","confirmation":"confirmed","confirmation_height":861204}"#,
        );
        assert_eq!(human_summary(&funding), "Funding transaction confirmed");
        assert!(event_help(&funding).contains("funds the channel"));
        assert!(event_help(&funding).contains("block 861204"));

        let mut close = with_detail(
            event(2, "CHANNEL_ONCHAIN_TX"),
            r#"{"tx_type":"COOPERATIVE_CLOSE","confirmation":"unconfirmed"}"#,
        );
        close.status = "pending".to_owned();
        assert_eq!(human_summary(&close), "Cooperative close transaction broadcast");
        assert!(event_help(&close).contains("waiting for confirmation"));

        funding.status = "failed".to_owned();
        funding.detail_json = r#"{"tx_type":"FUNDING","confirmation":"unconfirmed"}"#.to_owned();
        assert_eq!(human_summary(&funding), "Funding transaction failed");
        assert!(event_help(&funding).contains("dropped or replaced"));

        close.detail_json = r#"{"tx_type":"SOMETHING_NEW","confirmation":"unconfirmed"}"#.to_owned();
        assert_eq!(human_summary(&close), "On-chain channel transaction broadcast");
    }

    #[test]
    fn forwarded_rows_mention_a_skimmed_channel_open_fee() {
        let jit = with_detail(event(1, "PAYMENT_FORWARDED"), r#"{"skimmed_fee_msat":2500000}"#);
        assert!(event_help(&jit).contains("2,500 sats"));
        assert!(event_help(&jit).contains("channel-open fee"));
        let plain = with_detail(event(2, "PAYMENT_FORWARDED"), r#"{"skimmed_fee_msat":null}"#);
        assert!(!event_help(&plain).contains("channel-open fee"));
        let backfill = with_detail(
            event(3, "PAYMENT_FORWARDED_BACKFILL"),
            r#"{"skimmed_fee_msat":1500}"#,
        );
        assert!(event_help(&backfill).contains("not observed on the live event stream"));
        assert!(event_help(&backfill).contains("1,500 msat"));
    }
}

#[cfg(test)]
mod paging_tests {
    use super::*;
    use crate::state::{ChannelLedgerForm, ChannelLedgerRequestKey};

    fn event(id: i64, occurred_at_ms: i64) -> ChannelLedgerEvent {
        ChannelLedgerEvent {
            id,
            occurred_at_ms,
            ..Default::default()
        }
    }

    #[test]
    fn older_pages_merge_chronologically_without_duplicates() {
        let mut events = vec![event(3, 30), event(4, 40)];
        merge_ledger_events(&mut events, vec![event(2, 20), event(3, 30), event(1, 10)]);
        assert_eq!(
            events.iter().map(|event| event.id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn export_cursor_validation_terminates_and_rejects_cycles() {
        let mut seen = HashSet::new();
        assert_eq!(
            checked_next_cursor("", Some("50".to_owned()), &mut seen).unwrap(),
            Some("50".to_owned())
        );
        assert!(checked_next_cursor("50", Some("50".to_owned()), &mut seen).is_err());
        assert_eq!(checked_next_cursor("50", None, &mut seen).unwrap(), None);

        let mut seen = HashSet::new();
        assert!(checked_next_cursor("10", Some("20".to_owned()), &mut seen).is_ok());
        assert!(checked_next_cursor("30", Some("20".to_owned()), &mut seen).is_err());
    }

    #[test]
    fn the_timeline_never_sends_filters_that_are_hidden() {
        let form = ChannelLedgerForm {
            identifier: " uid ".to_owned(),
            category: "payment".to_owned(),
            status: "failed".to_owned(),
            completeness: "observed".to_owned(),
            newest_first: true,
            show_technical: false,
        };
        let timeline = crate::actions::ledger_request(&form, String::new(), 50);
        assert_eq!((timeline.category.as_str(), timeline.status.as_str(), timeline.completeness.as_str()), ("", "", ""));
        assert!(timeline.include_linked && timeline.state_changes_only);
        assert_eq!(timeline.identifier, "uid");
        let technical = crate::actions::ledger_request(&ChannelLedgerForm { show_technical: true, ..form }, "c".to_owned(), 200);
        assert_eq!(technical.status, "failed");
        assert!(!technical.state_changes_only);
        assert_eq!((technical.cursor.as_str(), technical.page_size), ("c", 200));
    }

    #[test]
    fn ledger_request_identity_covers_identifier_and_server_filters() {
        let form = ChannelLedgerForm {
            identifier: "  stable-channel  ".to_owned(),
            category: "payment".to_owned(),
            status: "failed".to_owned(),
            completeness: "observed".to_owned(),
            newest_first: false,
            show_technical: false,
        };
        let key = ChannelLedgerRequestKey::from(&form);
        let mut technical = form.clone();
        technical.show_technical = true;
        assert_ne!(ChannelLedgerRequestKey::from(&technical), key, "the technical toggle is a different request");
        assert_eq!(key.identifier, "stable-channel");

        let mut changed = form.clone();
        changed.status = "completed".to_owned();
        assert_ne!(key, ChannelLedgerRequestKey::from(&changed));
    }
}
