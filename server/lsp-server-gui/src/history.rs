//! Groups raw ledger events into human-readable channel history entries.

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use sc_rest_client::sc_protos::stable::ChannelLedgerEvent;
use serde_json::Value;

use crate::format::format_usd as usd;
use crate::ledger::{human_summary, humanize_enum};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
	Lifecycle,
	Trade,
	TradeRejected,
	Stability,
	Sync,
	Payment,
	Forward,
	Target,
	Peer,
	Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
	pub key: i64,
	pub kind: EntryKind,
	pub summary: String,
	/// Time of the earliest event in the entry (differs from `occurred_at_ms` for collapsed runs).
	pub started_at_ms: i64,
	pub occurred_at_ms: i64,
	pub amount_msat: Option<u64>,
	pub btc_price: Option<f64>,
	pub target_after: Option<f64>,
	/// True when this entry moved the stable target away from the previous entry's value.
	pub target_changed: bool,
	/// How many identical consecutive occurrences this entry stands for.
	pub repeats: usize,
	/// The flow ended badly: a rejected trade, or a failure with no later success in the same flow.
	pub failed: bool,
	pub events: Vec<ChannelLedgerEvent>,
}

const FAILURE_EVENTS: [&str; 10] = [
	"STABILITY_PAYMENT_FAILED",
	"PAYMENT_FAILED",
	"STABILITY_PAYMENT_ROLLED_BACK",
	"STABILITY_PAYMENT_ROLLBACK_SKIPPED",
	"STABILITY_PAYMENT_FAILED_RECONCILED",
	"SPLICE_NEGOTIATION_FAILED",
	"SPLICE_FAILED",
	"CHANNEL_OPEN_FAILED",
	"SYNC_RETRY_EXHAUSTED",
	"SYNC_RETRY_BLOCKED",
];
const SUCCESS_EVENTS: [&str; 6] = [
	"STABILITY_PAYMENT_SETTLED",
	"STABILITY_PAYMENT_RECORDED",
	"STABILITY_PAYMENT_V1_APPLIED",
	"STABILITY_RECEIVED_RECONCILED",
	"PAYMENT_SETTLED",
	"PAYMENT_SUCCESSFUL",
];

/// Routine housekeeping the owner does not need in a cross-channel feed (the Audit log still shows it).
pub fn is_routine(entry: &HistoryEntry) -> bool {
	// Healthy event-stream bookkeeping with no channel ref; a lossy or partial reconciliation stays visible.
	let channel_less_infrastructure = |e: &ChannelLedgerEvent| {
		let healthy = match e.event_type.as_str() {
			"EVENT_STREAM_GAP_STARTED" | "EVENT_STREAM_GAP_CLOSED" => true,
			"RECONCILIATION_RESULT" => detail(e).get("status").and_then(Value::as_str) == Some("completed"),
			_ => false,
		};
		healthy && !e.refs.iter().any(|r| matches!(r.role.as_str(), "user_channel_id" | "channel_id"))
	};
	match entry.kind {
		EntryKind::Peer => true,
		EntryKind::Sync => !entry.failed,
		EntryKind::Other => entry.summary.starts_with("Wallet above peg") || entry.events.iter().all(channel_less_infrastructure),
		_ => false,
	}
}

/// Why a flow failed, in the ledger's wording: LDK's failure reason, else the daemon's error text.
pub fn failure_detail(events: &[ChannelLedgerEvent]) -> Option<String> {
	events.iter().rev().find_map(|e| {
		let d = detail(e);
		match e.event_type.as_str() {
			"PAYMENT_FAILED" => d.get("reason").and_then(Value::as_str).map(humanize_enum),
			"STABILITY_PAYMENT_FAILED" | "SPLICE_NEGOTIATION_FAILED" | "SPLICE_FAILED" | "CHANNEL_OPEN_FAILED" => d
				.get("reason")
				.or_else(|| d.get("error"))
				.and_then(Value::as_str)
				.map(|r| if r.contains(' ') { r.to_owned() } else { humanize_enum(r) }),
			_ => None,
		}
	})
}

const LINK_ROLES: [&str; 3] = ["payment_id", "trade_id", "settlement_id"];

pub(crate) fn detail(event: &ChannelLedgerEvent) -> Value {
	serde_json::from_str(&event.detail_json).unwrap_or(Value::Null)
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
	while parent[i] != i {
		parent[i] = parent[parent[i]];
		i = parent[i];
	}
	i
}

/// Oldest-first entries: events sharing a payment, trade or settlement id (transitively) form one entry.
pub fn build_entries(events: &[ChannelLedgerEvent], channel: &str) -> Vec<HistoryEntry> {
	let mut sorted: Vec<&ChannelLedgerEvent> = events.iter().collect();
	sorted.sort_by_key(|e| (e.occurred_at_ms, e.id));
	let mut parent: Vec<usize> = (0..sorted.len()).collect();
	let mut owner: HashMap<(&str, &str), usize> = HashMap::new();
	for (i, event) in sorted.iter().enumerate() {
		for r in event.refs.iter().filter(|r| LINK_ROLES.contains(&r.role.as_str())) {
			match owner.get(&(r.role.as_str(), r.value.as_str())) {
				Some(&j) => {
					let (a, b) = (find(&mut parent, i), find(&mut parent, j));
					parent[a] = b;
				},
				None => {
					owner.insert((r.role.as_str(), r.value.as_str()), i);
				},
			}
		}
	}
	let mut groups: Vec<Vec<ChannelLedgerEvent>> = Vec::new();
	let mut slot: HashMap<usize, usize> = HashMap::new();
	for i in 0..sorted.len() {
		let root = find(&mut parent, i);
		let at = *slot.entry(root).or_insert_with(|| {
			groups.push(Vec::new());
			groups.len() - 1
		});
		groups[at].push(sorted[i].clone());
	}
	let mut entries: Vec<HistoryEntry> = groups.into_iter().map(|events| entry(events, channel)).collect();
	entries.sort_by_key(|e| (e.occurred_at_ms, e.key));
	let mut last_peer: Option<String> = None;
	entries.retain(|e| {
		if e.kind != EntryKind::Peer {
			return true;
		}
		let keep = last_peer.as_deref() != Some(e.summary.as_str());
		last_peer = Some(e.summary.clone());
		keep
	});
	let mut story: Vec<HistoryEntry> = Vec::new();
	let mut last_target: Option<f64> = None;
	for mut e in entries {
		// A no-change accounting record adds nothing to the story; keep it inspectable under the previous entry.
		let accounting_only = e.events.iter().all(|x| x.event_type == "CHANNEL_ACCOUNTING_STATE_COMMITTED");
		if accounting_only && e.target_after.is_some() && e.target_after == last_target {
			if let Some(previous) = story.last_mut() {
				previous.events.extend(e.events);
				continue;
			}
		}
		e.target_changed = e.target_after.is_some() && e.target_after != last_target;
		if e.target_after.is_some() {
			last_target = e.target_after;
		}
		// Runs of the same wake-up collapse into one counted entry.
		if let Some(previous) = story.last_mut() {
			if e.kind == EntryKind::Other && previous.kind == EntryKind::Other && previous.summary == e.summary && !e.target_changed {
				previous.repeats += 1;
				previous.occurred_at_ms = e.occurred_at_ms;
				previous.events.extend(e.events);
				continue;
			}
		}
		story.push(e);
	}
	story
}

fn has(events: &[ChannelLedgerEvent], names: &[&str]) -> bool {
	events.iter().any(|e| names.contains(&e.event_type.as_str()))
}

/// What an operator should know about a top-up that raised an alarm, or None once its own flow shows the alarm no longer applies.
pub fn top_up_alarm(events: &[ChannelLedgerEvent]) -> Option<&'static str> {
	let settled = has(events, &["STABILITY_PAYMENT_SETTLED"]);
	let over = settled
		|| has(events, &["STABILITY_PAYMENT_FAILED", "PAYMENT_FAILED", "STABILITY_PAYMENT_ROLLED_BACK", "STABILITY_PAYMENT_ROLLBACK_SKIPPED", "STABILITY_TOP_UP_RELEASED"]);
	if has(events, &["STABILITY_TOP_UP_RELEASE_CONFLICT"]) {
		// The node contradicted the operator's release; booking it does not take back what was paid in its place.
		Some("it was claimed after an operator counted it as not arrived: the user may have been paid twice")
	} else if has(events, &["STABILITY_TOP_UP_FAILED_AFTER_BOOKING"]) {
		Some("the node reports it failed after it was booked: verify this channel's books")
	} else if has(events, &["STABILITY_TOP_UP_LOOKUP_FAILED"]) {
		// Booking it later does not undo an event handled without knowing its outcome.
		Some("its outcome could not be checked before another event was handled: verify this channel's books")
	} else if !settled && has(events, &["STABILITY_TOP_UP_BOOKING_FAILED"]) {
		Some("it was claimed but not booked: the channel record was not found")
	} else if !over && has(events, &["STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN"]) {
		Some("the node lost its record of it: nothing is resent until an operator releases it")
	} else if !over && has(events, &["STABILITY_TOP_UP_DEFERRED_STILL_PENDING"]) {
		Some("it has had no outcome for over an hour: waiting, nothing resent")
	} else {
		None
	}
}

/// Whether the top-up's alarm is one that waiting will not clear: the node has no record of it, it was claimed and could not be booked, or the node contradicted what was booked or released.
pub fn top_up_alarm_is_urgent(events: &[ChannelLedgerEvent]) -> bool {
	let settled = has(events, &["STABILITY_PAYMENT_SETTLED"]);
	top_up_to_release(events).is_some()
		|| (!settled && has(events, &["STABILITY_TOP_UP_BOOKING_FAILED"]))
		|| has(events, &["STABILITY_TOP_UP_RELEASE_CONFLICT", "STABILITY_TOP_UP_FAILED_AFTER_BOOKING"])
}

/// The payment id of a top-up the node has no record of, for an operator to release; None once it has an outcome.
pub fn top_up_to_release(events: &[ChannelLedgerEvent]) -> Option<String> {
	let over = has(events, &[
		"STABILITY_PAYMENT_SETTLED",
		"STABILITY_PAYMENT_FAILED",
		"PAYMENT_FAILED",
		"STABILITY_PAYMENT_ROLLED_BACK",
		"STABILITY_PAYMENT_ROLLBACK_SKIPPED",
		"STABILITY_TOP_UP_RELEASED",
	]);
	if over || !has(events, &["STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN"]) {
		return None;
	}
	events.iter().flat_map(|e| e.refs.iter()).find(|r| r.role == "payment_id").map(|r| r.value.clone())
}

const NOT_ARRIVED_WORDS: &str = "an operator counted it as not arrived";

/// How an operator released the payment, if one did.
fn release_words(events: &[ChannelLedgerEvent]) -> Option<&'static str> {
	let released = events.iter().find(|e| e.event_type == "STABILITY_TOP_UP_RELEASED")?;
	Some(match detail(released).get("decision").and_then(Value::as_str) {
		Some("arrived") => "an operator counted it as arrived",
		_ => NOT_ARRIVED_WORDS,
	})
}

fn first_u64(events: &[ChannelLedgerEvent], keys: &[&str]) -> Option<u64> {
	events.iter().find_map(|e| {
		let d = detail(e);
		keys.iter().find_map(|k| d.get(*k).and_then(Value::as_u64))
	})
}

fn amount_msat(events: &[ChannelLedgerEvent]) -> Option<u64> {
	first_u64(events, &["amount_msat", "forwarded_msat"])
		.or_else(|| events.iter().find_map(|e| e.after.as_ref().and_then(|s| s.amount_msat)))
}

fn fee_msat(events: &[ChannelLedgerEvent]) -> Option<u64> {
	first_u64(events, &["fee_msat", "total_fee_msat"])
		.or_else(|| events.iter().find_map(|e| e.after.as_ref().and_then(|s| s.fee_msat)))
}

fn last_f64(events: &[ChannelLedgerEvent], keys: &[&str]) -> Option<f64> {
	events.iter().rev().find_map(|e| {
		let d = detail(e);
		keys.iter().find_map(|k| d.get(*k).and_then(Value::as_f64))
	})
}

fn first_str(events: &[ChannelLedgerEvent], key: &str) -> Option<String> {
	events.iter().find_map(|e| detail(e).get(key).and_then(Value::as_str).map(str::to_owned))
}

fn sats(msat: u64) -> String {
	if msat < 1_000 {
		return format!("{msat} msat");
	}
	format!("{} sats", crate::format::format_sats(msat / 1_000))
}

// " (route not found)" when LDK said why a payment in the flow failed.
fn failure_reason(events: &[ChannelLedgerEvent]) -> String {
	failure_detail(events).map(|r| format!(" ({r})")).unwrap_or_default()
}

fn reason_words(code: &str) -> String {
	match code {
		"insufficient_capacity" => "not enough channel capacity".to_owned(),
		"unsafe_allocation" => "the amount would leave the channel unsafe".to_owned(),
		"client_cancelled" => "cancelled by the wallet".to_owned(),
		"invalid_confirmation" => "the confirmation was invalid".to_owned(),
		"stale_request" => "the request expired".to_owned(),
		other => other.replace('_', " "),
	}
}

fn entry(events: Vec<ChannelLedgerEvent>, channel: &str) -> HistoryEntry {
	let key = events[0].id;
	let started_at_ms = events.iter().map(|e| e.occurred_at_ms).min().unwrap_or_default();
	let occurred_at_ms = events.iter().map(|e| e.occurred_at_ms).max().unwrap_or_default();
	let amount_msat = amount_msat(&events);
	let btc_price = last_f64(&events, &["btc_price", "lsp_price", "latest_price"])
		.or_else(|| events.iter().rev().find_map(|e| e.after.as_ref().and_then(|s| s.btc_price)));
	let target_after = events
		.iter()
		.rev()
		.find_map(|e| e.after.as_ref().and_then(|s| s.expected_usd))
		.or_else(|| last_f64(&events, &["new_expected_usd", "expected_usd"]));
	let (kind, summary) = describe(&events, channel, amount_msat, target_after);
	let failed = kind == EntryKind::TradeRejected || (has(&events, &FAILURE_EVENTS) && !has(&events, &SUCCESS_EVENTS));
	HistoryEntry { key, kind, summary, started_at_ms, occurred_at_ms, amount_msat, btc_price, target_after, target_changed: false, repeats: 1, failed, events }
}

fn describe(events: &[ChannelLedgerEvent], channel: &str, amount: Option<u64>, target: Option<f64>) -> (EntryKind, String) {
	let amt = amount.map(sats).unwrap_or_default();
	if has(events, &["TRADE_REJECTION_QUEUED", "TRADE_REJECTED_BY_LSP", "TRADE_FAILED"]) {
		let reason = first_str(events, "reason_code")
			.map(|code| reason_words(&code))
			.unwrap_or_else(|| "unknown reason".to_owned());
		return (EntryKind::TradeRejected, format!("Trade rejected: {reason}"));
	}
	if has(events, &["TRADE_APPLIED", "TRADE_ACCEPTED", "TRADE_RESERVED", "TRADE_MESSAGE_SENT"]) {
		let text = match target {
			Some(t) => format!("Trade applied: stable target {}", usd(t)),
			None => "Trade applied".to_owned(),
		};
		return (EntryKind::Trade, text);
	}
	if events.iter().any(|e| e.event_type.starts_with("STABILITY_PAYMENT") || e.event_type == "STABILITY_RECEIVED_RECONCILED") {
		let settled = has(events, &[
			"STABILITY_PAYMENT_SETTLED",
			"STABILITY_PAYMENT_RECORDED",
			"STABILITY_PAYMENT_V1_APPLIED",
			"STABILITY_RECEIVED_RECONCILED",
		]);
		let failed = has(events, &[
			"STABILITY_PAYMENT_FAILED",
			"PAYMENT_FAILED",
			"STABILITY_PAYMENT_ROLLED_BACK",
			"STABILITY_PAYMENT_ROLLBACK_SKIPPED",
			"STABILITY_PAYMENT_FAILED_RECONCILED",
		]);
		let to_user = first_str(events, "direction").is_some_and(|d| d == "lsp_to_user" || d == "outbound");
		let why = failure_reason(events);
		// A retried settlement shares its settlement id, so a later success outranks an earlier failure.
		let text = match (to_user, settled, failed) {
			// An operator's "not arrived" ends the flow without the node ever reporting a failure.
			(true, false, false) if release_words(events) == Some(NOT_ARRIVED_WORDS) => format!("LSP's payment of {amt} to keep the peg was dropped"),
			(true, true, _) => format!("LSP paid {amt} to keep the peg"),
			(true, false, true) => format!("LSP's payment of {amt} to keep the peg failed{why}"),
			(true, false, false) => format!("LSP sent {amt} to keep the peg (not yet confirmed)"),
			(false, true, _) => format!("User paid {amt} to the LSP to keep the peg"),
			(false, false, true) => format!("User's payment of {amt} to keep the peg failed{why}"),
			(false, false, false) => format!("User sent {amt} to the LSP to keep the peg (not yet confirmed)"),
		};
		// An alarm shares the payment's id and so lands in this entry; it must not vanish behind the payment line.
		let text = match top_up_alarm(events) {
			Some(alarm) => format!("{text}; {alarm}"),
			None => text,
		};
		let text = match release_words(events) {
			Some(release) => format!("{text}; {release}"),
			None => text,
		};
		return (EntryKind::Stability, text);
	}
	if has(events, &["SYNC_MESSAGE_SENT"]) {
		// SYNC_MESSAGE_SENT is written when the keysend starts; only a failure row changes the story.
		let failed = has(events, &["PAYMENT_FAILED"]) && !has(events, &["PAYMENT_SETTLED", "PAYMENT_SUCCESSFUL"]);
		let why = failure_reason(events);
		let text = match (failed, last_f64(events, &["expected_usd"])) {
			(false, Some(t)) => format!("LSP published balances: target {}", usd(t)),
			(false, None) => "LSP published balances".to_owned(),
			(true, Some(t)) => format!("Balance sync to the wallet failed{why}: target {}", usd(t)),
			(true, None) => format!("Balance sync to the wallet failed{why}"),
		};
		return (EntryKind::Sync, text);
	}
	if has(events, &["PAYMENT_FORWARDED", "PAYMENT_FORWARDED_BACKFILL"]) {
		let fee = fee_msat(events).map(|f| format!(" (fee {})", sats(f))).unwrap_or_default();
		let way = if first_str(events, "next_user_channel_id").as_deref() == Some(channel) {
			"out through this channel"
		} else {
			"in through this channel"
		};
		return (EntryKind::Forward, format!("Forwarded {amt} {way}{fee}"));
	}
	if events.iter().any(|e| e.event_type.starts_with("PAYMENT_")) {
		let text = match first_str(events, "direction").as_deref() {
			Some("inbound") => format!("Received {amt}"),
			Some("outbound") => format!("Sent {amt}"),
			_ => format!("Payment {amt}"),
		};
		let failed = has(events, &["PAYMENT_FAILED"]) && !has(events, &["PAYMENT_SETTLED", "PAYMENT_SUCCESSFUL"]);
		return (EntryKind::Payment, if failed { format!("{text} failed{}", failure_reason(events)) } else { text });
	}
	let only = &events[events.len() - 1];
	match only.event_type.as_str() {
		"PEER_CONNECTED" | "PEER_RECONSTRUCTED" => (EntryKind::Peer, "Wallet online".to_owned()),
		"PEER_DISCONNECTED" => (EntryKind::Peer, "Wallet offline".to_owned()),
		"STABILITY_PUSH_QUEUED" => (EntryKind::Other, "Wallet above peg: push sent to wake it".to_owned()),
		"STABILITY_CHECK_ONLY" => (EntryKind::Other, "Wallet above peg: waiting for it to pay".to_owned()),
		"STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN" => (EntryKind::Other, "Node lost its record of a stability payment: nothing is resent until an operator releases it".to_owned()),
		"STABILITY_TOP_UP_RELEASED" => (EntryKind::Other, "Operator released a stability payment".to_owned()),
		"STABILITY_TOP_UP_RELEASE_CONFLICT" => (EntryKind::Other, "Released stability payment was claimed after all: the user may have been paid twice".to_owned()),
		"STABILITY_TOP_UP_FAILED_AFTER_BOOKING" => (EntryKind::Other, "Booked stability payment reported failed: verify this channel's books".to_owned()),
		"STABILITY_TOP_UP_DEFERRED_STILL_PENDING" => (EntryKind::Other, "Stability payment without an outcome for over an hour: waiting, nothing resent".to_owned()),
		"STABILITY_TOP_UP_BOOKING_FAILED" => (EntryKind::Other, "Claimed stability payment not booked: channel record not found".to_owned()),
		"STABILITY_TOP_UP_LOOKUP_FAILED" => (EntryKind::Other, "Stability payment outcome could not be checked: verify this channel's books".to_owned()),
		"SYNC_RETRY_EXHAUSTED" => (EntryKind::Sync, "Stopped retrying the balance sync".to_owned()),
		"SYNC_RETRY_BLOCKED" => (EntryKind::Sync, "Balance sync blocked: the channel cannot carry it".to_owned()),
		"CHANNEL_ACCOUNTING_STATE_COMMITTED"
		| "STABLE_EDITED"
		| "BACKSTOP_STABLE_DEDUCTED"
		| "OUTGOING_STABLE_DEDUCTED"
		| "STABLE_SPEND_DEDUCTED"
		| "SPLICE_OUT_STABLE_DEDUCTED" => {
			let text = match (only.event_type.as_str(), target.map(usd)) {
				("CHANNEL_ACCOUNTING_STATE_COMMITTED", Some(t)) => format!("Stable target set to {t}"),
				("STABLE_EDITED", Some(t)) => format!("Operator set the stable target to {t}"),
				("BACKSTOP_STABLE_DEDUCTED", Some(t)) => format!("Balance fell below the stable target: target lowered to {t}"),
				("SPLICE_OUT_STABLE_DEDUCTED", Some(t)) => format!("Splice out lowered the stable target to {t}"),
				(_, Some(t)) => format!("Outgoing payment lowered the stable target to {t}"),
				(_, None) => human_summary(only),
			};
			(EntryKind::Target, text)
		},
		t if t.starts_with("CHANNEL_") || t.starts_with("SPLICE") || t.starts_with("SWEEP") => {
			(EntryKind::Lifecycle, human_summary(only))
		},
		_ => (EntryKind::Other, human_summary(only)),
	}
}

/// Calendar day in UTC, used as the timeline's section heading.
pub fn day_label(timestamp_ms: i64) -> String {
	Utc.timestamp_millis_opt(timestamp_ms)
		.single()
		.map(|t| t.format("%-d %b %Y").to_string())
		.unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use super::*;
	use sc_rest_client::sc_protos::stable::{ChannelLedgerEvent, LedgerRef};

	fn ev(id: i64, event_type: &str, detail: serde_json::Value, refs: &[(&str, &str)]) -> ChannelLedgerEvent {
		ChannelLedgerEvent {
			id,
			event_type: event_type.to_owned(),
			occurred_at_ms: 1_790_000_000_000 + id * 1_000,
			detail_json: detail.to_string(),
			refs: refs.iter().map(|(r, v)| LedgerRef { role: (*r).into(), value: (*v).into() }).collect(),
			..Default::default()
		}
	}

	fn events_start(id: i64) -> i64 {
		1_790_000_000_000 + id * 1_000
	}

	#[test]
	fn a_stability_settlement_is_one_entry_across_its_ids() {
		let events = vec![
			ev(1, "STABILITY_PAYMENT_V1_SENT", serde_json::json!({"direction": "lsp_to_user", "amount_msat": 520_000}), &[("payment_id", "p"), ("settlement_id", "s")]),
			ev(2, "STABILITY_PAYMENT_SENT", serde_json::json!({"amount_msat": 520_000}), &[("payment_id", "p")]),
			ev(3, "STABILITY_PAYMENT_SETTLED", serde_json::json!({"direction": "outbound", "amount_msat": 520_000}), &[("payment_id", "p")]),
			ev(4, "STABILITY_PAYMENT_RECORDED", serde_json::json!({"btc_price": 80_000.0}), &[("settlement_id", "s")]),
		];
		let entries = build_entries(&events, "uid");
		assert_eq!(entries.len(), 1);
		assert_eq!(entries[0].kind, EntryKind::Stability);
		assert!(entries[0].summary.starts_with("LSP paid"), "{}", entries[0].summary);
		assert_eq!(entries[0].events.len(), 4);
	}

	#[test]
	fn trades_read_as_target_changes_or_plain_rejections() {
		let applied = build_entries(&[
			ev(1, "TRADE_ACCEPTED", serde_json::json!({}), &[("trade_id", "t")]),
			ev(2, "TRADE_APPLIED", serde_json::json!({"new_expected_usd": 55.0}), &[("trade_id", "t")]),
		], "uid");
		assert_eq!(applied.len(), 1);
		assert_eq!(applied[0].kind, EntryKind::Trade);
		assert_eq!(applied[0].target_after, Some(55.0));
		let rejected = build_entries(&[ev(1, "TRADE_REJECTION_QUEUED", serde_json::json!({"reason_code": "insufficient_capacity"}), &[("trade_id", "t")])], "uid");
		assert_eq!(rejected[0].kind, EntryKind::TradeRejected);
		assert_eq!(rejected[0].summary, "Trade rejected: not enough channel capacity");
	}

	#[test]
	fn channel_less_stream_bookkeeping_is_routine() {
		let routine = |event, status: &str| is_routine(&build_entries(&[ev(1, event, serde_json::json!({"status": status}), &[])], "")[0]);
		assert!(routine("EVENT_STREAM_GAP_STARTED", ""));
		assert!(routine("EVENT_STREAM_GAP_CLOSED", ""));
		assert!(routine("RECONCILIATION_RESULT", "completed"));
		for status in ["partial", "failed", "completed_with_loss"] {
			assert!(!routine("RECONCILIATION_RESULT", status), "{status}");
		}
		assert!(!routine("RECONCILIATION_GAP_DETECTED", "partial"), "a detected gap is never routine");
		let attached = build_entries(&[ev(1, "EVENT_STREAM_GAP_CLOSED", serde_json::json!({}), &[("user_channel_id", "7")])], "7");
		assert!(!is_routine(&attached[0]), "a channel's own recovery stays in the feed");
		let other = build_entries(&[ev(1, "SOMETHING_NEW_HAPPENED", serde_json::json!({}), &[])], "");
		assert!(!is_routine(&other[0]), "unknown events are never hidden");
	}

	#[test]
	fn repeated_peer_states_collapse() {
		let entries = build_entries(&[
			ev(1, "PEER_CONNECTED", serde_json::json!({}), &[]),
			ev(2, "PEER_CONNECTED", serde_json::json!({}), &[]),
			ev(3, "PEER_DISCONNECTED", serde_json::json!({}), &[]),
			ev(4, "PEER_DISCONNECTED", serde_json::json!({}), &[]),
			ev(5, "PEER_CONNECTED", serde_json::json!({}), &[]),
		], "uid");
		let summaries: Vec<_> = entries.iter().map(|e| e.summary.as_str()).collect();
		assert_eq!(summaries, ["Wallet online", "Wallet offline", "Wallet online"]);
	}

	#[test]
	fn a_sync_folds_in_its_keysend_and_unknown_types_survive() {
		let entries = build_entries(&[
			ev(1, "SYNC_MESSAGE_SENT", serde_json::json!({"expected_usd": 43.63}), &[("payment_id", "k")]),
			ev(2, "PAYMENT_SETTLED", serde_json::json!({"amount_msat": 1, "direction": "outbound"}), &[("payment_id", "k")]),
			ev(3, "SOMETHING_NEW_HAPPENED", serde_json::json!({}), &[]),
		], "uid");
		assert_eq!(entries.len(), 2);
		assert_eq!(entries[0].summary, "LSP published balances: target $43.63");
		assert_eq!(entries[1].summary, "Something new happened");
	}

	#[test]
	fn the_target_shows_only_when_it_changes() {
		let entries = build_entries(&[
			ev(1, "SYNC_MESSAGE_SENT", serde_json::json!({"expected_usd": 40.0}), &[("payment_id", "a")]),
			ev(2, "STABILITY_PUSH_QUEUED", serde_json::json!({"expected_usd": 40.0}), &[]),
			ev(3, "BACKSTOP_STABLE_DEDUCTED", serde_json::json!({"new_expected_usd": 35.0}), &[]),
		], "uid");
		let changed: Vec<bool> = entries.iter().map(|e| e.target_changed).collect();
		assert_eq!(changed, [true, false, true]);
		assert_eq!(entries[2].summary, "Balance fell below the stable target: target lowered to $35.00");
	}

	#[test]
	fn repeated_wake_ups_collapse_into_one_counted_entry() {
		let entries = build_entries(&[
			ev(1, "STABILITY_PUSH_QUEUED", serde_json::json!({}), &[]),
			ev(2, "STABILITY_PUSH_QUEUED", serde_json::json!({}), &[]),
			ev(3, "STABILITY_PUSH_QUEUED", serde_json::json!({}), &[]),
			ev(4, "STABILITY_CHECK_ONLY", serde_json::json!({}), &[]),
		], "uid");
		assert_eq!(entries.len(), 2);
		assert_eq!(entries[0].repeats, 3);
		assert_eq!(entries[0].events.len(), 3);
		assert_eq!(entries[0].started_at_ms, events_start(1), "a collapsed run remembers when it began");
		assert_eq!(entries[0].occurred_at_ms, events_start(3));
		assert_eq!(entries[1].repeats, 1);
	}

	#[test]
	fn an_unchanged_accounting_record_folds_into_the_entry_before_it() {
		let entries = build_entries(&[
			ev(1, "TRADE_APPLIED", serde_json::json!({"new_expected_usd": 50.0}), &[("trade_id", "t")]),
			ev(2, "CHANNEL_ACCOUNTING_STATE_COMMITTED", serde_json::json!({"expected_usd": 50.0}), &[]),
			ev(3, "CHANNEL_ACCOUNTING_STATE_COMMITTED", serde_json::json!({"expected_usd": 45.0}), &[]),
		], "uid");
		assert_eq!(entries.len(), 2);
		assert_eq!(entries[0].events.len(), 2, "the no-change record stays inspectable under the trade");
		assert_eq!(entries[1].summary, "Stable target set to $45.00");
	}

	#[test]
	fn sub_sat_amounts_are_shown_in_msat() {
		let entries = build_entries(&[ev(1, "PAYMENT_SETTLED", serde_json::json!({"amount_msat": 1, "direction": "outbound"}), &[])], "uid");
		assert_eq!(entries[0].summary, "Sent 1 msat");
	}

	#[test]
	fn top_up_alarms_stay_visible_inside_the_payment_they_belong_to() {
		let flow = |alarm: &str, settled: bool| {
			let mut events = vec![
				ev(1, "STABILITY_PAYMENT_SENT", serde_json::json!({"direction": "lsp_to_user", "amount_msat": 520_000}), &[("payment_id", "p")]),
				ev(2, alarm, serde_json::json!({}), &[("payment_id", "p")]),
			];
			if settled {
				events.push(ev(3, "STABILITY_PAYMENT_SETTLED", serde_json::json!({"direction": "outbound", "amount_msat": 520_000}), &[("payment_id", "p")]));
			}
			let entries = build_entries(&events, "uid");
			assert_eq!(entries.len(), 1, "{alarm}");
			entries[0].summary.clone()
		};
		for (alarm, text) in [
			("STABILITY_TOP_UP_DEFERRED_STILL_PENDING", "it has had no outcome for over an hour: waiting, nothing resent"),
			("STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN", "the node lost its record of it: nothing is resent until an operator releases it"),
			("STABILITY_TOP_UP_BOOKING_FAILED", "it was claimed but not booked: the channel record was not found"),
			("STABILITY_TOP_UP_LOOKUP_FAILED", "verify this channel's books"),
		] {
			assert!(flow(alarm, false).ends_with(text), "{alarm}: {}", flow(alarm, false));
		}
		// A settled payment clears the waiting and booking alarms, but not an event handled without knowing its outcome.
		assert_eq!(flow("STABILITY_TOP_UP_DEFERRED_STILL_PENDING", true), "LSP paid 520 sats to keep the peg");
		assert_eq!(flow("STABILITY_TOP_UP_BOOKING_FAILED", true), "LSP paid 520 sats to keep the peg");
		assert!(flow("STABILITY_TOP_UP_LOOKUP_FAILED", true).ends_with("verify this channel's books"));
		// A top-up booked the old way whose failure was recovered from the node has only its rollback row to end the flow.
		for rollback in ["STABILITY_PAYMENT_ROLLED_BACK", "STABILITY_PAYMENT_ROLLBACK_SKIPPED"] {
			let entries = build_entries(&[
				ev(1, "STABILITY_PAYMENT_SENT", serde_json::json!({"direction": "lsp_to_user", "amount_msat": 520_000}), &[("payment_id", "p")]),
				ev(2, "STABILITY_TOP_UP_DEFERRED_STILL_PENDING", serde_json::json!({}), &[("payment_id", "p")]),
				ev(3, rollback, serde_json::json!({}), &[("payment_id", "p")]),
			], "uid");
			assert_eq!(entries[0].summary, "LSP's payment of 520 sats to keep the peg failed", "{rollback}");
			assert!(entries[0].failed, "{rollback}");
		}
		// Without its payment row loaded, the alarm still reads as an alarm.
		let alone = build_entries(&[ev(1, "STABILITY_TOP_UP_BOOKING_FAILED", serde_json::json!({}), &[("payment_id", "p")])], "uid");
		assert_eq!(alone[0].summary, "Claimed stability payment not booked: channel record not found");
	}

	#[test]
	fn a_top_up_waiting_will_not_settle_is_offered_for_release_until_it_has_an_outcome() {
		let sent = ev(1, "STABILITY_PAYMENT_SENT", serde_json::json!({"direction": "lsp_to_user", "amount_msat": 520_000}), &[("payment_id", "p")]);
		let alarm = |id, name: &str| ev(id, name, serde_json::json!({}), &[("payment_id", "p")]);
		assert_eq!(top_up_to_release(&[sent.clone(), alarm(2, "STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN")]).as_deref(), Some("p"));
		// A claimed payment that could not be booked is stuck too, but repairing its channel record settles it, not a release.
		let unbooked = [sent.clone(), alarm(2, "STABILITY_TOP_UP_BOOKING_FAILED")];
		assert!(top_up_alarm_is_urgent(&unbooked) && top_up_to_release(&unbooked).is_none());
		assert!(!top_up_alarm_is_urgent(&[sent.clone(), alarm(2, "STABILITY_TOP_UP_BOOKING_FAILED"), alarm(3, "STABILITY_PAYMENT_SETTLED")]), "booked after all");
		// A payment that is merely unclaimed is the node's to settle.
		let unclaimed = [sent.clone(), alarm(2, "STABILITY_TOP_UP_DEFERRED_STILL_PENDING")];
		assert!(!top_up_alarm_is_urgent(&unclaimed) && top_up_to_release(&unclaimed).is_none());
		// An outcome recovered the old way ends the offer as well.
		for ended in ["STABILITY_PAYMENT_SETTLED", "STABILITY_PAYMENT_FAILED", "STABILITY_PAYMENT_ROLLED_BACK", "STABILITY_PAYMENT_ROLLBACK_SKIPPED", "PAYMENT_FAILED"] {
			assert_eq!(top_up_to_release(&[sent.clone(), alarm(2, "STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN"), alarm(3, ended)]), None, "{ended}");
		}

		let released = |decision: &str| {
			vec![
				sent.clone(),
				alarm(2, "STABILITY_TOP_UP_DEFERRED_OUTCOME_UNKNOWN"),
				ev(3, "STABILITY_TOP_UP_RELEASED", serde_json::json!({"decision": decision}), &[("payment_id", "p")]),
			]
		};
		let settled = ev(4, "STABILITY_PAYMENT_SETTLED", serde_json::json!({"direction": "outbound", "amount_msat": 520_000}), &[("payment_id", "p")]);

		let mut arrived = released("arrived");
		arrived.push(settled.clone());
		assert_eq!(top_up_to_release(&arrived), None);
		assert_eq!(build_entries(&arrived, "uid")[0].summary, "LSP paid 520 sats to keep the peg; an operator counted it as arrived");

		// "Not arrived" ends the flow by itself: the node never reports a failure for it.
		let mut dropped = released("not_arrived");
		assert_eq!(top_up_to_release(&dropped), None);
		assert!(!top_up_alarm_is_urgent(&dropped));
		let entries = build_entries(&dropped, "uid");
		assert_eq!(entries.len(), 1);
		assert_eq!(entries[0].summary, "LSP's payment of 520 sats to keep the peg was dropped; an operator counted it as not arrived");
		assert!(!entries[0].failed);

		// The node then reports it claimed after all: that stays on the payment for good.
		dropped.push(settled);
		dropped.push(alarm(5, "STABILITY_TOP_UP_RELEASE_CONFLICT"));
		assert!(top_up_alarm_is_urgent(&dropped));
		assert!(build_entries(&dropped, "uid")[0].summary.ends_with("the user may have been paid twice; an operator counted it as not arrived"));
		let failed_after = [arrived[0].clone(), arrived[3].clone(), alarm(5, "STABILITY_TOP_UP_FAILED_AFTER_BOOKING")];
		assert!(top_up_alarm_is_urgent(&failed_after));
		assert_eq!(top_up_alarm(&failed_after), Some("the node reports it failed after it was booked: verify this channel's books"));
	}

	#[test]
	fn failed_or_unconfirmed_flows_never_read_as_success() {
		let sent = |id, t: &str| ev(id, t, serde_json::json!({"direction": "lsp_to_user", "amount_msat": 520_000}), &[("payment_id", "p")]);
		let failed = build_entries(&[
			sent(1, "STABILITY_PAYMENT_V1_SENT"),
			sent(2, "STABILITY_PAYMENT_SENT"),
			ev(3, "PAYMENT_FAILED", serde_json::json!({}), &[("payment_id", "p")]),
			ev(4, "STABILITY_PAYMENT_ROLLED_BACK", serde_json::json!({}), &[("payment_id", "p")]),
		], "uid");
		assert_eq!(failed[0].summary, "LSP's payment of 520 sats to keep the peg failed");
		let unconfirmed = build_entries(&[sent(1, "STABILITY_PAYMENT_V1_SENT"), sent(2, "STABILITY_PAYMENT_SENT")], "uid");
		assert_eq!(unconfirmed[0].summary, "LSP sent 520 sats to keep the peg (not yet confirmed)");
		let sync = build_entries(&[
			ev(1, "SYNC_MESSAGE_SENT", serde_json::json!({"expected_usd": 43.63}), &[("payment_id", "k")]),
			ev(2, "PAYMENT_FAILED", serde_json::json!({}), &[("payment_id", "k")]),
		], "uid");
		assert_eq!(sync[0].summary, "Balance sync to the wallet failed: target $43.63");
	}

	#[test]
	fn failures_name_the_ldk_reason() {
		let failed = |id, key: &str| ev(id, "PAYMENT_FAILED", serde_json::json!({"reason": "ROUTE_NOT_FOUND"}), &[("payment_id", key)]);
		let stability = build_entries(&[
			ev(1, "STABILITY_PAYMENT_SENT", serde_json::json!({"direction": "lsp_to_user", "amount_msat": 520_000}), &[("payment_id", "p")]),
			failed(2, "p"),
		], "uid");
		assert_eq!(stability[0].summary, "LSP's payment of 520 sats to keep the peg failed (route not found)");
		let sync = build_entries(&[
			ev(1, "SYNC_MESSAGE_SENT", serde_json::json!({"expected_usd": 43.63}), &[("payment_id", "k")]),
			failed(2, "k"),
		], "uid");
		assert_eq!(sync[0].summary, "Balance sync to the wallet failed (route not found): target $43.63");
		let sent = build_entries(&[
			ev(1, "PAYMENT_SUCCESSFUL", serde_json::json!({"direction": "outbound", "amount_msat": 2_000}), &[("payment_id", "s")]),
			ev(2, "PAYMENT_FAILED", serde_json::json!({"direction": "outbound", "reason": "RETRIES_EXHAUSTED"}), &[("payment_id", "t")]),
		], "uid");
		assert!(sent.iter().any(|e| e.summary == "Sent 2 sats"));
		assert!(sent.iter().any(|e| e.summary.ends_with("failed (retries exhausted)")));
	}

	#[test]
	fn a_backfilled_forward_reports_its_real_fee() {
		let mut forward = ev(1, "PAYMENT_FORWARDED_BACKFILL", serde_json::json!({"total_fee_msat": 1_000, "next_user_channel_id": "uid"}), &[]);
		forward.after = Some(sc_rest_client::sc_protos::stable::AccountingSnapshot {
			amount_msat: Some(5_000_000),
			fee_msat: Some(1_000),
			..Default::default()
		});
		let entries = build_entries(&[forward], "uid");
		assert!(entries[0].summary.ends_with("(fee 1 sats)"), "{}", entries[0].summary);
	}

	#[test]
	fn forwards_say_which_way_they_used_this_channel() {
		let out = build_entries(&[ev(1, "PAYMENT_FORWARDED", serde_json::json!({"forwarded_msat": 5_000_000, "fee_msat": 1_000,
			"prev_user_channel_id": "other", "next_user_channel_id": "uid"}), &[])], "uid");
		assert_eq!(out[0].kind, EntryKind::Forward);
		assert!(out[0].summary.contains("out through this channel"), "{}", out[0].summary);
	}
}
