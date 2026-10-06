//! Revenue tab: what the LSP earned, spent and settled for the peg, and every movement behind it.

use dioxus::prelude::*;
use sc_rest_client::sc_protos::revenue::{RevenueItem, RevenueLine};

use crate::actions;
use crate::format::csv_row;
use crate::state::{AppCtx, Dialog, Op, RefundTradeFeeForm, RevenueWindow};
use crate::ui::{close_dialog, open_dialog};
use crate::ui::widgets::{Bubble, Card, Empty, Gate, Icon, IdCopy, InfoTip, Kv, Modal, Peer, Pill, RefreshBtn, SegBtn, Spinner, Stat, Th};

const HELP_NET: &str = "Earned minus spent. Stability settlements are not included, nor routing fees paid on stability, protocol and refund payments.";
const HELP_STABILITY: &str = "Sats paid to users when BTC fell and received when it rose. This is the peg working, not income.";
const HELP_CLOSE_FEES: &str = "On-chain fees LDK reports for close transactions the wallet paid for. What a close itself costs the channel balance is not counted here; that needs the ledger.";
const HELP_UNTRACKED: &str = "This node's LDK Server does not list or classify these transactions, so the total would read as zero when it is not.";
const HELP_JIT_OPENS: &str = "On-chain fees the LSP paid to open private channels for users. Opening is free for them, so this is what the free service costs.";
const HELP_JIT: &str = "Opening fee skimmed from a JIT channel's first payment. Zero while the LSP opens channels for free.";

pub const EARNED: [&str; 3] = ["trade_fee", "routing_fee", "jit_fee"];
pub const SPENT: [&str; 9] = [
	"jit_open_fee", "channel_funding_fee", "close_fee", "close_fee_bump", "claim_sweep_fee", "onchain_fee", "lightning_send_fee",
	"protocol_message", "trade_fee_refund",
];
const FILTERS: [&str; 15] = [
	"trade_fee", "trade_fee_rejected", "routing_fee", "jit_fee", "stability_in", "stability_out", "jit_open_fee", "channel_funding_fee",
	"close_fee", "close_fee_bump", "claim_sweep_fee", "onchain_fee", "lightning_send_fee", "protocol_message", "trade_fee_refund",
];

/// Count and msat total of one category.
pub fn line(lines: &[RevenueLine], category: &str) -> (u64, u64) {
	lines.iter().find(|l| l.category == category).map(|l| (l.count, l.total_msat)).unwrap_or((0, 0))
}

/// Earned, spent and net msat; stability is left out.
pub fn totals(lines: &[RevenueLine]) -> (u64, u64, i128) {
	let sum = |categories: &[&str]| categories.iter().map(|c| line(lines, c).1).sum::<u64>();
	let (earned, spent) = (sum(&EARNED), sum(&SPENT));
	(earned, spent, earned as i128 - spent as i128)
}

/// A spent category the node cannot report makes spent a floor and net a ceiling; reads SPENT, the same list totals() sums.
pub fn spent_untracked(untracked: &[String]) -> bool {
	SPENT.iter().any(|c| untracked.iter().any(|u| u == c))
}

/// A net that may only be a ceiling reads "≤ x".
pub fn bounded(ctx: AppCtx, net: i128, ceiling: bool) -> String {
	let net = signed(ctx, net);
	if ceiling { format!("\u{2264} {net}") } else { net }
}

/// Stability received minus paid, in msat.
pub fn stability_net(lines: &[RevenueLine]) -> i128 {
	line(lines, "stability_in").1 as i128 - line(lines, "stability_out").1 as i128
}

pub fn category_label(category: &str) -> &str {
	match category {
		"trade_fee" => "Trade fee",
		"trade_fee_rejected" => "Rejected trades",
		"routing_fee" => "Routing",
		"jit_fee" => "JIT opening",
		"protocol_message" => "Protocol message",
		"trade_fee_refund" => "Trade fee refund",
		"jit_open_fee" => "JIT open",
		"channel_funding_fee" => "Channel open",
		"close_fee" => "Channel close",
		"close_fee_bump" => "Close fee bump",
		"claim_sweep_fee" => "Claim or sweep",
		"onchain_fee" => "On-chain fee",
		"lightning_send_fee" => "Send fee",
		"stability_in" => "Stability in",
		"stability_out" => "Stability out",
		other => other,
	}
}

fn category_tone(category: &str) -> &'static str {
	match category {
		"trade_fee" | "routing_fee" | "jit_fee" => "success",
		"stability_in" | "stability_out" => "info",
		_ => "orange",
	}
}

/// Unix seconds where the window starts; 0 for all time.
pub fn window_since(window: RevenueWindow, now: i64, today_start: i64) -> i64 {
	match window {
		RevenueWindow::Today => today_start,
		RevenueWindow::Week => now - 7 * 86_400,
		RevenueWindow::Month => now - 30 * 86_400,
		RevenueWindow::All => 0,
	}
}

/// Local midnight today, in unix seconds.
pub fn local_midnight() -> i64 {
	let now = chrono::Local::now();
	now.date_naive()
		.and_hms_opt(0, 0, 0)
		.and_then(|t| t.and_local_timezone(chrono::Local).earliest())
		.map(|t| t.timestamp())
		.unwrap_or(0)
}

/// True when the view moved on while a request for `window` and `categories` was in flight.
pub fn selection_moved(window: RevenueWindow, categories: &[String], view: &crate::state::ViewState) -> bool {
	view.revenue_window != window || view.revenue_categories != categories
}

/// Refund state text for a rejected trade fee.
pub fn refund_label(item: &RevenueItem) -> Option<&'static str> {
	if !item.trade_rejected {
		return None;
	}
	Some(match item.refund_status.as_str() {
		"pending" => "Refund pending",
		"succeeded" => "Refunded",
		"failed" => "Refund failed",
		"unknown" => "Refund outcome unknown · check Payments",
		_ => "Rejected",
	})
}

/// A rejected trade fee can be refunded when there is no refund yet or the last one failed.
pub fn can_refund(item: &RevenueItem) -> bool {
	item.category == "trade_fee" && item.trade_rejected && matches!(item.refund_status.as_str(), "" | "failed")
}

/// True when a formatted amount shows no non-zero digit, e.g. 1 msat shown as "$0.00".
pub fn reads_as_zero(text: &str) -> bool {
	!text.chars().any(|c| c.is_ascii_digit() && c != '0')
}

/// Puts a sign on a formatted amount unless it reads as zero.
pub fn with_sign(sign: &str, text: String) -> String {
	if reads_as_zero(&text) { text } else { format!("{sign}{text}") }
}

pub fn signed(ctx: AppCtx, msat: i128) -> String {
	let text = ctx.fmt_msat(msat.unsigned_abs().min(u64::MAX as u128) as u64);
	with_sign(if msat < 0 { "\u{2212}" } else { "" }, text)
}

fn revenue_csv(items: &[RevenueItem]) -> String {
	let mut out = String::from("occurred_at,category,direction,amount_msat,node_id,user_channel_id,payment_id,txid,approximate_time,trade_rejected,refund_status\n");
	for i in items {
		out.push_str(&csv_row(&[
			i.occurred_at.to_string(),
			i.category.clone(),
			i.direction.clone(),
			i.amount_msat.to_string(),
			i.node_id.clone(),
			i.user_channel_id.clone(),
			i.payment_id.clone(),
			i.txid.clone(),
			i.approximate_time.to_string(),
			i.trade_rejected.to_string(),
			i.refund_status.clone(),
		]));
		out.push('\n');
	}
	out
}

#[component]
fn Breakdown(label: String, count: Option<u64>, msat: u64) -> Element {
	let ctx = use_context::<AppCtx>();
	let count = count.map(|n| format!(" · {n}")).unwrap_or_default();
	rsx! {
		Kv { label, span { class: "num", "{ctx.fmt_msat(msat)}" } span { class: "muted small", "{count}" } }
	}
}

#[component]
pub fn Revenue() -> Element {
	let ctx = use_context::<AppCtx>();
	use_hook(move || {
		if ctx.is_connected_peek() && ctx.data.peek().revenue.is_none() {
			actions::fetch_revenue(ctx, false);
		}
	});
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let loading = ctx.busy(Op::GetRevenue);
	let (window, filter) = {
		let view = ctx.view.read();
		(view.revenue_window, view.revenue_categories.clone())
	};
	let mut view = ctx.view;
	let refresh = move |_| actions::fetch_revenue(ctx, false);
	let data = ctx.data.read();
	let Some(resp) = data.revenue.clone() else {
		drop(data);
		return rsx! {
			Card {
				if loading {
					Empty { icon: "coins", title: "Loading...", Spinner { large: true } }
				} else {
					Empty { icon: "coins", title: "No revenue loaded", hint: "Click Refresh to load", RefreshBtn { busy: loading, onclick: refresh } }
				}
			}
		};
	};
	let items = data.revenue_items.clone();
	let has_more = data.revenue_cursor.is_some();
	drop(data);
	let lines = resp.lines;
	let (earned, spent, net) = totals(&lines);
	let spent_unknown = spent_untracked(&resp.untracked);
	let stability = stability_net(&lines);
	let (trade_n, trade_msat) = line(&lines, "trade_fee");
	let (rejected_n, _) = line(&lines, "trade_fee_rejected");
	let (routing_n, routing_msat) = line(&lines, "routing_fee");
	let (_, jit_msat) = line(&lines, "jit_fee");
	let (paid_n, paid_msat) = line(&lines, "stability_out");
	let (got_n, got_msat) = line(&lines, "stability_in");
	let untracked = resp.untracked.clone();
	let spent_line = move |label: &'static str, category: &'static str, help: Option<&'static str>, lines: &[RevenueLine]| {
		if untracked.iter().any(|c| c == category) {
			rsx! { Kv { label, help: HELP_UNTRACKED, span { class: "muted", "not tracked on this node" } } }
		} else {
			let (count, msat) = line(lines, category);
			rsx! { Kv { label, help: help.map(String::from), span { class: "num", "{ctx.fmt_msat(msat)}" } span { class: "muted small", " · {count}" } } }
		}
	};
	let (opens_n, opens_msat) = line(&lines, "jit_open_fee");
	let jit_opens = {
		let each = opens_msat.checked_div(opens_n).map(|m| ctx.fmt_msat(m)).unwrap_or_default();
		let noun = if opens_n == 1 { "open" } else { "opens" };
		rsx! {
			Kv { label: "JIT channel opens", help: HELP_JIT_OPENS,
				span { class: "num", "{ctx.fmt_msat(opens_msat)}" }
				if opens_n > 0 {
					span { class: "muted small", " · {opens_n} {noun} · {each} each" }
				}
			}
		}
	};
	let age = crate::format::now_secs() as i64 - resp.snapshot_at;
	let as_of = crate::ledger::relative_timestamp(resp.snapshot_at * 1000);
	let count = items.len();
	let csv = revenue_csv(&items);
	let export = move |_| actions::save_export(ctx, "revenue.csv", "text/csv", csv.clone(), format!("{count} movements"));
	let mut set_window = move |w: RevenueWindow| {
		view.write().revenue_window = w;
		actions::fetch_revenue(ctx, false);
	};
	let mut toggle = move |category: &'static str| {
		{
			let mut v = view.write();
			match v.revenue_categories.iter().position(|c| c == category) {
				Some(i) => {
					v.revenue_categories.remove(i);
				},
				None => v.revenue_categories.push(category.to_owned()),
			}
		}
		actions::fetch_revenue(ctx, false);
	};
	// Show only rejected trade fees, where the Refund buttons are.
	let show_rejected = move |_| {
		view.write().revenue_categories = vec!["trade_fee_rejected".to_owned()];
		actions::fetch_revenue(ctx, false);
	};
	rsx! {
		div { class: "row between", style: "margin-bottom: 12px;",
			div { class: "seg",
				for w in RevenueWindow::ALL {
					SegBtn { key: "{w.label()}", active: w == window, onclick: move |_| set_window(w), "{w.label()}" }
				}
			}
			div { class: "row", style: "gap: 8px;",
				span { class: if age > 180 { "small pill warning" } else { "small muted" }, "as of {as_of}" }
				RefreshBtn { busy: loading, onclick: refresh }
			}
		}
		if resp.partial {
			div { class: "card inner small", style: "margin-bottom: 12px;", "Totals for this window miss older movements: the daemon keeps a year of routing fees, the newest 50,000 movements and a capped payment scan." }
		}
		div { class: "grid-4",
			Stat { title: "Earned", value: ctx.fmt_msat(earned), sub: if jit_msat > 0 { "Trade, routing and JIT fees" } else { "Trade and routing fees" } }
			Stat { title: "Spent", value: if spent_unknown { format!("\u{2265} {}", ctx.fmt_msat(spent)) } else { ctx.fmt_msat(spent) }, sub: if spent_unknown { "Fees the LSP paid · some not tracked" } else { "Fees the LSP paid" } }
			div { class: "card stat",
				div { class: "stat-title", "Net" InfoTip { text: HELP_NET } }
				div { class: "stat-value", style: if net > 0 && !spent_unknown && !reads_as_zero(&signed(ctx, net)) { "color: var(--green-text);" } else { "" }, "{bounded(ctx, net, spent_unknown)}" }
				div { class: "stat-sub", if spent_unknown { "At most: some fees are not tracked" } else { "Earned minus spent" } }
			}
			Stat { title: "Stability net", help: HELP_STABILITY, value: signed(ctx, stability), sub: "Peg settlements, not revenue" }
		}
		div { class: "grid-3", style: "margin-top: 14px;",
			Card { title: "Earned",
				div { class: "kv",
					Breakdown { label: "Trade fees", count: trade_n, msat: trade_msat }
					if rejected_n > 0 {
						Kv { label: "", button { class: "btn sm ghost", title: "Show the rejected trades, where each fee can be refunded", onclick: show_rejected, "{rejected_n} rejected · show" } }
					}
					Breakdown { label: "Routing fees", count: routing_n, msat: routing_msat }
					if jit_msat > 0 {
						Kv { label: "JIT opening fees", help: HELP_JIT, span { class: "num", "{ctx.fmt_msat(jit_msat)}" } }
					}
				}
			}
			Card { title: "Spent",
				div { class: "kv",
					{jit_opens}
					{spent_line("Channel open & splice fees", "channel_funding_fee", None, &lines)}
					{spent_line("Channel closes", "close_fee", Some(HELP_CLOSE_FEES), &lines)}
					{spent_line("Close fee bumps", "close_fee_bump", None, &lines)}
					{spent_line("Claims & sweeps", "claim_sweep_fee", None, &lines)}
					{spent_line("Other on-chain fees", "onchain_fee", None, &lines)}
					Breakdown { label: "Lightning send fees", msat: line(&lines, "lightning_send_fee").1 }
					Breakdown { label: "Protocol messages", count: line(&lines, "protocol_message").0, msat: line(&lines, "protocol_message").1 }
					Breakdown { label: "Trade fee refunds", count: line(&lines, "trade_fee_refund").0, msat: line(&lines, "trade_fee_refund").1 }
				}
			}
			Card { title: "Stability settlements", help: HELP_STABILITY,
				div { class: "kv",
					Breakdown { label: "Paid to users", count: paid_n, msat: paid_msat }
					Breakdown { label: "Received from users", count: got_n, msat: got_msat }
					Kv { label: "Net", span { class: "num", "{signed(ctx, stability)}" } }
				}
			}
		}
		Card { title: "Activity", class: "flush",
			actions: rsx! {
				button { class: "btn sm", disabled: count == 0 || ctx.busy(Op::Export), onclick: export, Icon { name: "csv", size: 14 } "Export CSV" }
			},
			div { class: "chips", style: "padding: 12px 16px;",
				for category in FILTERS {
					button {
						key: "{category}",
						class: if filter.iter().any(|c| c == category) { "chip on" } else { "chip" },
						onclick: move |_| toggle(category),
						"{category_label(category)}"
					}
				}
			}
			if items.is_empty() {
				Empty { icon: "coins", title: "No movements in this window" }
			} else {
				div { class: "table-wrap",
					table { class: "table", style: "min-width: 720px;",
						thead {
							tr {
								Th { label: "When" }
								Th { label: "Category" }
								Th { label: "User or peer" }
								Th { label: "Amount", class: "right" }
								Th { label: "Payment" }
								Th { label: "" }
							}
						}
						tbody {
							for item in items {
								ActivityRow { key: "{item.key}", item }
							}
						}
					}
				}
				if has_more {
					div { class: "row", style: "padding: 12px 16px;",
						button { class: "btn sm", disabled: loading, onclick: move |_| actions::fetch_revenue(ctx, true), "Load more" }
					}
				}
			}
		}
	}
}

#[component]
fn ActivityRow(item: RevenueItem) -> Element {
	let ctx = use_context::<AppCtx>();
	// 0 is a forward LDK gave no time: shown undated, never as 1970.
	let when = if item.occurred_at == 0 { "undated".to_owned() } else { crate::ledger::relative_timestamp(item.occurred_at * 1000) };
	let approx = if item.approximate_time && item.occurred_at != 0 { "≈ " } else { "" };
	let sign = if item.direction == "out" { "\u{2212}" } else { "+" };
	let id = if item.payment_id.is_empty() { item.txid.clone() } else { item.payment_id.clone() };
	rsx! {
		tr {
			td { class: "small", title: "{crate::ledger::exact_timestamp(item.occurred_at * 1000)}", "{approx}{when}" }
			td {
				Pill { tone: category_tone(&item.category), "{category_label(&item.category)}" }
				if let Some(state) = refund_label(&item) {
					span { class: "small muted", style: "margin-left: 6px;", "{state}" }
				}
			}
			td {
				if item.node_id.is_empty() {
					span { class: "faint", "—" }
				} else {
					Peer { node_id: item.node_id.clone() }
				}
			}
			td { class: "right num", "{with_sign(sign, ctx.fmt_msat(item.amount_msat))}" }
			td {
				if id.is_empty() { span { class: "faint", "—" } } else { IdCopy { value: id } }
			}
			td { class: "right",
				if can_refund(&item) {
					button {
						class: "btn sm",
						onclick: {
							let item = item.clone();
							move |e: MouseEvent| {
								e.stop_propagation();
								let mut forms = ctx.forms;
								forms.write().refund_trade_fee = RefundTradeFeeForm { trade_payment_id: item.payment_id.clone(), amount_msat: item.amount_msat, node_id: item.node_id.clone() };
								open_dialog(ctx, Dialog::RefundTradeFee);
							}
						},
						"Refund"
					}
				}
			}
		}
	}
}

#[component]
pub fn RefundTradeFeeDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().refund_trade_fee.clone();
	let pending = ctx.busy(Op::RefundTradeFee);
	let who = ctx.data.read().alias(&form.node_id).unwrap_or_else(|| crate::format::truncate_id(&form.node_id, 8, 8));
	let amount = ctx.fmt_msat(form.amount_msat);
	let mut cancel = move || {
		forms.write().refund_trade_fee = Default::default();
		close_dialog(ctx);
	};
	rsx! {
		Modal {
			title: "Refund trade fee",
			sub: "Send a rejected trade's fee back to the user",
			icon: rsx! { Bubble { icon: "arrow-up-right", tone: "orange" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				button { class: "btn primary", disabled: pending, onclick: move |_| actions::refund_trade_fee(ctx),
					if pending { Spinner {} }
					"Send refund"
				}
			},
			p { "Send {amount} back to {who}?" }
			span { class: "small muted", "It arrives as an ordinary payment in the user's wallet. The daemon sends it once; a failed refund can be retried." }
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sc_rest_client::sc_protos::revenue::RevenueLine;

	fn line(category: &str, count: u64, total_msat: u64) -> RevenueLine {
		RevenueLine { category: category.into(), direction: String::new(), count, total_msat }
	}

	#[test]
	fn totals_add_earned_and_spent_and_ignore_the_rejected_subset() {
		let lines = vec![
			line("trade_fee", 2, 3_000_000),
			line("trade_fee_rejected", 1, 2_000_000),
			line("routing_fee", 3, 3_000),
			line("channel_funding_fee", 1, 500_000),
			line("protocol_message", 10, 10),
			line("jit_open_fee", 1, 287_000),
			line("stability_in", 1, 40_000_000),
			line("stability_out", 1, 100_000_000),
		];
		assert_eq!(totals(&lines), (3_003_000, 787_010, 2_215_990));
		assert_eq!(stability_net(&lines), -60_000_000);
	}

	#[test]
	fn an_untracked_fee_category_turns_net_into_a_ceiling() {
		assert!(spent_untracked(&["onchain_fee".to_string()]), "on-chain fees exist but the node cannot report them");
		assert!(!spent_untracked(&["stability_in".to_string()]), "only spent categories matter");
	}

	#[test]
	fn windows_start_where_the_operator_expects() {
		assert_eq!(window_since(RevenueWindow::Today, 1_000_000, 990_000), 990_000);
		assert_eq!(window_since(RevenueWindow::Week, 1_000_000, 0), 1_000_000 - 7 * 86_400);
		assert_eq!(window_since(RevenueWindow::Month, 1_000_000, 0), 1_000_000 - 30 * 86_400);
		assert_eq!(window_since(RevenueWindow::All, 1_000_000, 0), 0);
	}

	#[test]
	fn categories_read_as_words_and_unknown_ones_pass_through() {
		assert_eq!(category_label("trade_fee"), "Trade fee");
		assert_eq!(category_label("channel_funding_fee"), "Channel open");
		assert_eq!(category_label("trade_fee_rejected"), "Rejected trades");
		assert_eq!(category_label("something_new"), "something_new");
	}

	#[test]
	fn the_forwarded_page_calls_its_total_routing_fees() {
		let source = include_str!("forwarded_payments.rs");
		assert!(source.contains("title: \"Routing fees\""));
		assert!(!source.contains("title: \"Revenue\""));
	}

	#[test]
	fn only_rejected_trade_fees_without_a_live_refund_offer_the_button() {
		let base = RevenueItem { category: "trade_fee".into(), trade_rejected: true, ..Default::default() };
		assert!(can_refund(&base));
		assert!(can_refund(&RevenueItem { refund_status: "failed".into(), ..base.clone() }));
		for status in ["pending", "succeeded", "unknown"] {
			assert!(!can_refund(&RevenueItem { refund_status: status.into(), ..base.clone() }), "{status}");
		}
		assert!(!can_refund(&RevenueItem { trade_rejected: false, ..base.clone() }));
		assert_eq!(refund_label(&RevenueItem { refund_status: "unknown".into(), ..base.clone() }), Some("Refund outcome unknown · check Payments"));
	}

	#[test]
	fn a_response_for_an_older_selection_is_recognised() {
		let mut view = crate::state::ViewState::default();
		assert!(!selection_moved(RevenueWindow::Week, &[], &view));
		view.revenue_window = RevenueWindow::Today;
		assert!(selection_moved(RevenueWindow::Week, &[], &view), "window changed while in flight");
		view.revenue_window = RevenueWindow::Week;
		view.revenue_categories = vec!["trade_fee".into()];
		assert!(selection_moved(RevenueWindow::Week, &[], &view), "filter changed while in flight");
	}

	#[test]
	fn amounts_that_round_to_zero_carry_no_sign() {
		assert_eq!(with_sign("\u{2212}", "$0.00".into()), "$0.00");
		assert_eq!(with_sign("+", "0 sats".into()), "0 sats");
		assert_eq!(with_sign("\u{2212}", "$0.01".into()), "\u{2212}$0.01");
		assert_eq!(with_sign("+", "0.001 sats".into()), "+0.001 sats");
		assert!(reads_as_zero("0.00000000 BTC") && !reads_as_zero("$0.04"));
	}
}
