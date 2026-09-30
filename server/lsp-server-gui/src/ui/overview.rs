//! Owner dashboard: how the business is going and what is happening right now.

use dioxus::prelude::*;

use crate::actions;
use crate::dashboard::{self, FeedEntry, Peg, Progress};
use crate::format::{format_usd, local_time, relative_short};
use crate::health::{self, Attention, Severity, Target};
use crate::state::{ActiveTab, AppCtx, Drawer, Op, RevenueWindow};
use crate::ui::open_drawer;
use crate::ui::revenue::{signed, totals};
use crate::ui::widgets::{Bubble, Card, Empty, Gate, Icon, Pill, RefreshBtn, Spinner, Stat};

const HELP_USERS: &str = "Channels with a USD target. Online counts the wallets connected right now; new means the LSP started tracking them in the last 7 days.";
const HELP_STABILIZED: &str = "Sum of the USD targets the LSP keeps stable, and whether every position is at its peg.";
const HELP_NET: &str = "Fees earned minus fees paid over the last 7 days. Stability settlements are the peg working, not revenue, so they are left out.";
const HELP_ROOM: &str = "How many more JIT channels the spendable on-chain balance can fund at the average size of the private channels this LSP has opened.";
const HELP_ATTENTION: &str = "Things the operator may need to act on. Normal work underway is listed under In progress instead.";
const HELP_PROGRESS: &str = "Channels opening, splicing or closing, funds unlocking after a close, and channel transactions waiting to confirm.";
const HELP_ACTIVITY: &str = "What happened across every channel, one line per trade, settlement or lifecycle step. Successful balance syncs and wallet reconnects are left out; Channel History has them.";
const SHORT_LIST: usize = 15;
const LONG_LIST: usize = 60;

#[component]
pub fn Overview() -> Element {
	let ctx = use_context::<AppCtx>();
	// Grouping the feed parses every event; do it when the feed changes, not on every clock tick.
	let feed_memo = use_memo(move || ctx.data.read().activity.as_ref().map(|a| dashboard::feed(&a.events)));
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	// Recent activity starts at 24 h; "Show more" widens it to 7 days and pages the feed.
	let extended = ctx.view.read().activity_extended;
	let now = *ctx.now.read();
	let price = ctx.price_value();
	let fmt = |sats: u64| ctx.fmt_sats(sats);
	let data = ctx.data.read();
	let feed_entries = feed_memo.read();
	let best_height = data.node_info.as_ref().and_then(|n| n.current_best_block.as_ref()).map(|b| b.height);
	let dctx = dashboard::Ctx {
		now,
		best_height,
		channels: data.channels.as_ref(),
		balances: data.balances.as_ref(),
		stable: data.stable_channels.as_ref(),
		feed: feed_entries.as_deref(),
		feed_has_more: data.activity_cursor.is_some(),
		aliases: &data.aliases,
	};
	let users = data.stable_channels.as_ref().map(|s| dashboard::stable_users(s, data.peers.as_ref(), now));
	let peg = data.stable_channels.as_ref().map(|s| dashboard::stabilized(s, price));
	let week = data.revenue_week.as_ref().map(|r| totals(&r.lines));
	let week_sub_error = data.revenue_week_error.as_ref().map(|_| {
		if data.revenue_week_unsupported { "Revenue needs the updated daemon" } else { "Revenue unavailable right now" }
	});
	let room = match (data.balances.as_ref(), data.channels.as_ref()) {
		(Some(b), Some(c)) => Some(dashboard::room_to_grow(b, c)),
		_ => None,
	};
	let mut items = health::attention(
		&health::Snapshot {
			now,
			price,
			node_info: data.node_info.as_ref(),
			channels: data.channels.as_ref(),
			stable: data.stable_channels.as_ref(),
			peers: data.peers.as_ref(),
			payments: data.payments.as_ref(),
			aliases: &data.aliases,
		},
		&fmt,
	);
	items.extend(dashboard::feed_attention(&dctx, room.as_ref(), &fmt));
	items.sort_by_key(|item| item.severity);
	let progress = dashboard::in_progress(&dctx, &fmt);
	let window_ms = if extended { dashboard::WEEK_MS } else { dashboard::DAY_MS };
	let recent: Option<Vec<FeedEntry>> = feed_entries.as_ref().map(|f| {
		dashboard::recent_activity(f, now as i64 * 1_000, window_ms)
			.into_iter()
			.take(if extended { LONG_LIST } else { SHORT_LIST })
			.cloned()
			.collect()
	});
	let peers_of: Vec<String> = recent.iter().flatten().map(|e| dctx.name_for(e)).collect();
	let activity_error = data.activity_error.clone();
	let activity_unsupported = activity_error.as_deref().is_some_and(dashboard::needs_newer_daemon);
	let activity_more = data.activity_cursor.is_some();
	let loading = data.channels.is_none() || data.balances.is_none();
	let count = items.len();
	let count_tone = if items.iter().any(|i| i.severity == Severity::Danger) { "danger" } else { "warning" };
	drop(data);
	drop(feed_entries);
	let busy = [Op::NodeInfo, Op::Balances, Op::Channels, Op::ListStableChannels, Op::Peers, Op::Payments, Op::RevenueWeek, Op::ActivityFeed]
		.iter()
		.any(|op| ctx.busy(*op));
	let feed_busy = ctx.busy(Op::ActivityFeed);
	let refresh_all = move |_| {
		actions::fetch_node_info(ctx);
		actions::fetch_balances(ctx);
		actions::fetch_channels(ctx);
		actions::fetch_stable_channels(ctx);
		actions::fetch_peers(ctx);
		actions::refresh_first_payments_page(ctx);
		actions::fetch_revenue_week(ctx);
		// A manual refresh reloads the newest page; older pages come back through "Load older".
		actions::fetch_activity_feed(ctx, false);
	};
	let show_more = move |_| {
		let mut view = ctx.view;
		if !view.peek().activity_extended {
			view.write().activity_extended = true;
		}
		actions::fetch_activity_feed(ctx, true);
	};
	let open_revenue_week = move |_| {
		let mut view = ctx.view;
		view.write().revenue_window = RevenueWindow::Week;
		let mut nav = ctx.nav;
		nav.write().active_tab = ActiveTab::Revenue;
		actions::fetch_revenue(ctx, false);
	};
	let dash = || "—".to_string();
	let users_sub = users.as_ref().map(|u| {
		let online = u.online.map(|n| n.to_string()).unwrap_or_else(dash);
		format!("{online} online · +{} new this week", u.new_this_week)
	});
	let peg_sub = peg.map(|(_, peg)| match peg {
		Peg::AtPar => "all at peg".to_string(),
		Peg::SettlementsDue(n) => format!("{n} settlement{} due", if n == 1 { "" } else { "s" }),
		Peg::OffTarget(n) => format!("{n} off target"),
	});
	let (week_value, week_sub) = match (&week, week_sub_error) {
		(Some((earned, spent, net)), _) => (signed(ctx, *net), format!("earned {} · spent {}", ctx.fmt_msat(*earned), ctx.fmt_msat(*spent))),
		(None, Some(e)) => (dash(), e.to_string()),
		(None, None) => (dash(), String::new()),
	};
	let feed_title = if activity_unsupported { "Activity needs the updated daemon" } else { "Activity feed unavailable" };
	let (room_value, room_sub) = match &room {
		Some(r) => match (r.channels, r.avg_jit_sats) {
			(Some(n), Some(avg)) => (
				format!("~{n} more channel{}", if n == 1 { "" } else { "s" }),
				format!("at your average JIT size of {} · {} on-chain", ctx.fmt_sats(avg), ctx.fmt_sats(r.spendable_sats)),
			),
			_ => (ctx.fmt_sats(r.spendable_sats), "no JIT channels yet".to_string()),
		},
		None => (dash(), String::new()),
	};

	rsx! {
		div { class: "row end",
			RefreshBtn { busy, onclick: refresh_all, label: "Refresh all", op: Op::Channels }
		}
		div { class: "grid-4",
			Stat {
				title: "Stable users",
				help: HELP_USERS,
				value: users.as_ref().map(|u| u.count.to_string()).unwrap_or_else(dash),
				sub: users_sub.unwrap_or_default(),
			}
			Stat {
				title: "Stabilized",
				help: HELP_STABILIZED,
				value: peg.map(|(usd, _)| format_usd(usd)).unwrap_or_else(dash),
				sub: peg_sub.unwrap_or_default(),
			}
			Stat { title: "Net this week", help: HELP_NET, value: week_value, sub: week_sub, onclick: open_revenue_week }
			Stat { title: "Room to grow", help: HELP_ROOM, value: room_value, sub: room_sub }
		}
		Card {
			title: "Needs attention",
			help: HELP_ATTENTION,
			actions: rsx! {
				if count > 0 {
					Pill { tone: count_tone, "{count}" }
				}
			},
			if loading && busy {
				div { class: "empty", Spinner { large: true } }
			} else if items.is_empty() {
				Empty { icon: "check", title: "All clear", hint: "Nothing needs attention right now." }
			} else {
				div { class: "attention-list",
					for (i, item) in items.into_iter().enumerate() {
						AttentionRow { key: "{i}", item }
					}
				}
			}
		}
		if !progress.is_empty() {
			Card { title: "In progress", help: HELP_PROGRESS,
				div { class: "attention-list",
					for (i, item) in progress.into_iter().enumerate() {
						ProgressRow { key: "{i}", item }
					}
				}
			}
		}
		Card {
			title: "Recent activity",
			help: HELP_ACTIVITY,
			sub: if extended { "Last 7 days across every channel" } else { "Last 24 hours across every channel" },
			class: "flush",
			match (recent, activity_error) {
				(None, Some(error)) => rsx! {
					Empty { icon: "list", title: feed_title, hint: error }
				},
				(None, None) => rsx! {
					div { class: "empty", Spinner { large: true } }
				},
				(Some(list), error) => rsx! {
					if let Some(error) = error {
						div { class: "small danger", style: "padding: 10px 16px;", "Last refresh failed: {error}" }
					}
					if list.is_empty() {
						Empty { icon: "list", title: "Nothing happened in this window", hint: "Trades, settlements and channel changes will show here." }
					}
					for (i, entry) in list.into_iter().enumerate() {
						ActivityRow { key: "{entry.channel}-{entry.entry.key}", entry, peer: peers_of[i].clone(), now }
					}
					if !extended || activity_more {
						div { class: "row", style: "padding: 12px 16px;",
							button { class: "btn sm", disabled: feed_busy, onclick: show_more,
								if feed_busy { Spinner {} } else { Icon { name: "chevron-down", size: 14 } }
								if extended { "Load older" } else { "Show more" }
							}
						}
					}
				},
			}
		}
	}
}

/// Follows an attention or progress item to its tab, side panel or history.
fn go(ctx: AppCtx, target: Target) {
	let mut nav = ctx.nav;
	match target {
		Target::Channel(id) => {
			nav.write().active_tab = ActiveTab::Channels;
			open_drawer(ctx, Drawer::Channel(id));
		},
		Target::History(id) => actions::open_channel_ledger(ctx, id),
		Target::FailedPayments => {
			let mut view = ctx.view;
			{
				let mut v = view.write();
				v.payment_status = 2;
				v.payment_filter.clear();
				v.payment_direction = -1;
				v.payment_type.clear();
			}
			nav.write().active_tab = ActiveTab::Payments;
		},
		Target::Peers => nav.write().active_tab = ActiveTab::Peers,
		Target::Balances => nav.write().active_tab = ActiveTab::Balances,
		Target::NodeInfo => nav.write().active_tab = ActiveTab::NodeInfo,
	}
}

#[component]
fn AttentionRow(item: Attention) -> Element {
	let ctx = use_context::<AppCtx>();
	let (icon, tone) = match item.severity {
		Severity::Danger => ("alert", "red"),
		Severity::Warning => ("alert", "orange"),
		Severity::Info => ("info", "blue"),
	};
	let target = item.target.clone();
	rsx! {
		div { class: "attention",
			Bubble { icon, tone, small: true }
			div { class: "grow stack tight", style: "gap: 2px;",
				span { class: "strong", "{item.title}" }
				span { class: "small muted", "{item.detail}" }
			}
			button { class: "btn sm", onclick: move |_| go(ctx, target.clone()), "View" Icon { name: "chevron-right", size: 14 } }
		}
	}
}

#[component]
fn ProgressRow(item: Progress) -> Element {
	let ctx = use_context::<AppCtx>();
	let target = item.target.clone();
	rsx! {
		div { class: "attention",
			Bubble { icon: item.icon, tone: "blue", small: true }
			div { class: "grow stack tight", style: "gap: 2px;",
				span { class: "strong", "{item.title}" }
				if let Some((stages, current)) = item.stages {
					div { class: "stages",
						for (i, stage) in stages.iter().enumerate() {
							span { key: "{stage}", class: if i < current { "stage done" } else if i == current { "stage now" } else { "stage" }, "{stage}" }
						}
					}
				}
				span { class: "small muted", "{item.detail}" }
			}
			button { class: "btn sm", onclick: move |_| go(ctx, target.clone()), "View" Icon { name: "chevron-right", size: 14 } }
		}
	}
}

/// One feed line: time, peer, what happened; click opens that channel's history.
#[component]
fn ActivityRow(entry: FeedEntry, peer: String, now: u64) -> Element {
	let ctx = use_context::<AppCtx>();
	let at = (entry.entry.occurred_at_ms / 1_000).max(0) as u64;
	let when = if now.saturating_sub(at) < 86_400 { local_time(at) } else { relative_short(at, now) };
	let channel = entry.channel.clone();
	// Events with no channel reference have no history to open.
	let linked = !channel.is_empty();
	let (tone, text) = if entry.entry.failed { ("danger", entry.entry.summary.clone()) } else { ("", entry.entry.summary.clone()) };
	rsx! {
		div { class: if linked { "hist-row feed-row" } else { "hist-row feed-row static" }, onclick: move |_| {
			if linked {
				actions::open_channel_ledger(ctx, channel.clone());
			}
		},
			span { class: "hist-time num", "{when}" }
			span { class: "hist-summary",
				span { class: "strong", "{peer}" }
				" · "
				span { class: tone, "{text}" }
			}
			span { class: "hist-amount num",
				if let Some(msat) = entry.entry.amount_msat.filter(|m| *m >= 1_000) {
					"{ctx.fmt_msat(msat)}"
				}
			}
			span {
				if linked {
					Icon { name: "chevron-right", size: 14 }
				}
			}
		}
	}
}
