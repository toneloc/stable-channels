//! Pure rules behind the owner dashboard: business tiles, work in progress, feed-derived alerts.

use std::collections::{HashMap, HashSet};

use sc_rest_client::ldk_server_grpc::api::{GetBalancesResponse, ListChannelsResponse, ListPeersResponse};
use sc_rest_client::ldk_server_grpc::types::{lightning_balance, pending_sweep_balance, Channel};
use sc_rest_client::sc_protos::stable::{ChannelLedgerEvent, ListStableChannelsResponse};
use serde_json::Value;

use crate::format::truncate_id;
use crate::health::{has_stable_position, settlement, stable_drift, Attention, DriftLevel, Settlement, Severity, Target};
use crate::history::{self, detail, EntryKind, HistoryEntry};
use crate::ledger::humanize_enum;

pub const WEEK_SECS: u64 = 7 * 86_400;
pub const DAY_MS: i64 = 86_400_000;
pub const WEEK_MS: i64 = 7 * DAY_MS;
/// A cooperative close sitting on one stage this long is flagged.
pub const STUCK_CLOSE_SECS: i64 = 3_600;
/// Fewer JIT channels' worth of on-chain funds than this is "running low".
pub const LOW_ROOM_CHANNELS: u64 = 3;
const BLOCK_SECS: u64 = 600;

// ---------- Business tiles ----------

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StableUsers {
	pub count: usize,
	/// Unknown until the peer list is loaded.
	pub online: Option<usize>,
	pub new_this_week: usize,
}

/// Channels with a USD target, how many of their wallets are connected, and how many the daemon started tracking this week.
pub fn stable_users(stable: &ListStableChannelsResponse, peers: Option<&ListPeersResponse>, now: u64) -> StableUsers {
	let users: Vec<_> = stable.channels.iter().filter(|sc| has_stable_position(sc)).collect();
	let online = peers.map(|p| {
		let connected: HashSet<&str> = p.peers.iter().filter(|p| p.is_connected).map(|p| p.node_id.as_str()).collect();
		users.iter().filter(|sc| connected.contains(sc.counterparty.as_str())).count()
	});
	let new_this_week =
		users.iter().filter(|sc| sc.created_at > 0 && now.saturating_sub(sc.created_at.max(0) as u64) < WEEK_SECS).count();
	StableUsers { count: users.len(), online, new_this_week }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Peg {
	AtPar,
	SettlementsDue(usize),
	OffTarget(usize),
}

/// USD held stable and the worst state across stable users: off target beats a settlement due beats at peg.
pub fn stabilized(stable: &ListStableChannelsResponse, price: Option<f64>) -> (f64, Peg) {
	let mut usd = 0.0;
	let (mut off, mut due) = (0, 0);
	for sc in stable.channels.iter().filter(|sc| has_stable_position(sc)) {
		usd += sc.expected_usd;
		let Some(d) = stable_drift(sc, price) else { continue };
		let px = if sc.latest_price > 0.0 { sc.latest_price } else { price.unwrap_or(0.0) };
		if d.level == DriftLevel::OffTarget {
			off += 1;
		} else if px > 0.0 && !matches!(settlement(&d, sc.expected_usd, px), Settlement::NotYet { .. }) {
			due += 1;
		}
	}
	let peg = if off > 0 {
		Peg::OffTarget(off)
	} else if due > 0 {
		Peg::SettlementsDue(due)
	} else {
		Peg::AtPar
	};
	(usd, peg)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Room {
	pub spendable_sats: u64,
	/// Mean size of the private channels the LSP opened; None before the first JIT channel.
	pub avg_jit_sats: Option<u64>,
	/// How many more such channels the spendable on-chain balance funds.
	pub channels: Option<u64>,
}

/// LSPS2 opens JIT channels outbound and unannounced; their average size is the unit of growth.
pub fn room_to_grow(balances: &GetBalancesResponse, channels: &ListChannelsResponse) -> Room {
	let jit: Vec<u64> =
		channels.channels.iter().filter(|c| c.is_outbound && !c.is_announced).map(|c| c.channel_value_sats).collect();
	let spendable_sats = balances.spendable_onchain_balance_sats;
	let avg_jit_sats = (!jit.is_empty()).then(|| jit.iter().sum::<u64>() / jit.len() as u64).filter(|avg| *avg > 0);
	Room { spendable_sats, avg_jit_sats, channels: avg_jit_sats.map(|avg| spendable_sats / avg) }
}

// ---------- The cross-channel feed ----------

/// One Channel History entry, tagged with the channel it belongs to.
#[derive(Clone, PartialEq, Debug)]
pub struct FeedEntry {
	/// user_channel_id, else the channel id, else empty for events with no channel reference.
	pub channel: String,
	/// The peer named in the events themselves, when any.
	pub node_id: Option<String>,
	pub entry: HistoryEntry,
}

fn channel_key(event: &ChannelLedgerEvent) -> String {
	for role in ["user_channel_id", "channel_id"] {
		if let Some(r) = event.refs.iter().find(|r| r.role == role && !r.value.is_empty()) {
			return r.value.clone();
		}
	}
	String::new()
}

fn detail_str(event: &ChannelLedgerEvent, key: &str) -> Option<String> {
	detail(event).get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A settlement the LSP sent (or tried to send) to the user's wallet.
fn to_user(events: &[ChannelLedgerEvent]) -> bool {
	events.iter().any(|e| detail_str(e, "direction").is_some_and(|d| d == "lsp_to_user" || d == "outbound"))
}

fn node_id_of(events: &[ChannelLedgerEvent]) -> Option<String> {
	events.iter().find_map(|e| {
		let d = detail(e);
		["counterparty_node_id", "node_id", "counterparty"]
			.iter()
			.find_map(|k| d.get(k).and_then(Value::as_str).map(str::to_owned))
			.or_else(|| d.get("node_ids").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).map(str::to_owned))
	})
}

/// Events grouped by channel, each group in ledger order.
pub fn split_by_channel(events: &[ChannelLedgerEvent]) -> HashMap<String, Vec<ChannelLedgerEvent>> {
	let mut by: HashMap<String, Vec<ChannelLedgerEvent>> = HashMap::new();
	for e in events {
		by.entry(channel_key(e)).or_default().push(e.clone());
	}
	by
}

/// Every channel's history entries merged newest first (routine entries included; see `recent_activity`).
pub fn feed(events: &[ChannelLedgerEvent]) -> Vec<FeedEntry> {
	let mut out: Vec<FeedEntry> = split_by_channel(events)
		.into_iter()
		.flat_map(|(channel, events)| {
			history::build_entries(&events, &channel).into_iter().map(move |entry| {
				let node_id = node_id_of(&entry.events);
				FeedEntry { channel: channel.clone(), node_id, entry }
			})
		})
		.collect();
	out.sort_by(|a, b| b.entry.occurred_at_ms.cmp(&a.entry.occurred_at_ms).then(b.entry.key.cmp(&a.entry.key)));
	out
}

/// Non-routine entries newer than `now_ms - window_ms`, newest first.
pub fn recent_activity(feed: &[FeedEntry], now_ms: i64, window_ms: i64) -> Vec<&FeedEntry> {
	feed.iter().filter(|f| !history::is_routine(&f.entry) && now_ms - f.entry.occurred_at_ms < window_ms).collect()
}

// ---------- Names and targets ----------

/// Everything the dashboard reads, plus the lookups that turn ids into names.
pub struct Ctx<'a> {
	pub now: u64,
	pub best_height: Option<u32>,
	pub channels: Option<&'a ListChannelsResponse>,
	pub balances: Option<&'a GetBalancesResponse>,
	pub stable: Option<&'a ListStableChannelsResponse>,
	/// None while the feed is unavailable (older daemon or a failed request).
	pub feed: Option<&'a [FeedEntry]>,
	/// Older pages exist beyond the loaded feed, so an event can be older than everything loaded.
	pub feed_has_more: bool,
	pub aliases: &'a HashMap<String, Option<String>>,
}

/// The daemon (or the nginx in front of it) does not know the request: an older deployment.
pub fn needs_newer_daemon(error: &str) -> bool {
	error.contains("identifier is required") || error.contains("HTTP 404")
}

impl Ctx<'_> {
	fn channel(&self, uid: &str) -> Option<&Channel> {
		self.channels?.channels.iter().find(|c| c.user_channel_id == uid || c.channel_id == uid)
	}

	fn node_for(&self, uid: &str) -> Option<String> {
		self.channel(uid)
			.map(|c| c.counterparty_node_id.clone())
			.or_else(|| self.stable?.channels.iter().find(|s| s.user_channel_id == uid || s.channel_id == uid).map(|s| s.counterparty.clone()))
	}

	pub fn name(&self, node_id: &str) -> String {
		match self.aliases.get(node_id).cloned().flatten() {
			Some(alias) => alias,
			None if node_id.is_empty() => "unknown peer".to_owned(),
			None => truncate_id(node_id, 8, 6),
		}
	}

	/// Peer name for a feed entry's channel, from the channel lists or the events themselves.
	pub fn name_for(&self, entry: &FeedEntry) -> String {
		match self.node_for(&entry.channel).or_else(|| entry.node_id.clone()) {
			Some(node) => self.name(&node),
			None if entry.channel.is_empty() => "LSP".to_owned(),
			None => truncate_id(&entry.channel, 8, 6),
		}
	}

	/// The side panel while the channel is listed, its history once it is gone.
	pub fn target(&self, uid: &str) -> Target {
		match self.channel(uid) {
			Some(c) => Target::Channel(c.user_channel_id.clone()),
			None => Target::History(uid.to_owned()),
		}
	}

	fn entries_of<'b>(&'b self, uid: &str) -> impl Iterator<Item = &'b FeedEntry> + 'b {
		let uid = uid.to_owned();
		self.feed.unwrap_or(&[]).iter().filter(move |f| f.channel == uid)
	}

	fn events<'b>(&'b self) -> impl Iterator<Item = (&'b str, &'b ChannelLedgerEvent)> + 'b {
		self.feed.unwrap_or(&[]).iter().flat_map(|f| f.entry.events.iter().map(move |e| (f.channel.as_str(), e)))
	}

	fn now_ms(&self) -> i64 {
		self.now as i64 * 1_000
	}
}

// ---------- Closing stages ----------

pub const CLOSE_STAGES: [&str; 4] = ["shutdown started", "resolving HTLCs", "negotiating fee", "closed"];

/// Index into `CLOSE_STAGES` for an LDK ChannelShutdownState; None when not shutting down.
pub fn close_stage(state: Option<i32>) -> Option<usize> {
	match state? {
		2 => Some(0),
		3 => Some(1),
		4 => Some(2),
		5 => Some(3),
		_ => None,
	}
}

/// When the channel's shutdown last changed stage, from the feed.
fn stage_since_ms(c: &Ctx, uid: &str) -> Option<i64> {
	c.events()
		.filter(|(ch, e)| *ch == uid && e.event_type == "CHANNEL_SHUTDOWN_STATE_CHANGED")
		.map(|(_, e)| e.occurred_at_ms)
		.max()
}

// ---------- In progress ----------

#[derive(Clone, PartialEq, Debug)]
pub struct Progress {
	pub icon: &'static str,
	pub title: String,
	pub detail: String,
	pub target: Target,
	/// A stage strip: the stages and the current one.
	pub stages: Option<(&'static [&'static str], usize)>,
}

fn duration(secs: u64) -> String {
	match secs {
		0..=89 => "a minute".to_owned(),
		90..=5_399 => format!("{} min", (secs + 30) / 60),
		5_400..=172_799 => format!("{} h", (secs + 1_800) / 3_600),
		_ => format!("{} days", secs / 86_400),
	}
}

fn clock(ms: i64) -> String {
	crate::format::local_time((ms / 1_000).max(0) as u64)
}

fn confirmed_uids(c: &Ctx) -> HashSet<(String, i64)> {
	c.events()
		.filter(|(_, e)| {
			matches!(
				e.event_type.as_str(),
				"CHANNEL_READY_SPLICE" | "SPLICE_IN_RECONCILED" | "SPLICE_OUT_STABLE_RECONCILED" | "SPLICE_FAILED"
			)
		})
		.map(|(ch, e)| (ch.to_owned(), e.occurred_at_ms))
		.collect()
}

/// Channels whose newest negotiated splice has not been reported ready or reconciled since.
pub fn splicing(c: &Ctx) -> Vec<(String, i64)> {
	let done = confirmed_uids(c);
	let mut newest: HashMap<String, i64> = HashMap::new();
	for (ch, e) in c.events().filter(|(_, e)| e.event_type == "SPLICE_NEGOTIATED") {
		let at = newest.entry(ch.to_owned()).or_insert(e.occurred_at_ms);
		*at = (*at).max(e.occurred_at_ms);
	}
	let mut out: Vec<(String, i64)> = newest
		.into_iter()
		.filter(|(ch, at)| !done.iter().any(|(d, dat)| d == ch && dat > at))
		.collect();
	out.sort();
	out
}

fn tx_type_words(t: &str) -> String {
	match t {
		"FUNDING" => "Funding transaction".to_owned(),
		"INTERACTIVE_FUNDING" => "Splice transaction".to_owned(),
		"COOPERATIVE_CLOSE" => "Close transaction".to_owned(),
		"UNILATERAL_CLOSE" => "Force-close transaction".to_owned(),
		"ANCHOR_BUMP" => "Fee bump".to_owned(),
		"CLAIM" => "Claim transaction".to_owned(),
		"SWEEP" => "Sweep transaction".to_owned(),
		other => format!("{} transaction", humanize_enum(other)),
	}
}

/// Normal work underway, each item linking to its channel.
pub fn in_progress(c: &Ctx, fmt_sats: &dyn Fn(u64) -> String) -> Vec<Progress> {
	let mut items = Vec::new();
	if let Some(channels) = c.channels {
		for ch in &channels.channels {
			let peer = c.name(&ch.counterparty_node_id);
			let target = Target::Channel(ch.user_channel_id.clone());
			if !ch.is_channel_ready && close_stage(ch.channel_shutdown_state).is_none() {
				let confirmations = match (ch.confirmations, ch.confirmations_required) {
					(Some(have), Some(need)) => format!("{have}/{need} confirmations"),
					_ => "waiting for the funding transaction to confirm".to_owned(),
				};
				let funding = ch.funding_txo.as_ref().map(|o| format!(" · funding {}", truncate_id(&o.txid, 8, 6))).unwrap_or_default();
				items.push(Progress {
					icon: "plus",
					title: format!("Channel with {peer} opening"),
					detail: format!("{confirmations}{funding}"),
					target,
					stages: None,
				});
			} else if let Some(stage) = close_stage(ch.channel_shutdown_state) {
				let since = stage_since_ms(c, &ch.user_channel_id);
				let detail = match since {
					Some(ms) => format!("{} for {}", CLOSE_STAGES[stage], duration(c.now.saturating_sub((ms / 1_000).max(0) as u64))),
					None => CLOSE_STAGES[stage].to_owned(),
				};
				items.push(Progress {
					icon: "x",
					title: format!("Closing the channel with {peer}"),
					detail,
					target,
					stages: Some((&CLOSE_STAGES, stage)),
				});
			}
		}
	}
	for (uid, since) in splicing(c) {
		let peer = c.node_for(&uid).map(|n| c.name(&n)).unwrap_or_else(|| truncate_id(&uid, 8, 6));
		items.push(Progress {
			icon: "split",
			title: format!("Splice with {peer} agreed, waiting to confirm"),
			detail: format!("since {}", clock(since)),
			target: c.target(&uid),
			stages: None,
		});
	}
	if let Some(b) = c.balances {
		for bal in &b.lightning_balances {
			if let Some(lightning_balance::BalanceType::ClaimableAwaitingConfirmations(x)) = &bal.balance_type {
				let peer = c.name(&x.counterparty_node_id);
				let eta = c
					.best_height
					.filter(|h| x.confirmation_height > *h)
					.map(|h| format!(" (~{})", duration((x.confirmation_height - h) as u64 * BLOCK_SECS)))
					.unwrap_or_default();
				items.push(Progress {
					icon: "clock",
					title: format!("{} from the close with {peer} unlock at block {}{eta}", fmt_sats(x.amount_satoshis), x.confirmation_height),
					detail: "Locked by the close's timelock, then swept to the on-chain wallet.".to_owned(),
					target: c.target(&x.channel_id),
					stages: None,
				});
			}
		}
		for sweep in &b.pending_balances_from_channel_closures {
			let (title, detail, channel) = match &sweep.balance_type {
				Some(pending_sweep_balance::BalanceType::PendingBroadcast(x)) => (
					format!("{} waiting to be swept", fmt_sats(x.amount_satoshis)),
					"The sweep transaction has not been broadcast yet.".to_owned(),
					x.channel_id.clone(),
				),
				Some(pending_sweep_balance::BalanceType::BroadcastAwaitingConfirmation(x)) => (
					format!("Sweep of {} broadcast, waiting to confirm", fmt_sats(x.amount_satoshis)),
					format!("tx {}", truncate_id(&x.latest_spending_txid, 8, 6)),
					x.channel_id.clone(),
				),
				Some(pending_sweep_balance::BalanceType::AwaitingThresholdConfirmations(x)) => (
					format!("Sweep of {} confirmed, waiting for more confirmations", fmt_sats(x.amount_satoshis)),
					format!("confirmed at block {}", x.confirmation_height),
					x.channel_id.clone(),
				),
				None => continue,
			};
			let target = channel.as_deref().map(|id| c.target(id)).unwrap_or(Target::Balances);
			items.push(Progress { icon: "download", title, detail, target, stages: None });
		}
	}
	// On-chain channel transactions the ledger saw broadcast and not yet confirmed.
	let mut pending: HashMap<String, (&str, &ChannelLedgerEvent)> = HashMap::new();
	let mut settled: HashSet<String> = HashSet::new();
	for (ch, e) in c.events().filter(|(_, e)| e.event_type == "CHANNEL_ONCHAIN_TX") {
		let txid = detail_str(e, "txid").unwrap_or_default();
		if e.status == "pending" {
			let slot = pending.entry(txid).or_insert((ch, e));
			if e.occurred_at_ms > slot.1.occurred_at_ms {
				*slot = (ch, e);
			}
		} else {
			settled.insert(txid);
		}
	}
	let mut waiting: Vec<_> = pending.into_iter().filter(|(txid, _)| !settled.contains(txid)).collect();
	waiting.sort_by(|a, b| b.1 .1.occurred_at_ms.cmp(&a.1 .1.occurred_at_ms));
	for (txid, (ch, e)) in waiting {
		let peer = c.node_for(ch).map(|n| c.name(&n)).unwrap_or_else(|| truncate_id(ch, 8, 6));
		items.push(Progress {
			icon: "cube",
			title: format!("{} with {peer} broadcast, waiting to confirm", tx_type_words(&detail_str(e, "tx_type").unwrap_or_default())),
			detail: format!("tx {} · since {}", truncate_id(&txid, 8, 6), clock(e.occurred_at_ms)),
			target: c.target(ch),
			stages: None,
		});
	}
	items
}

// ---------- Feed-derived attention ----------

/// Alerts that need the feed or the tiles: failing settlements, abandoned syncs, failed splices, stuck closes, low funds.
pub fn feed_attention(c: &Ctx, room: Option<&Room>, fmt_sats: &dyn Fn(u64) -> String) -> Vec<Attention> {
	let mut items = Vec::new();
	let now_ms = c.now_ms();
	let mut channels: Vec<&str> = c.feed.unwrap_or(&[]).iter().map(|f| f.channel.as_str()).collect();
	channels.sort();
	channels.dedup();
	for uid in channels {
		let peer_of = |entry: &FeedEntry| c.name_for(entry);
		// Stability payments to this user that failed in the last 24 h (a user's own failed payment is theirs to fix).
		let failed: Vec<&FeedEntry> = c
			.entries_of(uid)
			.filter(|f| f.entry.kind == EntryKind::Stability && f.entry.failed && to_user(&f.entry.events) && now_ms - f.entry.occurred_at_ms < DAY_MS)
			.collect();
		if let Some(last) = failed.first() {
			let n = failed.len();
			let reason = history::failure_detail(&last.entry.events).unwrap_or_else(|| "no reason recorded".to_owned());
			items.push(Attention {
				severity: Severity::Danger,
				title: format!("{n} stability payment{} to {} failed", if n == 1 { "" } else { "s" }, peer_of(last)),
				detail: format!("Latest failure: {reason}."),
				target: c.target(uid),
			});
		}
		// The daemon stopped retrying balance syncs after the last one that went through.
		let exhausted = c
			.events()
			.filter(|(ch, e)| *ch == uid && e.event_type == "SYNC_RETRY_EXHAUSTED")
			.map(|(_, e)| e.occurred_at_ms)
			.max();
		let last_good_sync = c
			.entries_of(uid)
			.filter(|f| f.entry.kind == EntryKind::Sync && !f.entry.failed)
			.map(|f| f.entry.occurred_at_ms)
			.max();
		if let Some(at) = exhausted.filter(|at| last_good_sync.is_none_or(|good| *at > good)) {
			if let Some(entry) = c.entries_of(uid).next() {
				items.push(Attention {
					severity: Severity::Warning,
					title: format!("Balance sync with {} gave up", peer_of(entry)),
					detail: format!("The LSP stopped retrying at {}; the wallet may be offline or rejecting syncs.", clock(at)),
					target: c.target(uid),
				});
			}
		}
		// A splice negotiation failed in the last 24 h.
		if let Some(f) = c
			.entries_of(uid)
			.find(|f| f.entry.events.iter().any(|e| e.event_type == "SPLICE_NEGOTIATION_FAILED") && now_ms - f.entry.occurred_at_ms < DAY_MS)
		{
			items.push(Attention {
				severity: Severity::Warning,
				title: format!("Splice with {} failed", peer_of(f)),
				detail: format!("Nothing changed on-chain; the channel keeps its current funding. At {}.", clock(f.entry.occurred_at_ms)),
				target: c.target(uid),
			});
		}
	}
	if let Some(channels) = c.channels {
		for ch in &channels.channels {
			let Some(stage) = close_stage(ch.channel_shutdown_state) else { continue };
			// Stage age comes from the feed: missing from a full page means older than it, from a short page not yet polled (30 s).
			let Some(since) = stage_since_ms(c, &ch.user_channel_id).or_else(|| {
				if !c.feed_has_more {
					return None;
				}
				c.feed?.iter().map(|f| f.entry.started_at_ms).min()
			}) else {
				continue;
			};
			let age = c.now.saturating_sub((since / 1_000).max(0) as u64) as i64;
			if age > STUCK_CLOSE_SECS && stage < CLOSE_STAGES.len() - 1 {
				items.push(Attention {
					severity: Severity::Warning,
					title: format!("Close with {} stuck at {} for {}", c.name(&ch.counterparty_node_id), CLOSE_STAGES[stage], duration(age as u64)),
					detail: "A cooperative close normally finishes in minutes; the peer may be offline. Force-closing is the fallback.".to_owned(),
					target: Target::Channel(ch.user_channel_id.clone()),
				});
			}
		}
	}
	if let Some(room) = room {
		if let (Some(avg), Some(n)) = (room.avg_jit_sats, room.channels) {
			if n < LOW_ROOM_CHANNELS {
				items.push(Attention {
					severity: Severity::Warning,
					title: "Running low for new users".to_owned(),
					detail: format!(
						"{} spendable on-chain covers about {n} more JIT channel{} at your average size of {}.",
						fmt_sats(room.spendable_sats),
						if n == 1 { "" } else { "s" },
						fmt_sats(avg)
					),
					target: Target::Balances,
				});
			}
		}
	}
	items
}

#[cfg(test)]
mod tests {
	use super::*;
	use sc_rest_client::ldk_server_grpc::api::GetBalancesResponse;
	use sc_rest_client::ldk_server_grpc::types::{
		ClaimableAwaitingConfirmations, LightningBalance, OutPoint, Peer, PendingSweepBalance, PendingBroadcast,
	};
	use sc_rest_client::sc_protos::stable::{LedgerRef, StableChannelInfo};

	const NOW: u64 = 1_800_000_000;

	fn fmt(sats: u64) -> String {
		format!("{sats} sats")
	}

	fn ev(id: i64, event_type: &str, uid: &str, secs_ago: u64, detail: serde_json::Value) -> ChannelLedgerEvent {
		let mut refs = vec![LedgerRef { role: "user_channel_id".into(), value: uid.into() }];
		for role in ["payment_id", "settlement_id", "trade_id"] {
			if let Some(v) = detail.get(role).and_then(Value::as_str) {
				refs.push(LedgerRef { role: role.into(), value: v.into() });
			}
		}
		ChannelLedgerEvent {
			id,
			event_type: event_type.into(),
			status: detail.get("status").and_then(Value::as_str).unwrap_or("completed").into(),
			occurred_at_ms: (NOW - secs_ago) as i64 * 1_000,
			detail_json: detail.to_string(),
			refs,
			..Default::default()
		}
	}

	fn channel(uid: &str, node: &str) -> Channel {
		Channel {
			user_channel_id: uid.into(),
			channel_id: format!("chan-{uid}"),
			counterparty_node_id: node.into(),
			is_channel_ready: true,
			is_usable: true,
			..Default::default()
		}
	}

	fn ctx<'a>(channels: Option<&'a ListChannelsResponse>, feed: Option<&'a [FeedEntry]>, aliases: &'a HashMap<String, Option<String>>) -> Ctx<'a> {
		Ctx { now: NOW, best_height: Some(900_000), channels, balances: None, stable: None, feed, feed_has_more: false, aliases }
	}

	#[test]
	fn stable_users_count_targets_online_wallets_and_new_ones() {
		let stable = ListStableChannelsResponse {
			channels: vec![
				StableChannelInfo { user_channel_id: "1".into(), counterparty: "02aa".into(), expected_usd: 50.0, created_at: NOW as i64 - 3 * 86_400, ..Default::default() },
				StableChannelInfo { user_channel_id: "2".into(), counterparty: "02bb".into(), expected_usd: 20.0, created_at: NOW as i64 - 30 * 86_400, ..Default::default() },
				StableChannelInfo { user_channel_id: "3".into(), counterparty: "02cc".into(), expected_usd: 0.0, created_at: NOW as i64, ..Default::default() },
			],
		};
		let peers = ListPeersResponse {
			peers: vec![
				Peer { node_id: "02aa".into(), is_connected: true, ..Default::default() },
				Peer { node_id: "02bb".into(), is_connected: false, ..Default::default() },
			],
		};
		assert_eq!(stable_users(&stable, None, NOW), StableUsers { count: 2, online: None, new_this_week: 1 });
		assert_eq!(stable_users(&stable, Some(&peers), NOW).online, Some(1));
		let unknown = ListStableChannelsResponse { channels: vec![StableChannelInfo { expected_usd: 5.0, ..Default::default() }] };
		assert_eq!(stable_users(&unknown, None, NOW).new_this_week, 0, "created_at 0 never counts as new");
	}

	#[test]
	fn stabilized_reports_the_worst_peg_state() {
		let sc = |uid: &str, usd: f64, backing: u64| StableChannelInfo {
			user_channel_id: uid.into(),
			expected_usd: usd,
			expected_msats: backing * 1_000,
			latest_price: 100_000.0,
			..Default::default()
		};
		let calm = ListStableChannelsResponse { channels: vec![sc("1", 50.0, 50_000), sc("2", 50.0, 50_100)] };
		assert_eq!(stabilized(&calm, None), (100.0, Peg::AtPar));
		let due = ListStableChannelsResponse { channels: vec![sc("1", 50.0, 50_000), sc("2", 50.0, 50_400)] };
		assert_eq!(stabilized(&due, None).1, Peg::SettlementsDue(1));
		let off = ListStableChannelsResponse { channels: vec![sc("1", 50.0, 48_000), sc("2", 50.0, 50_400)] };
		assert_eq!(stabilized(&off, None).1, Peg::OffTarget(1), "off target wins over a settlement due");
	}

	#[test]
	fn room_to_grow_uses_the_average_private_outbound_channel() {
		let balances = GetBalancesResponse { spendable_onchain_balance_sats: 2_500_000, ..Default::default() };
		let jit = |uid: &str, sats: u64| Channel { user_channel_id: uid.into(), is_outbound: true, is_announced: false, channel_value_sats: sats, ..Default::default() };
		let channels = ListChannelsResponse {
			channels: vec![jit("1", 800_000), jit("2", 1_200_000), Channel { is_outbound: true, is_announced: true, channel_value_sats: 9_000_000, ..Default::default() }],
		};
		assert_eq!(room_to_grow(&balances, &channels), Room { spendable_sats: 2_500_000, avg_jit_sats: Some(1_000_000), channels: Some(2) });
		let none = ListChannelsResponse { channels: vec![] };
		assert_eq!(room_to_grow(&balances, &none), Room { spendable_sats: 2_500_000, avg_jit_sats: None, channels: None });
	}

	#[test]
	fn the_feed_merges_channels_newest_first_and_hides_routine_entries() {
		let events = vec![
			ev(1, "SYNC_MESSAGE_SENT", "a", 3_000, serde_json::json!({"expected_usd": 50.0, "payment_id": "s1"})),
			ev(2, "TRADE_APPLIED", "b", 2_000, serde_json::json!({"new_expected_usd": 20.0, "trade_id": "t1"})),
			ev(3, "PEER_CONNECTED", "a", 1_000, serde_json::json!({})),
			ev(4, "STABILITY_PAYMENT_V1_SENT", "a", 500, serde_json::json!({"direction": "lsp_to_user", "amount_msat": 20_000, "payment_id": "p1"})),
			ev(5, "PAYMENT_FAILED", "a", 400, serde_json::json!({"payment_id": "p1", "reason": "ROUTE_NOT_FOUND"})),
			ev(6, "CHANNEL_PENDING", "", 100, serde_json::json!({})),
		];
		let feed = feed(&events);
		let keys: Vec<(&str, i64)> = feed.iter().map(|f| (f.channel.as_str(), f.entry.key)).collect();
		assert_eq!(keys, [("", 6), ("a", 4), ("a", 3), ("b", 2), ("a", 1)], "one entry per flow, newest first, across channels");
		assert!(feed[1].entry.failed, "the failed settlement is one failed entry");
		let recent = recent_activity(&feed, NOW as i64 * 1_000, DAY_MS);
		let shown: Vec<i64> = recent.iter().map(|f| f.entry.key).collect();
		assert_eq!(shown, [6, 4, 2], "peer state and successful syncs are routine");
		assert!(recent_activity(&feed, NOW as i64 * 1_000, 200_000).iter().all(|f| f.entry.key == 6), "the window applies");
	}

	#[test]
	fn opening_splicing_closing_and_locked_funds_are_in_progress() {
		let mut opening = channel("1", "02aa");
		opening.is_channel_ready = false;
		opening.is_usable = false;
		opening.confirmations = Some(2);
		opening.confirmations_required = Some(3);
		opening.funding_txo = Some(OutPoint { txid: "f".repeat(64), vout: 0 });
		let mut closing = channel("2", "02bb");
		closing.channel_shutdown_state = Some(4);
		let channels = ListChannelsResponse { channels: vec![opening, closing, channel("3", "02cc")] };
		let events = vec![
			ev(1, "CHANNEL_SHUTDOWN_STATE_CHANGED", "2", 600, serde_json::json!({"shutdown_state": "NEGOTIATING_CLOSING_FEE"})),
			ev(2, "SPLICE_NEGOTIATED", "3", 900, serde_json::json!({"funding_txo": "aa:1"})),
			ev(3, "SPLICE_NEGOTIATED", "4", 800, serde_json::json!({"funding_txo": "bb:1", "counterparty_node_id": "02dd"})),
			ev(4, "CHANNEL_READY_SPLICE", "4", 700, serde_json::json!({})),
			ev(5, "CHANNEL_ONCHAIN_TX", "3", 300, serde_json::json!({"txid": "1".repeat(64), "tx_type": "INTERACTIVE_FUNDING", "status": "pending"})),
			ev(6, "CHANNEL_ONCHAIN_TX", "2", 200, serde_json::json!({"txid": "2".repeat(64), "tx_type": "COOPERATIVE_CLOSE", "status": "pending"})),
			ev(7, "CHANNEL_ONCHAIN_TX", "2", 100, serde_json::json!({"txid": "2".repeat(64), "tx_type": "COOPERATIVE_CLOSE", "status": "completed"})),
		];
		let feed = feed(&events);
		let aliases: HashMap<String, Option<String>> = [("02cc".to_string(), Some("Breez".to_string()))].into();
		let mut c = ctx(Some(&channels), Some(&feed), &aliases);
		let balances = GetBalancesResponse {
			lightning_balances: vec![LightningBalance {
				balance_type: Some(lightning_balance::BalanceType::ClaimableAwaitingConfirmations(ClaimableAwaitingConfirmations {
					channel_id: "chan-9".into(),
					counterparty_node_id: "02ee".into(),
					amount_satoshis: 40_210,
					confirmation_height: 900_036,
					source: 0,
				})),
			}],
			pending_balances_from_channel_closures: vec![PendingSweepBalance {
				balance_type: Some(pending_sweep_balance::BalanceType::PendingBroadcast(PendingBroadcast { channel_id: None, amount_satoshis: 5_000 })),
			}],
			..Default::default()
		};
		c.balances = Some(&balances);
		let items = in_progress(&c, &fmt);
		let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
		assert_eq!(titles, [
			"Channel with 02aa opening",
			"Closing the channel with 02bb",
			"Splice with Breez agreed, waiting to confirm",
			"40210 sats from the close with 02ee unlock at block 900036 (~6 h)",
			"5000 sats waiting to be swept",
			"Splice transaction with Breez broadcast, waiting to confirm",
		]);
		assert_eq!(items[0].detail, "2/3 confirmations · funding ffffffff..ffffff");
		assert_eq!(items[1].stages, Some((&CLOSE_STAGES[..], 2)));
		assert_eq!(items[1].detail, "negotiating fee for 10 min");
		assert_eq!(items[3].target, Target::History("chan-9".into()), "a closed channel links to its history");
		assert_eq!(items[4].target, Target::Balances);
		assert!(!titles.iter().any(|t| t.contains("02dd")), "a splice reported ready is no longer in progress");
	}

	#[test]
	fn feed_alerts_name_the_user_and_the_reason() {
		let channels = ListChannelsResponse { channels: vec![channel("a", "02aa"), channel("b", "02bb")] };
		let events = vec![
			ev(1, "STABILITY_PAYMENT_V1_SENT", "a", 7_000, serde_json::json!({"direction": "lsp_to_user", "amount_msat": 20_000, "payment_id": "p1"})),
			ev(2, "PAYMENT_FAILED", "a", 6_900, serde_json::json!({"payment_id": "p1", "reason": "ROUTE_NOT_FOUND"})),
			ev(3, "STABILITY_PAYMENT_FAILED", "a", 3_000, serde_json::json!({"direction": "lsp_to_user", "error": "no route to the wallet", "payment_id": "p2"})),
			ev(4, "SYNC_MESSAGE_SENT", "b", 5_000, serde_json::json!({"expected_usd": 20.0, "payment_id": "s1"})),
			ev(5, "SYNC_RETRY_EXHAUSTED", "b", 1_000, serde_json::json!({"attempts": 10})),
			ev(6, "SPLICE_NEGOTIATION_FAILED", "b", 500, serde_json::json!({})),
			// Two days old: outside every 24 h window.
			ev(7, "STABILITY_PAYMENT_FAILED", "b", 200_000, serde_json::json!({"direction": "lsp_to_user", "error": "old", "payment_id": "p9"})),
			// The user's own payment to the LSP failed: not the LSP's settlement.
			ev(10, "STABILITY_PAYMENT_FAILED", "a", 1_000, serde_json::json!({"direction": "user_to_lsp", "error": "wallet offline", "payment_id": "p3"})),
		];
		let feed = feed(&events);
		let aliases: HashMap<String, Option<String>> = [("02aa".to_string(), Some("Alice".to_string()))].into();
		let c = ctx(Some(&channels), Some(&feed), &aliases);
		let room = Room { spendable_sats: 900_000, avg_jit_sats: Some(500_000), channels: Some(1) };
		let items = feed_attention(&c, Some(&room), &fmt);
		let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
		assert_eq!(titles, [
			"2 stability payments to Alice failed",
			"Balance sync with 02bb gave up",
			"Splice with 02bb failed",
			"Running low for new users",
		]);
		assert_eq!(items[0].detail, "Latest failure: no route to the wallet.");
		assert_eq!(items[0].severity, Severity::Danger);
		assert_eq!(items[0].target, Target::Channel("a".into()));
		assert!(items[3].detail.contains("about 1 more JIT channel at your average size of 500000 sats"));

		// A sync that went through after the give-up clears it.
		let mut recovered = events.clone();
		recovered.push(ev(8, "SYNC_MESSAGE_SENT", "b", 100, serde_json::json!({"expected_usd": 20.0, "payment_id": "s2"})));
		let feed = self::feed(&recovered);
		let c = ctx(Some(&channels), Some(&feed), &aliases);
		assert!(!feed_attention(&c, None, &fmt).iter().any(|i| i.title.contains("gave up")));
	}

	#[test]
	fn a_close_sitting_on_one_stage_for_an_hour_is_stuck() {
		let mut closing = channel("2", "02bb");
		closing.channel_shutdown_state = Some(3);
		let channels = ListChannelsResponse { channels: vec![closing] };
		let fresh = vec![ev(1, "CHANNEL_SHUTDOWN_STATE_CHANGED", "2", 600, serde_json::json!({"shutdown_state": "RESOLVING_HTLCS"}))];
		let stale = vec![ev(1, "CHANNEL_SHUTDOWN_STATE_CHANGED", "2", 7_200, serde_json::json!({"shutdown_state": "RESOLVING_HTLCS"}))];
		let aliases = HashMap::new();
		let feed = feed(&fresh);
		assert!(feed_attention(&ctx(Some(&channels), Some(&feed), &aliases), None, &fmt).is_empty());
		let feed = self::feed(&stale);
		let items = feed_attention(&ctx(Some(&channels), Some(&feed), &aliases), None, &fmt);
		assert_eq!(items[0].title, "Close with 02bb stuck at resolving HTLCs for 2 h");
		assert!(feed_attention(&ctx(Some(&channels), None, &aliases), None, &fmt).is_empty(), "no feed, no age to judge");

		// No stage event loaded: only a full page (older pages exist) means the close predates everything loaded.
		let unrelated = vec![ev(1, "TRADE_APPLIED", "9", 10_000, serde_json::json!({"trade_id": "t"}))];
		let feed = self::feed(&unrelated);
		let mut c = ctx(Some(&channels), Some(&feed), &aliases);
		assert!(feed_attention(&c, None, &fmt).is_empty(), "a short page: the 30 s poll has not written the stage yet");
		c.feed_has_more = true;
		assert_eq!(feed_attention(&c, None, &fmt)[0].title, "Close with 02bb stuck at resolving HTLCs for 3 h");
	}

	#[test]
	fn an_old_daemon_is_recognised_by_its_refusal_or_a_missing_route() {
		assert!(needs_newer_daemon("Error: [InvalidRequestError]: An exact ledger identifier is required"));
		assert!(needs_newer_daemon("Error: [InternalError]: HTTP 404 Not Found"), "an empty 404 body carries its status");
		assert!(!needs_newer_daemon("HTTP request failed: connection reset"));
		assert!(!needs_newer_daemon("Error: [InvalidRequestError]: unknown channel 4041"), "a 404 inside other text is not a missing route");
	}
}
