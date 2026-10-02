use dioxus::prelude::*;

use std::collections::HashMap;

use sc_rest_client::sc_protos::stable::StableChannelInfo;

use crate::actions;
use crate::format::{format_sats, format_usd, unit_label};
use crate::health::{has_stable_position, settle_threshold_usd, settlement, stable_drift, Drift, Settlement};
use crate::state::{AppCtx, ChannelSortColumn, ChannelStatusFilter, Dialog, Drawer, Op};
use crate::ui::widgets::{
	Amount, Bubble, Card, Check, CopyBtn, Empty, Field, Gate, Hover, Icon, IdCopy, Kv, LiquidityBar, Modal, Peer, Pill,
	RefreshBtn, SidePanel, SortTh, Spinner, Stat, TextInput, Th,
};
use crate::ui::{close_dialog, close_drawer, open_dialog, open_drawer};

const HELP_CHANNEL_ID: &str = "The Lightning channel identifier for this channel.";
const HELP_USER_CHANNEL_ID: &str =
	"Application-level identifier associated with the channel by the opener or server.";
const HELP_COUNTERPARTY: &str = "The node public key of the peer on the other side of the channel.";
const HELP_FUNDING_TX: &str = "The Bitcoin transaction that funded the channel.";
const HELP_CAPACITY: &str = "The total channel size currently tracked for this channel.";
const HELP_OUTBOUND: &str =
	"Amount this node can currently send through the channel, subject to reserves, pending HTLCs, and channel limits.";
const HELP_INBOUND: &str =
	"Amount this node can currently receive through the channel, subject to counterparty liquidity and channel limits.";
const HELP_LIQUIDITY: &str = "Visual split of outbound and inbound channel liquidity.";
const HELP_STATUS: &str =
	"Active: ready and usable for payments. Inactive: open but not usable, usually because the peer is offline. Opening: waiting for the funding transaction to confirm. Closing: a cooperative close is under way, with its stage. Splicing: a negotiated splice is waiting to confirm.";
const HELP_SCID: &str = "The channel's position on-chain (block, transaction, output), used by the network to route through it.";
const HELP_SCID_ALIAS: &str = "Random stand-ins for the short channel id: outbound is ours for routes, inbound is the peer's for invoice route hints.";
const HELP_HTLC_MAX: &str = "The largest single payment this channel can currently receive from the peer.";
const HELP_HTLC_MIN: &str = "The smallest payment the peer will send through this channel.";
const HELP_RESERVE_TYPE: &str = "Adaptive keeps a per-channel on-chain reserve for fee-bumping a force close. No reserve means the peer is trusted. Legacy is a pre-anchor channel.";
const HELP_CLOSE_STAGE: &str = "Where the cooperative close is: HTLCs are resolved, the closing fee is negotiated, then the close transaction is broadcast.";
const HELP_STABLE: &str = "USD target of the stable position on this channel, its role, and how far the backing has drifted.";
const HELP_DRIFT: &str =
	"Backing valued at the latest price, compared with the USD target: green above it, red below.";
const HELP_SETTLES_AT: &str =
	"The daemon pays or asks for a stability payment once drift reaches both $0.25 and 0.1% of the target, whichever is larger.";
const HELP_NODE_PUBKEY: &str = "The counterparty node public key to connect to or open a channel with.";
const HELP_PEER_ADDRESS: &str = "Network address for the Lightning peer, typically host:port.";
const HELP_CHANNEL_AMOUNT: &str = "The amount of on-chain bitcoin committed to the channel at open.";
const HELP_PUSH_AMOUNT: &str =
	"Funds given to the counterparty at channel open. This reduces this node's initial local balance.";
const HELP_ANNOUNCE_CHANNEL: &str =
	"Whether to advertise the channel publicly so it can be used for network routing.";
const HELP_FEE_PROPORTIONAL: &str =
	"The proportional forwarding fee for this channel, in millionths of the forwarded amount.";
const HELP_FEE_BASE: &str =
	"The fixed forwarding fee charged for routed payments through this channel, in millisatoshis.";
const HELP_CLTV_EXPIRY_DELTA: &str =
	"Extra blocks this channel requires on forwarded HTLCs so there is time to resolve them on-chain if needed.";
const HELP_CLOSE_CHANNEL: &str =
	"Cooperatively close the channel when possible and return funds on-chain after confirmation.";
const HELP_SPLICE_IN: &str = "Add on-chain funds to an existing channel without closing and reopening it.";
const HELP_SPLICE_OUT: &str = "Move funds from a channel to an on-chain address without fully closing the channel.";
const HELP_ONCHAIN_ADDRESS: &str =
	"The Bitcoin address for the current network. Verify it belongs to the intended recipient and network before sending.";

// Per-row snapshot extracted from state.channels.
#[derive(Clone, PartialEq)]
struct ChannelRow {
	channel_id: String,
	counterparty_node_id: String,
	alias: Option<String>,
	user_channel_id: String,
	channel_value_sats: u64,
	outbound_capacity_msat: u64,
	inbound_capacity_msat: u64,
	is_channel_ready: bool,
	is_usable: bool,
	shutdown_state: Option<i32>,
	splicing: bool,
	stable: Option<StableSummary>,
}

/// Stable-channel view of a channel, joined in by user_channel_id.
#[derive(Clone, PartialEq)]
struct StableSummary {
	expected_usd: f64,
	is_stable_receiver: bool,
	drift: Option<Drift>,
}

fn status_label(filter: ChannelStatusFilter) -> &'static str {
	match filter {
		ChannelStatusFilter::All => "All",
		ChannelStatusFilter::Ready => "Ready",
		ChannelStatusFilter::Usable => "Usable",
		ChannelStatusFilter::Pending => "Pending",
	}
}

/// Single status for a channel: a close or splice under way first, else Active, Inactive or Opening.
fn channel_state(ready: bool, usable: bool, shutdown_state: Option<i32>, splicing: bool) -> (String, &'static str) {
	if let Some(stage) = crate::dashboard::close_stage(shutdown_state) {
		return (format!("Closing · {}", crate::dashboard::CLOSE_STAGES[stage]), "warning");
	}
	if splicing {
		return ("Splicing".to_string(), "info");
	}
	let (text, tone) = match (ready, usable) {
		(_, true) => ("Active", "success"),
		(true, false) => ("Inactive", "danger"),
		(false, false) => ("Opening", "warning"),
	};
	(text.to_string(), tone)
}

fn alias_text(alias: Option<u64>) -> String {
	alias.map(|a| a.to_string()).unwrap_or_else(|| "—".to_string())
}

/// Channels whose negotiated splice the feed has not seen confirmed; they show as "Splicing".
fn splicing_channels(data: &crate::state::Data) -> std::collections::HashSet<String> {
	let Some(activity) = data.activity.as_ref() else { return Default::default() };
	let feed = crate::dashboard::feed(&activity.events);
	let dctx = crate::dashboard::Ctx {
		now: 0,
		best_height: None,
		channels: None,
		balances: None,
		stable: None,
		feed: Some(&feed),
		feed_has_more: data.activity_cursor.is_some(),
		aliases: &data.aliases,
	};
	crate::dashboard::splicing(&dctx).into_iter().map(|(uid, _)| uid).collect()
}

/// Short channel id as block x tx x output.
fn format_scid(scid: u64) -> String {
	format!("{}x{}x{}", scid >> 40, (scid >> 16) & 0xff_ffff, scid & 0xffff)
}

/// LDK's reserve type for the side panel; None for values this build does not know.
fn reserve_type_label(reserve_type: Option<i32>) -> Option<&'static str> {
	match reserve_type? {
		1 => Some("Adaptive"),
		2 => Some("No reserve (trusted peer)"),
		3 => Some("Legacy"),
		_ => None,
	}
}

/// Pill text and tone for a stable drift.
pub fn drift_pill(drift: &Drift) -> (String, &'static str) {
	// Signed drift from the USD target: green above, red below, grey when it rounds to zero.
	let percent = format!("{:.2}", drift.percent);
	if percent == "0.00" {
		("0.00%".to_string(), "muted")
	} else if drift.usd > 0.0 {
		(format!("+{percent}%"), "success")
	} else {
		(format!("\u{2212}{percent}%"), "danger")
	}
}

/// Signed dollars with a true minus sign, e.g. "−$0.30".
fn signed_usd(usd: f64) -> String {
	crate::ui::revenue::with_sign(if usd < 0.0 { "\u{2212}" } else { "+" }, format_usd(usd.abs()))
}

#[component]
pub fn Channels() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let loading = ctx.busy(Op::Channels);
	let price = ctx.price_value();
	let data = ctx.data.read();
	let stable_by_id: HashMap<String, StableSummary> = data
		.stable_channels
		.as_ref()
		.map(|s| {
			s.channels
				.iter()
				.filter(|sc| has_stable_position(sc))
				.map(|sc| {
					(sc.user_channel_id.clone(), StableSummary {
						expected_usd: sc.expected_usd,
						is_stable_receiver: sc.is_stable_receiver,
						drift: stable_drift(sc, price),
					})
				})
				.collect()
		})
		.unwrap_or_default();
	let splicing = splicing_channels(&data);
	let rows: Option<Vec<ChannelRow>> = data.channels.as_ref().map(|resp| {
		resp.channels
			.iter()
			.map(|ch| ChannelRow {
				channel_id: ch.channel_id.clone(),
				counterparty_node_id: ch.counterparty_node_id.clone(),
				alias: data.alias(&ch.counterparty_node_id),
				user_channel_id: ch.user_channel_id.clone(),
				channel_value_sats: ch.channel_value_sats,
				outbound_capacity_msat: ch.outbound_capacity_msat,
				inbound_capacity_msat: ch.inbound_capacity_msat,
				is_channel_ready: ch.is_channel_ready,
				is_usable: ch.is_usable,
				shutdown_state: ch.channel_shutdown_state,
				splicing: splicing.contains(&ch.user_channel_id),
				stable: stable_by_id.get(&ch.user_channel_id).cloned(),
			})
			.collect()
	});
	drop(data);
	let mut view = ctx.view;
	let v = view.read();
	let filter = v.channel_filter.clone();
	let status_filter = v.channel_status;
	let sort = v.channel_sort;
	drop(v);

	let Some(rows) = rows else {
		return rsx! {
			Card {
				if loading {
					Empty { icon: "link", title: "Loading channels...", Spinner { large: true } }
				} else {
					Empty { icon: "link", title: "No channel data available", hint: "Click Refresh to fetch.",
						RefreshBtn { busy: loading, onclick: move |_| actions::fetch_channels(ctx) }
					}
				}
			}
		};
	};

	let ready = rows.iter().filter(|r| r.is_channel_ready).count();
	let usable = rows.iter().filter(|r| r.is_usable).count();
	let capacity: u64 = rows.iter().map(|r| r.channel_value_sats).sum();
	let outbound: u64 = rows.iter().map(|r| r.outbound_capacity_msat).sum();
	let inbound: u64 = rows.iter().map(|r| r.inbound_capacity_msat).sum();

	let needle = filter.trim().to_lowercase();
	let mut view_rows: Vec<&ChannelRow> = rows
		.iter()
		.filter(|r| {
			let matches_text = needle.is_empty()
				|| r.channel_id.to_lowercase().contains(&needle)
				|| r.user_channel_id.to_lowercase().contains(&needle)
				|| r.counterparty_node_id.to_lowercase().contains(&needle)
				|| r.alias.as_deref().map(|a| a.to_lowercase().contains(&needle)).unwrap_or(false);
			let matches_status = match status_filter {
				ChannelStatusFilter::Ready => r.is_channel_ready,
				ChannelStatusFilter::Usable => r.is_usable,
				ChannelStatusFilter::Pending => !r.is_channel_ready,
				ChannelStatusFilter::All => true,
			};
			matches_text && matches_status
		})
		.collect();
	// Sort the view (display ordering only; the underlying list is untouched).
	view_rows.sort_by(|ra, rb| {
		let ord = match sort.column {
			ChannelSortColumn::Outbound => ra.outbound_capacity_msat.cmp(&rb.outbound_capacity_msat),
			ChannelSortColumn::Inbound => ra.inbound_capacity_msat.cmp(&rb.inbound_capacity_msat),
			ChannelSortColumn::Capacity => ra.channel_value_sats.cmp(&rb.channel_value_sats),
		};
		if sort.descending {
			ord.reverse()
		} else {
			ord
		}
	});
	let shown = view_rows.len();
	let total = rows.len();
	let view_rows: Vec<ChannelRow> = view_rows.into_iter().cloned().collect();
	let mut sort_by = move |column: ChannelSortColumn| {
		let current = view.peek().channel_sort;
		view.write().channel_sort = current.toggled(column);
	};

	rsx! {
		div { class: "grid-3",
			Stat { title: "Channels", value: format!("{total}"), sub: format!("{ready} ready · {usable} usable · {} capacity", ctx.fmt_sats(capacity)) }
			Stat { title: "Outbound", help: HELP_OUTBOUND, value: ctx.fmt_msat(outbound), sub: "Can send" }
			Stat { title: "Inbound", help: HELP_INBOUND, value: ctx.fmt_msat(inbound), sub: "Can receive" }
		}
		Card { class: "flush",
			div { class: "toolbar",
				div { class: "search",
					Icon { name: "search", size: 15 }
					TextInput { value: filter, small: true, placeholder: "peer name, node id or channel id", oninput: move |v| view.write().channel_filter = v }
				}
				select {
					class: "select sm",
					style: "width: 140px;",
					"aria-label": "Status filter",
					onchange: move |e| {
						view.write().channel_status = match e.value().as_str() {
							"Ready" => ChannelStatusFilter::Ready,
							"Usable" => ChannelStatusFilter::Usable,
							"Pending" => ChannelStatusFilter::Pending,
							_ => ChannelStatusFilter::All,
						};
					},
					for option in [ChannelStatusFilter::All, ChannelStatusFilter::Ready, ChannelStatusFilter::Usable, ChannelStatusFilter::Pending] {
						option { value: status_label(option), selected: option == status_filter, "{status_label(option)}" }
					}
				}
				span { class: "count", "{shown} of {total} channel(s)" }
				div { class: "row", style: "margin-left: auto;",
					RefreshBtn { busy: loading, onclick: move |_| actions::fetch_channels(ctx), op: Op::Channels }
					button { class: "btn sm accent", onclick: move |_| open_dialog(ctx, Dialog::OpenChannel), Icon { name: "plus", size: 14 } "Open Channel" }
				}
			}
			if rows.is_empty() {
				Empty { icon: "link", title: "No channels found.", hint: "Open a channel to start routing payments." }
			} else {
				div { class: "table-wrap",
					table { class: "table clickable", style: "min-width: 960px;",
						thead {
							tr {
								Th { label: "Peer", help: HELP_COUNTERPARTY }
								SortTh { label: "Capacity", class: "right", help: HELP_CAPACITY, active: sort.column == ChannelSortColumn::Capacity, descending: sort.descending, onclick: move |_| sort_by(ChannelSortColumn::Capacity) }
								SortTh { label: "Outbound", class: "right", help: HELP_OUTBOUND, active: sort.column == ChannelSortColumn::Outbound, descending: sort.descending, onclick: move |_| sort_by(ChannelSortColumn::Outbound) }
								SortTh { label: "Inbound", class: "right", help: HELP_INBOUND, active: sort.column == ChannelSortColumn::Inbound, descending: sort.descending, onclick: move |_| sort_by(ChannelSortColumn::Inbound) }
								Th { label: "Liquidity", help: HELP_LIQUIDITY }
								Th { label: "Status", help: HELP_STATUS }
								Th { label: "Stable", help: HELP_STABLE }
								th { "" }
							}
						}
						tbody {
							for ch in view_rows {
								ChannelRowView { key: "{ch.user_channel_id}-{ch.channel_id}", row: ch }
							}
						}
					}
				}
			}
		}
	}
}

#[component]
fn ChannelRowView(row: ChannelRow) -> Element {
	let ctx = use_context::<AppCtx>();
	let total = row.outbound_capacity_msat + row.inbound_capacity_msat;
	let frac = if total == 0 { 0.0 } else { row.outbound_capacity_msat as f64 / total as f64 };
	let hover =
		format!("out {} / in {}", ctx.fmt_msat(row.outbound_capacity_msat), ctx.fmt_msat(row.inbound_capacity_msat));
	let (state_text, state_tone) = channel_state(row.is_channel_ready, row.is_usable, row.shutdown_state, row.splicing);
	let uid = row.user_channel_id.clone();
	rsx! {
		tr { onclick: move |_| open_drawer(ctx, Drawer::Channel(uid.clone())),
			td { Peer { node_id: row.counterparty_node_id.clone() } }
			td { class: "right", Amount { msat: row.channel_value_sats * 1000 } }
			td { class: "right", Amount { msat: row.outbound_capacity_msat } }
			td { class: "right", Amount { msat: row.inbound_capacity_msat } }
			td { Hover { tip: hover, LiquidityBar { frac } } }
			td { Pill { tone: state_tone, "{state_text}" } }
			td {
				match row.stable.clone() {
					Some(stable) => rsx! { StableCell { stable } },
					None => rsx! { span { class: "faint", "—" } },
				}
			}
			td { class: "right", RowMenu { user_channel_id: row.user_channel_id.clone(), counterparty: row.counterparty_node_id.clone() } }
		}
	}
}

#[component]
fn StableCell(stable: StableSummary) -> Element {
	let role = if stable.is_stable_receiver { "Receiver" } else { "Provider" };
	rsx! {
		span { class: "amount",
			span { class: "num", style: "color: var(--green-text); font-weight: 700;", "{format_usd(stable.expected_usd)}" }
			span { class: "amount-sub",
				"{role}"
				if let Some(drift) = stable.drift {
					{
						let (text, tone) = drift_pill(&drift);
						rsx! { " · " span { class: "pill {tone}", style: "height: 18px; padding: 0 6px;", "{text}" } }
					}
				}
			}
		}
	}
}

/// Prefill the form behind `which` with this channel's ids and open its dialog.
fn open_channel_action(ctx: AppCtx, which: Dialog, user_channel_id: String, counterparty: String) {
	let mut forms = ctx.forms;
	let mut view = ctx.view;
	{
		let mut f = forms.write();
		match which {
			Dialog::CloseChannel => {
				f.close_channel.user_channel_id = user_channel_id;
				f.close_channel.counterparty_node_id = counterparty;
				view.write().force_close_confirm = false;
			},
			Dialog::SpliceIn => {
				f.splice_in.user_channel_id = user_channel_id;
				f.splice_in.counterparty_node_id = counterparty;
			},
			Dialog::SpliceOut => {
				f.splice_out.user_channel_id = user_channel_id;
				f.splice_out.counterparty_node_id = counterparty;
				view.write().splice_out_confirm = false;
			},
			_ => {
				f.update_channel_config.user_channel_id = user_channel_id;
				f.update_channel_config.counterparty_node_id = counterparty;
			},
		}
	}
	close_drawer(ctx);
	open_dialog(ctx, which);
}

const CHANNEL_ACTIONS: [(Dialog, &str, &str, &str); 4] = [
	(Dialog::SpliceIn, "merge", "Splice+ (add funds)", ""),
	(Dialog::SpliceOut, "split", "Splice- (remove funds)", ""),
	(Dialog::UpdateChannelConfig, "sliders", "Config", ""),
	(Dialog::CloseChannel, "x", "Close", "danger"),
];

/// Collapsed row actions menu.
#[component]
fn RowMenu(user_channel_id: String, counterparty: String) -> Element {
	let ctx = use_context::<AppCtx>();
	let mut open = use_signal(|| false);
	rsx! {
		// Clicks inside the menu must not open the row's side panel; Esc closes it.
		div {
			class: "menu-anchor",
			onclick: move |e| e.stop_propagation(),
			onkeydown: move |e| {
				if e.key() == Key::Escape {
					open.set(false);
				}
			},
			button { class: "icon-btn", "aria-label": "Channel actions", "aria-haspopup": "menu", onclick: move |_| open.toggle(), Icon { name: "more", size: 18 } }
			if open() {
				div { class: "menu-catcher", onclick: move |_| open.set(false) }
				div { class: "menu", role: "menu",
					for (which, icon, label, class) in CHANNEL_ACTIONS {
						button {
							key: "{label}",
							role: "menuitem",
							class,
							onclick: {
								let user_channel_id = user_channel_id.clone();
								let counterparty = counterparty.clone();
								move |_| {
									open.set(false);
									open_channel_action(ctx, which, user_channel_id.clone(), counterparty.clone());
								}
							},
							Icon { name: icon, size: 16 }
							"{label}"
						}
					}
				}
			}
		}
	}
}

/// Everything known about one channel, keyed by its splice-stable user_channel_id.
#[component]
pub fn ChannelDrawer(user_channel_id: String) -> Element {
	let ctx = use_context::<AppCtx>();
	let price = ctx.price_value();
	let data = ctx.data.read();
	let channel = data
		.channels
		.as_ref()
		.and_then(|c| c.channels.iter().find(|ch| ch.user_channel_id == user_channel_id).cloned());
	let stable = data
		.stable_channels
		.as_ref()
		.and_then(|s| s.channels.iter().find(|sc| sc.user_channel_id == user_channel_id).cloned());
	let alias = channel.as_ref().and_then(|ch| data.alias(&ch.counterparty_node_id));
	let splicing = splicing_channels(&data).contains(&user_channel_id);
	drop(data);
	let title = alias.unwrap_or_else(|| "Channel".to_string());
	let Some(ch) = channel else {
		return rsx! {
			SidePanel { title, onclose: move |_| close_drawer(ctx),
				Empty { icon: "link", title: "Channel not found", hint: "It may have closed since the list was loaded." }
			}
		};
	};
	let (state_text, state_tone) = channel_state(ch.is_channel_ready, ch.is_usable, ch.channel_shutdown_state, splicing);
	let close_stage = crate::dashboard::close_stage(ch.channel_shutdown_state);
	let total = ch.outbound_capacity_msat + ch.inbound_capacity_msat;
	let frac = if total == 0 { 0.0 } else { ch.outbound_capacity_msat as f64 / total as f64 };
	let yes_no = |b: bool| if b { "Yes" } else { "No" };
	let config = ch.channel_config.clone().unwrap_or_default();
	let fee_line = |ppm: Option<u32>, base: Option<u32>, cltv: Option<u32>| {
		format!(
			"{} ppm · {} msat base · CLTV {}",
			ppm.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
			base.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
			cltv.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
		)
	};
	let uid = ch.user_channel_id.clone();
	let counterparty = ch.counterparty_node_id.clone();
	let actions_row = rsx! {
		for (which, icon, label, class) in CHANNEL_ACTIONS {
			button {
				key: "{label}",
				class: "btn sm {class}",
				onclick: {
					let uid = uid.clone();
					let counterparty = counterparty.clone();
					move |_| open_channel_action(ctx, which, uid.clone(), counterparty.clone())
				},
				Icon { name: icon, size: 14 }
				"{label}"
			}
		}
	};
	rsx! {
		SidePanel {
			title,
			sub: crate::format::truncate_id(&ch.counterparty_node_id, 10, 10),
			icon: rsx! { Bubble { icon: "link", tone: "blue" } },
			onclose: move |_| close_drawer(ctx),
			footer: actions_row,
			div { class: "drawer-hero",
				div { class: "row between",
					span { class: "hero-amount", Amount { msat: ch.channel_value_sats * 1000, strong: true } }
					Pill { tone: state_tone, "{state_text}" }
				}
				div { class: "liq wide", div { class: "out", style: "width: {(frac * 100.0).round()}%;" } }
				div { class: "row between small",
					span { class: "legend", i { style: "background: var(--orange);" } "Outbound {ctx.fmt_msat(ch.outbound_capacity_msat)}" }
					span { class: "legend", i { style: "background: var(--inbound);" } "Inbound {ctx.fmt_msat(ch.inbound_capacity_msat)}" }
				}
			}
			if let Some(sc) = stable {
				StableSection { sc, price }
			}
			div { class: "card inner",
				div { class: "field-label", style: "margin-bottom: 4px;", "Identifiers" }
				div { class: "kv",
					Kv { label: "Channel ID", help: HELP_CHANNEL_ID, IdCopy { value: ch.channel_id.clone(), head: 10, tail: 10 } }
					Kv { label: "User Channel ID", help: HELP_USER_CHANNEL_ID, IdCopy { value: ch.user_channel_id.clone(), head: 10, tail: 10 } }
					Kv { label: "Counterparty", help: HELP_COUNTERPARTY, IdCopy { value: ch.counterparty_node_id.clone(), head: 10, tail: 10 } }
					if let Some(txo) = ch.funding_txo.clone() {
						Kv { label: "Funding Tx", help: HELP_FUNDING_TX, IdCopy { value: txo.txid.clone(), head: 10, tail: 10 } span { class: "muted small", ":{txo.vout}" } }
					}
					// Fields an older LDK Server leaves unset render nothing.
					if let Some(scid) = ch.short_channel_id {
						Kv { label: "Short channel ID", help: HELP_SCID, span { class: "mono", "{format_scid(scid)}" } CopyBtn { value: scid.to_string() } }
					}
					if ch.outbound_scid_alias.is_some() || ch.inbound_scid_alias.is_some() {
						Kv { label: "SCID aliases", help: HELP_SCID_ALIAS,
							span { class: "mono small", "out {alias_text(ch.outbound_scid_alias)}" }
							span { class: "mono small", "in {alias_text(ch.inbound_scid_alias)}" }
						}
					}
				}
			}
			div { class: "card inner",
				div { class: "field-label", style: "margin-bottom: 4px;", "Details" }
				div { class: "kv",
					Kv { label: "Opened by", if ch.is_outbound { "This node" } else { "Peer" } }
					Kv { label: "Announced", "{yes_no(ch.is_announced)}" }
					if let (Some(have), Some(need)) = (ch.confirmations, ch.confirmations_required) {
						Kv { label: "Confirmations", "{have} / {need}" }
					}
					Kv { label: "Fee rate", "{ch.feerate_sat_per_1000_weight} sat/kw" }
					if let Some(reserve) = ch.unspendable_punishment_reserve {
						Kv { label: "Our reserve", Amount { msat: reserve * 1000 } }
					}
					Kv { label: "Peer reserve", Amount { msat: ch.counterparty_unspendable_punishment_reserve * 1000 } }
					Kv { label: "Next HTLC limit", Amount { msat: ch.next_outbound_htlc_limit_msat } }
					if let Some(max) = ch.inbound_htlc_maximum_msat {
						Kv { label: "Largest payment it can receive", help: HELP_HTLC_MAX, Amount { msat: max } }
					}
					if ch.inbound_htlc_minimum_msat > 1 {
						Kv { label: "Smallest payment it accepts", help: HELP_HTLC_MIN, Amount { msat: ch.inbound_htlc_minimum_msat } }
					}
					if let Some(reserve) = reserve_type_label(ch.reserve_type) {
						Kv { label: "Reserve type", help: HELP_RESERVE_TYPE, "{reserve}" }
					}
					if let Some(stage) = close_stage {
						Kv { label: "Close stage", help: HELP_CLOSE_STAGE, Pill { tone: "warning", "{crate::dashboard::CLOSE_STAGES[stage]}" } }
					}
					Kv { label: "Our fees", help: HELP_FEE_PROPORTIONAL,
						"{fee_line(config.forwarding_fee_proportional_millionths, config.forwarding_fee_base_msat, config.cltv_expiry_delta)}"
					}
					Kv { label: "Peer fees",
						"{fee_line(ch.counterparty_forwarding_info_fee_proportional_millionths, ch.counterparty_forwarding_info_fee_base_msat, ch.counterparty_forwarding_info_cltv_expiry_delta)}"
					}
				}
			}
		}
	}
}

/// Side-panel stand-in for a channel with no USD target, with a way to set one.
#[component]
fn NoStablePosition(sc: StableChannelInfo) -> Element {
	let ctx = use_context::<AppCtx>();
	rsx! {
		div { class: "card inner",
			div { class: "row between",
				span { class: "field-label", "Stable position" }
				button { class: "btn sm", onclick: move |_| {
					let mut forms = ctx.forms;
					{
						let mut f = forms.write();
						f.edit_stable_channel.channel_id = sc.channel_id.clone();
						f.edit_stable_channel.expected_usd = String::new();
						f.edit_stable_channel.note = sc.note.clone();
					}
					close_drawer(ctx);
					let mut nav = ctx.nav;
					nav.write().active_tab = crate::state::ActiveTab::StableChannels;
				}, Icon { name: "edit", size: 14 } "Set target" }
			}
			div { class: "muted small", "No stable position. This channel holds only bitcoin, so the LSP sends it no balance syncs." }
		}
	}
}

/// Stable position of a channel inside its side panel.
#[component]
fn StableSection(sc: StableChannelInfo, price: Option<f64>) -> Element {
	let ctx = use_context::<AppCtx>();
	if !has_stable_position(&sc) {
		return rsx! { NoStablePosition { sc } };
	}
	let drift = stable_drift(&sc, price);
	let valued_at = if sc.latest_price > 0.0 { Some(sc.latest_price) } else { price };
	let uid = sc.user_channel_id.clone();
	let edit = sc.clone();
	let role = if sc.is_stable_receiver { "Receiver" } else { "Provider" };
	rsx! {
		div { class: "card inner",
			div { class: "row between", style: "margin-bottom: 4px;",
				span { class: "field-label", "Stable position" }
				div { class: "row", style: "gap: 6px;",
					button { class: "btn sm", onclick: move |_| {
						close_drawer(ctx);
						actions::open_channel_ledger(ctx, uid.clone());
					}, Icon { name: "list", size: 14 } "Ledger" }
					button { class: "btn sm", onclick: move |_| {
						let mut forms = ctx.forms;
						{
							let mut f = forms.write();
							f.edit_stable_channel.channel_id = edit.channel_id.clone();
							f.edit_stable_channel.expected_usd = format!("{:.2}", edit.expected_usd);
							f.edit_stable_channel.note = edit.note.clone();
						}
						close_drawer(ctx);
						let mut nav = ctx.nav;
						nav.write().active_tab = crate::state::ActiveTab::StableChannels;
					}, Icon { name: "edit", size: 14 } "Edit target" }
				}
			}
			div { class: "kv",
				Kv { label: "Target", span { class: "num strong", style: "color: var(--green-text);", "{format_usd(sc.expected_usd)}" } }
				Kv { label: "Backing", Amount { msat: sc.expected_msats } }
				if let Some(d) = drift {
					{
						let (text, tone) = drift_pill(&d);
						rsx! {
							Kv { label: "Value now", help: HELP_DRIFT,
								span { class: "num", "{format_usd(d.value_usd)}" }
								Pill { tone, "{text}" }
							}
							Kv { label: "Drift", "{signed_usd(d.usd)}" }
							Kv { label: "Settles at", help: HELP_SETTLES_AT, "±{format_usd(settle_threshold_usd(sc.expected_usd))}" }
							if let Some(px) = valued_at {
								Kv { label: "Status",
									match settlement(&d, sc.expected_usd, px) {
										Settlement::NotYet { headroom_usd } => rsx! { "No settlement yet · {format_usd(headroom_usd)} more drift before one" },
										Settlement::LspPays { sats } => rsx! { "Settlement due · LSP sends ≈ {format_sats(sats)} sats to the user" },
										Settlement::UserPays { sats } => rsx! { "Settlement due · user's wallet sends ≈ {format_sats(sats)} sats (woken by push if offline)" },
									}
								}
							}
						}
					}
				}
				Kv { label: "Role", "{role}" }
				if !sc.note.is_empty() {
					Kv { label: "Note", "{sc.note}" }
				}
			}
			if !sc.recent_locations.is_empty() {
				div { class: "stack tight", style: "margin-top: 12px;",
					span { class: "field-label", "Locations" }
					for loc in sc.recent_locations.clone() {
						div { key: "{loc.ip}", class: "row between small",
							span { "{crate::format::location_label(&loc)} · " span { class: "mono", "{loc.ip}" } }
							span { class: "muted", "{crate::ledger::exact_timestamp(loc.first_seen_at * 1000)} – {crate::ledger::relative_timestamp(loc.last_seen_at * 1000)}" }
						}
					}
					a { class: "small muted", href: "https://db-ip.com", target: "_blank", rel: "noopener noreferrer", "IP geolocation by DB-IP" }
				}
			}
		}
	}
}

#[component]
pub fn OpenChannelDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().open_channel.clone();
	let unit = unit_label(ctx.unit());
	let pending = ctx.busy(Op::OpenChannel);
	let preview = |input: &str| crate::format::amount_entry_preview(input, ctx.unit(), ctx.price_value());
	let mut cancel = move || {
		forms.write().open_channel = Default::default();
		close_dialog(ctx);
	};
	rsx! {
		Modal {
			title: "Open Channel",
			sub: "Fund a new Lightning channel from the on-chain wallet",
			icon: rsx! { Bubble { icon: "plus", tone: "green" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				button { class: "btn primary", disabled: pending, onclick: move |_| actions::open_channel(ctx),
					if pending { Spinner {} } else { Icon { name: "plus", size: 16 } }
					"Open Channel"
				}
			},
			Field { label: "Node Pubkey", help: HELP_NODE_PUBKEY, TextInput { value: form.node_pubkey.clone(), mono: true, oninput: move |v| forms.write().open_channel.node_pubkey = v } }
			Field { label: "Address", help: HELP_PEER_ADDRESS, TextInput { value: form.address.clone(), mono: true, placeholder: "host:port", oninput: move |v| forms.write().open_channel.address = v } }
			div { class: "form-grid",
				Field { label: "Channel Amount ({unit})", help: HELP_CHANNEL_AMOUNT, preview: preview(&form.channel_amount_sats),
					TextInput { value: form.channel_amount_sats.clone(), oninput: move |v| forms.write().open_channel.channel_amount_sats = v }
				}
				Field { label: "Push Amount ({unit})", help: HELP_PUSH_AMOUNT, preview: preview(&form.push_to_counterparty_msat),
					TextInput { value: form.push_to_counterparty_msat.clone(), oninput: move |v| forms.write().open_channel.push_to_counterparty_msat = v }
				}
			}
			div { class: "toggle-row",
				Check { checked: form.announce_channel, label: "Announce Channel", help: HELP_ANNOUNCE_CHANNEL, onchange: move |v| forms.write().open_channel.announce_channel = v }
			}
			details { class: "disclosure",
				summary { Icon { name: "chevron-right", size: 14 } "Advanced Options" }
				div { class: "body form-grid",
					Field { label: "Fee Proportional (millionths)", help: HELP_FEE_PROPORTIONAL, TextInput { value: form.forwarding_fee_proportional_millionths.clone(), oninput: move |v| forms.write().open_channel.forwarding_fee_proportional_millionths = v } }
					Field { label: "Fee Base (msat)", help: HELP_FEE_BASE, TextInput { value: form.forwarding_fee_base_msat.clone(), oninput: move |v| forms.write().open_channel.forwarding_fee_base_msat = v } }
					Field { label: "CLTV Expiry Delta", help: HELP_CLTV_EXPIRY_DELTA, TextInput { value: form.cltv_expiry_delta.clone(), oninput: move |v| forms.write().open_channel.cltv_expiry_delta = v } }
				}
			}
		}
	}
}

#[component]
pub fn CloseChannelDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let mut view = ctx.view;
	let form = forms.read().close_channel.clone();
	let confirmed = view.read().force_close_confirm;
	let close_pending = ctx.busy(Op::CloseChannel);
	let force_pending = ctx.busy(Op::ForceCloseChannel);
	let mut cancel = move || {
		forms.write().close_channel = Default::default();
		close_dialog(ctx);
	};
	rsx! {
		Modal {
			title: "Close Channel",
			sub: "Return the channel's funds on-chain",
			icon: rsx! { Bubble { icon: "x", tone: "red" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				Hover { tip: HELP_CLOSE_CHANNEL.to_string(),
					button { class: "btn primary", disabled: close_pending || force_pending, onclick: move |_| actions::close_channel(ctx),
						if close_pending { Spinner {} }
						"Close (Cooperative)"
					}
				}
			},
			Field { label: "Channel ID", help: HELP_CHANNEL_ID, TextInput { value: form.user_channel_id.clone(), mono: true, oninput: move |v| forms.write().close_channel.user_channel_id = v } }
			Field { label: "Counterparty", help: HELP_COUNTERPARTY, TextInput { value: form.counterparty_node_id.clone(), mono: true, oninput: move |v| forms.write().close_channel.counterparty_node_id = v } }
			Field { label: "Force Close Reason", hint: "Only used for a force close",
				TextInput { value: form.force_close_reason.clone(), oninput: move |v| forms.write().close_channel.force_close_reason = v }
			}
			// Destructive force close: gated behind an irreversibility checkbox.
			div { class: "confirm-box",
				div { class: "title", Icon { name: "alert", size: 16 } "Force Close" }
				span { class: "small muted", "Broadcasts the latest commitment transaction unilaterally. Funds stay locked until the timelock expires." }
				Check { checked: confirmed, danger: true, label: "I understand this is irreversible", onchange: move |v| view.write().force_close_confirm = v }
				div { class: "row",
					button { class: "btn danger solid", disabled: !confirmed || force_pending, onclick: move |_| actions::force_close_channel(ctx),
						if force_pending { Spinner {} }
						"Force Close"
					}
				}
			}
		}
	}
}

#[component]
pub fn SpliceInDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().splice_in.clone();
	let unit = unit_label(ctx.unit());
	let pending = ctx.busy(Op::SpliceIn);
	let preview = crate::format::amount_entry_preview(&form.splice_amount_sats, ctx.unit(), ctx.price_value());
	let mut cancel = move || {
		forms.write().splice_in = Default::default();
		close_dialog(ctx);
	};
	rsx! {
		Modal {
			title: "Splice In",
			sub: "Add funds to an existing channel",
			icon: rsx! { Bubble { icon: "merge", tone: "green" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				button { class: "btn primary", disabled: pending, onclick: move |_| actions::splice_in(ctx),
					if pending { Spinner {} }
					"Splice In"
				}
			},
			div { class: "notice", Icon { name: "info", size: 16 } "{HELP_SPLICE_IN}" }
			Field { label: "Channel ID", help: HELP_CHANNEL_ID, TextInput { value: form.user_channel_id.clone(), mono: true, oninput: move |v| forms.write().splice_in.user_channel_id = v } }
			Field { label: "Counterparty", help: HELP_COUNTERPARTY, TextInput { value: form.counterparty_node_id.clone(), mono: true, oninput: move |v| forms.write().splice_in.counterparty_node_id = v } }
			Field { label: "Amount ({unit})", help: HELP_SPLICE_IN, preview,
				TextInput { value: form.splice_amount_sats.clone(), oninput: move |v| forms.write().splice_in.splice_amount_sats = v }
			}
		}
	}
}

#[component]
pub fn SpliceOutDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let mut view = ctx.view;
	let form = forms.read().splice_out.clone();
	let confirmed = view.read().splice_out_confirm;
	let unit = unit_label(ctx.unit());
	let pending = ctx.busy(Op::SpliceOut);
	let preview = crate::format::amount_entry_preview(&form.splice_amount_sats, ctx.unit(), ctx.price_value());
	let mut cancel = move || {
		forms.write().splice_out = Default::default();
		close_dialog(ctx);
	};
	rsx! {
		Modal {
			title: "Splice Out",
			sub: "Remove funds from an existing channel",
			icon: rsx! { Bubble { icon: "split", tone: "purple" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				button { class: "btn danger solid", disabled: !confirmed || pending, onclick: move |_| actions::splice_out(ctx),
					if pending { Spinner {} }
					"Splice Out"
				}
			},
			div { class: "notice", Icon { name: "info", size: 16 } "{HELP_SPLICE_OUT}" }
			Field { label: "Channel ID", help: HELP_CHANNEL_ID, TextInput { value: form.user_channel_id.clone(), mono: true, oninput: move |v| forms.write().splice_out.user_channel_id = v } }
			Field { label: "Counterparty", help: HELP_COUNTERPARTY, TextInput { value: form.counterparty_node_id.clone(), mono: true, oninput: move |v| forms.write().splice_out.counterparty_node_id = v } }
			div { class: "form-grid",
				Field { label: "Amount ({unit})", help: HELP_SPLICE_OUT, preview,
					TextInput { value: form.splice_amount_sats.clone(), oninput: move |v| forms.write().splice_out.splice_amount_sats = v }
				}
				Field { label: "Address (optional)", help: HELP_ONCHAIN_ADDRESS,
					TextInput { value: form.address.clone(), mono: true, oninput: move |v| forms.write().splice_out.address = v }
				}
			}
			// Destructive splice out: gated behind an irreversibility checkbox.
			div { class: "confirm-box",
				Check { checked: confirmed, danger: true, label: "I understand this is irreversible", onchange: move |v| view.write().splice_out_confirm = v }
			}
		}
	}
}

#[component]
pub fn UpdateConfigDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().update_channel_config.clone();
	let pending = ctx.busy(Op::UpdateChannelConfig);
	let mut cancel = move || {
		forms.write().update_channel_config = Default::default();
		close_dialog(ctx);
	};
	rsx! {
		Modal {
			title: "Update Channel Config",
			sub: "Forwarding fees and CLTV delta for this channel",
			icon: rsx! { Bubble { icon: "sliders", tone: "blue" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				button { class: "btn primary", disabled: pending, onclick: move |_| actions::update_channel_config(ctx),
					if pending { Spinner {} }
					"Update Config"
				}
			},
			Field { label: "Channel ID", help: HELP_CHANNEL_ID, TextInput { value: form.user_channel_id.clone(), mono: true, oninput: move |v| forms.write().update_channel_config.user_channel_id = v } }
			Field { label: "Counterparty", help: HELP_COUNTERPARTY, TextInput { value: form.counterparty_node_id.clone(), mono: true, oninput: move |v| forms.write().update_channel_config.counterparty_node_id = v } }
			div { class: "form-grid",
				Field { label: "Fee Proportional (millionths)", help: HELP_FEE_PROPORTIONAL, TextInput { value: form.forwarding_fee_proportional_millionths.clone(), oninput: move |v| forms.write().update_channel_config.forwarding_fee_proportional_millionths = v } }
				Field { label: "Fee Base (msat)", help: HELP_FEE_BASE, TextInput { value: form.forwarding_fee_base_msat.clone(), oninput: move |v| forms.write().update_channel_config.forwarding_fee_base_msat = v } }
				Field { label: "CLTV Expiry Delta", help: HELP_CLTV_EXPIRY_DELTA, TextInput { value: form.cltv_expiry_delta.clone(), oninput: move |v| forms.write().update_channel_config.cltv_expiry_delta = v } }
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::health::drift;

	#[component]
	fn PanelHarness(sc: sc_rest_client::sc_protos::stable::StableChannelInfo) -> Element {
		use crate::state::{Connection, Forms, Nav, Prefs};
		use_context_provider(|| AppCtx::new(Connection::new(), Forms::default(), Nav::default(), Prefs::default(), false));
		rsx! { StableSection { sc, price: None } }
	}

	#[test]
	fn the_stable_panel_lists_past_locations_with_attribution() {
		use sc_rest_client::sc_protos::stable::{PeerLocation, StableChannelInfo};
		let loc = |ip: &str, code: &str, name: &str| PeerLocation {
			ip: ip.into(),
			country_code: code.into(),
			country_name: name.into(),
			first_seen_at: 1_790_000_000,
			last_seen_at: 1_790_000_100,
		};
		let sc = StableChannelInfo {
			user_channel_id: "u".into(),
			expected_usd: 10.0,
			recent_locations: vec![loc("118.95.161.42", "IN", "India"), loc("203.0.113.9", "", "")],
			..Default::default()
		};
		let mut dom = VirtualDom::new_with_props(PanelHarness, PanelHarnessProps { sc });
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Locations"));
		assert!(html.contains("IN · India") && html.contains("118.95.161.42"));
		assert!(html.contains("Unknown") && html.contains("203.0.113.9"));
		assert!(html.contains("IP geolocation by DB-IP"));
	}

	#[test]
	fn a_channel_without_a_target_reads_as_no_stable_position() {
		use sc_rest_client::sc_protos::stable::StableChannelInfo;
		let sc = StableChannelInfo { user_channel_id: "u".into(), expected_usd: 0.0, ..Default::default() };
		let mut dom = VirtualDom::new_with_props(PanelHarness, PanelHarnessProps { sc });
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("No stable position"));
		assert!(html.contains("Set target"));
		assert!(!html.contains("Backing"), "a routing peer has no backing to show");
	}

	#[test]
	fn drift_pills_are_green_above_the_target_and_red_below() {
		// $50 target at $100k: 50,030 sats is +0.06%, 49,990 is -0.02% (inside tolerance, still red), 49,700 is -0.60%.
		assert_eq!(drift_pill(&drift(50_030, 50.0, 100_000.0).unwrap()), ("+0.06%".to_owned(), "success"));
		assert_eq!(drift_pill(&drift(49_990, 50.0, 100_000.0).unwrap()), ("\u{2212}0.02%".to_owned(), "danger"));
		assert_eq!(drift_pill(&drift(49_700, 50.0, 100_000.0).unwrap()), ("\u{2212}0.60%".to_owned(), "danger"));
		assert_eq!(drift_pill(&drift(50_000, 50.0, 100_000.0).unwrap()), ("0.00%".to_owned(), "muted"));
	}

	#[test]
	fn drift_dollars_that_round_to_zero_carry_no_sign() {
		assert_eq!(signed_usd(-0.001), "$0.00");
		assert_eq!(signed_usd(0.004), "$0.00");
		assert_eq!(signed_usd(-0.30), "\u{2212}$0.30");
		assert_eq!(signed_usd(0.35), "+$0.35");
	}

	fn panel_html(backing_sats: u64) -> String {
		use sc_rest_client::sc_protos::stable::StableChannelInfo;
		let sc = StableChannelInfo {
			user_channel_id: "u".into(),
			expected_usd: 50.0,
			expected_msats: backing_sats * 1000,
			latest_price: 100_000.0,
			..Default::default()
		};
		let mut dom = VirtualDom::new_with_props(PanelHarness, PanelHarnessProps { sc });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn the_stable_panel_explains_drift_and_when_a_settlement_happens() {
		let due = panel_html(49_700);
		assert!(due.contains(">\u{2212}$0.30<") && !due.contains("from the"), "drift reads as just the signed dollars");
		for text in ["Settles at", "±$0.25", "Settlement due", "LSP sends ≈ 300 sats to the user"] {
			assert!(due.contains(text), "missing {text}");
		}
		let above = panel_html(50_400);
		assert!(above.contains("user&#39;s wallet sends ≈ 400 sats"), "above target the user pays (SSR escapes the apostrophe)");
		let calm = panel_html(49_900);
		assert!(calm.contains("No settlement yet · $0.15 more drift before one"));
	}
}
