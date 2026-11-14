use dioxus::prelude::*;

use std::collections::HashMap;

use sc_rest_client::sc_protos::stable::{QueuedSpliceInfo, StableChannelInfo};

use crate::actions;
use crate::format::{ago, format_sats, format_usd, unit_label};
use crate::health::{has_stable_position, settle_threshold_usd, settlement, stable_drift, Drift, Settlement};
use crate::state::{AppCtx, ChannelSortColumn, ChannelStatusFilter, Dialog, Drawer, Op, QueuedRelease};
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
const HELP_SPLICE_IN: &str = "Add on-chain funds to an existing channel without closing and reopening it. The daemon queues it, wakes the phone by push, and runs the splice once the channel is usable; the row below shows where it got to.";
const HELP_QUEUED_SPLICES: &str = "Splice-ins queued for the phone. A call the node has not answered stays pending: no second call and no new request go out until it locks, the node drops it, or you release it after checking the node.";
const RELEASE_WARNING: &str = "Release clears the call the node has not answered, so a new request can go out. If the node is still working on the earlier splice-in, both can land: the LSP's funds go into the channel twice, with a second on-chain fee. Release only after checking on the node that it holds no splice for this channel. The tick covers this call only.";
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
	let now = *ctx.now.read();
	let queued: Vec<QueuedSpliceInfo> = data
		.queued_splices
		.as_ref()
		.map(|q| q.splices.iter().filter(|row| queued_splice_is_shown(row, now)).cloned().collect())
		.unwrap_or_default();
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
			if !queued.is_empty() {
				QueuedSplices { rows: queued }
			}
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
		if !queued.is_empty() {
			QueuedSplices { rows: queued }
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
					RefreshBtn { busy: loading, onclick: move |_| { actions::fetch_channels(ctx); actions::fetch_queued_splices(ctx); }, op: Op::Channels }
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

/// The pill tone for a queued splice's status.
fn queued_splice_tone(status: &str) -> &'static str {
	match status {
		"locked" => "success",
		"failed" | "stalled" => "danger",
		"cancelled" => "muted",
		_ => "info",
	}
}

/// Whether a queued splice still belongs on the Channels tab: open, pending on the node (its Release button lives here), or finished within the last day.
fn queued_splice_is_shown(row: &QueuedSpliceInfo, now: u64) -> bool {
	row.pending_on_node
		|| !matches!(row.status.as_str(), "locked" | "cancelled")
		|| now.saturating_sub(row.updated_at.max(0) as u64) < 86_400
}

/// The row an open Release confirmation belongs to, with its tick; None once that row no longer has the confirmed call pending on the node.
fn armed_release(release: Option<&QueuedRelease>, rows: &[QueuedSpliceInfo]) -> Option<(QueuedSpliceInfo, bool)> {
	let release = release?;
	rows.iter().find(|row| release.is_for(row)).map(|row| (row.clone(), release.confirmed))
}

/// The row's note: its last error, and why the phone could not be reached when a later push failed.
fn queued_splice_note(row: &QueuedSpliceInfo) -> String {
	[row.last_error.as_str(), row.wake_note.as_str()].into_iter().filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" · ")
}

#[component]
fn QueuedSplices(rows: Vec<QueuedSpliceInfo>) -> Element {
	let ctx = use_context::<AppCtx>();
	let now = *ctx.now.read();
	let busy = ctx.busy(Op::CancelQueuedSplice);
	let loading = ctx.busy(Op::ListQueuedSplices);
	let mut view = ctx.view;
	let armed = armed_release(view.read().queued_release.as_ref(), &rows);
	rsx! {
		Card { title: "Queued splice-ins", help: HELP_QUEUED_SPLICES, class: "flush",
			actions: rsx! { RefreshBtn { busy: loading, onclick: move |_| actions::fetch_queued_splices(ctx), op: Op::ListQueuedSplices } },
			div { class: "table-wrap",
				table { class: "table",
					thead {
						tr {
							Th { label: "Peer" }
							Th { label: "Amount", class: "right" }
							Th { label: "Status" }
							Th { label: "Note" }
							Th { label: "Updated" }
							th { "" }
						}
					}
					tbody {
						for row in rows {
							{
								let uid = row.user_channel_id.clone();
								let release_row = row.clone();
								let tone = queued_splice_tone(&row.status);
								let note = queued_splice_note(&row);
								let open = !matches!(row.status.as_str(), "locked" | "cancelled");
								let updated = ago(row.updated_at.max(0) as u64, now);
								rsx! {
									tr { key: "{row.user_channel_id}",
										td { Peer { node_id: row.counterparty_node_id.clone() } }
										td { class: "right", Amount { msat: row.amount_sats.saturating_mul(1_000) } }
										td {
											Pill { tone, "{row.status}" }
											if row.attempts > 0 { " · " span { class: "muted", "{row.attempts} failed" } }
											if row.pending_on_node { " · " Pill { tone: "warning", "pending on node" } }
										}
										td { class: "muted", style: "max-width: 420px; white-space: normal;", "{note}" }
										td { class: "muted", "{updated}" }
										td { class: "right",
											if open {
												button { class: "btn sm ghost", disabled: busy, onclick: move |_| actions::cancel_queued_splice(ctx, uid.clone()), "Cancel" }
											}
											if row.pending_on_node {
												" "
												// Opens the confirmation below; nothing is sent from here.
												button { class: "btn sm danger", disabled: busy, title: "Only after checking that the node holds no splice for this channel",
													onclick: move |_| view.write().open_release(&release_row), "Release" }
											}
										}
									}
								}
							}
						}
					}
				}
			}
			if let Some((row, confirmed)) = armed {
				ReleaseConfirm { row, confirmed }
			}
		}
	}
}

/// The forced cancel behind a tick, like force close: it drops the queue's guard against a second splice on this channel.
#[component]
fn ReleaseConfirm(row: QueuedSpliceInfo, confirmed: bool) -> Element {
	let ctx = use_context::<AppCtx>();
	let mut view = ctx.view;
	let busy = ctx.busy(Op::CancelQueuedSplice);
	let called = ago(row.last_attempt_at.max(0) as u64, *ctx.now.read());
	let tick_row = row.clone();
	rsx! {
		div { class: "confirm-box", style: "margin: 12px;",
			div { class: "title", Icon { name: "alert", size: 16 } "Release the splice-in pending on the node" }
			div { class: "row",
				Peer { node_id: row.counterparty_node_id.clone() }
				Amount { msat: row.amount_sats.saturating_mul(1_000) }
				span { class: "small muted", "Call made {called}" }
			}
			if !row.funding_txo.is_empty() {
				span { class: "small muted", "The node reported this call's splice as signed: {row.funding_txo}" }
			}
			span { class: "small muted", "{RELEASE_WARNING}" }
			Check { checked: confirmed, danger: true, label: "I checked the node: it holds no splice for this channel",
				onchange: move |v| view.write().tick_release(&tick_row, v) }
			div { class: "row",
				button { class: "btn ghost", onclick: move |_| view.write().close_release(), "Keep waiting" }
				button { class: "btn danger solid", disabled: !confirmed || busy, onclick: move |_| actions::release_queued_splice(ctx), "Release" }
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
			sub: "Queue funds to add to a channel; the phone is woken by push",
			icon: rsx! { Bubble { icon: "merge", tone: "green" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				button { class: "btn primary", disabled: pending, onclick: move |_| actions::splice_in(ctx),
					if pending { Spinner {} }
					"Queue splice-in"
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

	fn queued_row(status: &str, pending_on_node: bool, updated_at: i64) -> QueuedSpliceInfo {
		QueuedSpliceInfo {
			user_channel_id: "7".into(),
			counterparty_node_id: "02aa".into(),
			amount_sats: 50_000,
			status: status.into(),
			pending_on_node,
			updated_at,
			created_at: 1_000,
			last_attempt_at: 2_000,
			..Default::default()
		}
	}

	#[test]
	fn a_finished_row_drops_off_after_a_day_but_a_call_pending_on_the_node_stays() {
		let now = 10_000_000u64;
		let ago = |secs: u64| (now - secs) as i64;
		for status in ["queued", "waking", "splicing", "negotiated", "failed", "stalled"] {
			assert!(queued_splice_is_shown(&queued_row(status, false, ago(30 * 86_400)), now), "{status} is open");
		}
		for status in ["locked", "cancelled"] {
			assert!(queued_splice_is_shown(&queued_row(status, false, ago(86_399)), now), "{status} within the day");
			assert!(!queued_splice_is_shown(&queued_row(status, false, ago(86_400)), now), "{status} after the day");
		}
		// A plain Cancel on a pending call: nothing touches the row again until the node answers or it is released.
		assert!(queued_splice_is_shown(&queued_row("cancelled", true, ago(30 * 86_400)), now));
	}

	#[test]
	fn queued_splice_pills_read_by_status() {
		assert_eq!(queued_splice_tone("locked"), "success");
		assert_eq!(queued_splice_tone("failed"), "danger");
		assert_eq!(queued_splice_tone("stalled"), "danger");
		assert_eq!(queued_splice_tone("cancelled"), "muted");
		for status in ["queued", "waking", "splicing", "negotiated"] {
			assert_eq!(queued_splice_tone(status), "info", "{status}");
		}
	}

	#[component]
	fn ChannelsHarness(row: QueuedSpliceInfo, channels_loaded: bool, release: Option<QueuedRelease>) -> Element {
		use crate::state::{Connection, ConnectionStatus, Forms, Nav, Prefs};
		use sc_rest_client::ldk_server_grpc::api::ListChannelsResponse;
		use sc_rest_client::sc_protos::stable::ListQueuedSplicesResponse;
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut data = ctx.data;
			let mut d = data.write();
			d.queued_splices = Some(ListQueuedSplicesResponse { splices: vec![row.clone()] });
			if channels_loaded {
				d.channels = Some(ListChannelsResponse::default());
			}
			let mut view = ctx.view;
			view.write().queued_release = release.clone();
		});
		rsx! { Channels {} }
	}

	fn channels_html(row: QueuedSpliceInfo, channels_loaded: bool) -> String {
		channels_html_with(row, channels_loaded, None)
	}

	fn channels_html_with(row: QueuedSpliceInfo, channels_loaded: bool, release: Option<QueuedRelease>) -> String {
		let mut dom = VirtualDom::new_with_props(ChannelsHarness, ChannelsHarnessProps { row, channels_loaded, release });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn a_cancelled_call_still_pending_on_the_node_keeps_its_release_button_after_a_day() {
		let two_days_ago = crate::format::now_secs() as i64 - 2 * 86_400;
		for channels_loaded in [true, false] {
			let html = channels_html(queued_row("cancelled", true, two_days_ago), channels_loaded);
			assert!(html.contains("Queued splice-ins"), "card shown (channels loaded: {channels_loaded})");
			assert!(html.contains("pending on node") && html.contains(">Release<"), "Release offered (channels loaded: {channels_loaded})");
		}
		let released = channels_html(queued_row("cancelled", false, two_days_ago), true);
		assert!(!released.contains("Queued splice-ins"), "a day-old row with nothing pending is gone");
	}

	#[test]
	fn the_queue_card_shows_before_the_channel_list_has_loaded() {
		let html = channels_html(queued_row("queued", false, crate::format::now_secs() as i64), false);
		assert!(html.contains("Queued splice-ins") && html.contains(">Cancel<"));
		assert!(html.contains("Loading channels...") || html.contains("No channel data available"), "the channel card still says why it is empty");
	}

	/// A confirmation for the call `queued_row` makes on channel `uid`.
	fn release_for(uid: &str, confirmed: bool) -> Option<QueuedRelease> {
		Some(QueuedRelease { user_channel_id: uid.into(), ..QueuedRelease::for_call(&queued_row("splicing", true, 0), confirmed) })
	}

	/// Whether the confirmation's final Release button is disabled; None when there is no confirmation.
	fn final_release_disabled(html: &str) -> Option<bool> {
		let at = html.find("btn danger solid")?;
		let tag_start = html[..at].rfind("<button")?;
		let tag_end = at + html[at..].find('>')?;
		Some(html[tag_start..tag_end].contains("disabled"))
	}

	#[test]
	fn release_only_sends_from_a_ticked_confirmation_for_that_channel() {
		let now = crate::format::now_secs() as i64;
		let pending = queued_row("splicing", true, now);
		let plain = channels_html_with(pending.clone(), true, None);
		assert!(plain.contains(">Release<") && !plain.contains("confirm-box"), "the row's Release only opens the confirmation");
		assert_eq!(final_release_disabled(&plain), None);

		let unticked = channels_html_with(pending.clone(), true, release_for("7", false));
		assert!(unticked.contains("confirm-box") && unticked.contains("I checked the node"));
		assert_eq!(final_release_disabled(&unticked), Some(true), "nothing can be sent before the tick");

		let ticked = channels_html_with(pending.clone(), true, release_for("7", true));
		assert_eq!(final_release_disabled(&ticked), Some(false));

		let other_channel = channels_html_with(pending, true, release_for("8", true));
		assert!(!other_channel.contains("confirm-box"), "a tick for another channel arms nothing here");
	}

	#[test]
	fn a_release_confirmation_belongs_to_a_row_still_pending_on_the_node() {
		let rows = vec![queued_row("splicing", true, 0), QueuedSpliceInfo { user_channel_id: "9".into(), ..queued_row("queued", false, 0) }];
		let open = |uid: &str, confirmed: bool| armed_release(release_for(uid, confirmed).as_ref(), &rows);
		assert_eq!(open("7", true).map(|(row, confirmed)| (row.user_channel_id, confirmed)), Some(("7".to_owned(), true)));
		assert_eq!(open("9", true), None, "no call pending on the node, nothing to release");
		assert_eq!(open("8", true), None, "no such row");
		assert_eq!(armed_release(None, &rows), None);
	}

	#[test]
	fn a_release_tick_is_for_one_call_and_only_a_new_call_or_an_answer_ends_it() {
		let call = queued_row("splicing", true, 3_000);
		let release = QueuedRelease::for_call(&call, true);
		assert!(release.is_for(&call));
		// Still the same call: the row stalls, is pushed again, or gets a plain Cancel while the call is pending.
		assert!(release.is_for(&QueuedSpliceInfo { status: "stalled".into(), updated_at: 9_000, ..call.clone() }));
		assert!(release.is_for(&QueuedSpliceInfo { wake_sent_at: 9_000, wake_note: "no usable push token".into(), ..call.clone() }));
		assert!(release.is_for(&QueuedSpliceInfo { status: "cancelled".into(), ..call.clone() }));
		// Not that call any more.
		assert!(!release.is_for(&QueuedSpliceInfo { last_attempt_at: 2_130, ..call.clone() }), "a new call, or the node's answer");
		assert!(!release.is_for(&QueuedSpliceInfo { created_at: 5_000, ..call.clone() }), "a new request on the channel");
		assert!(!release.is_for(&QueuedSpliceInfo { pending_on_node: false, ..call.clone() }), "nothing pending");
		assert!(!release.is_for(&QueuedSpliceInfo { user_channel_id: "8".into(), ..call }), "another channel");
	}

	#[test]
	fn the_rows_release_opens_an_unticked_confirmation_for_its_call_and_keep_waiting_closes_it() {
		let call = queued_row("splicing", true, 0);
		let mut view = crate::state::ViewState::default();
		view.open_release(&call);
		assert_eq!(view.queued_release, Some(QueuedRelease::for_call(&call, false)), "opening sends nothing and ticks nothing");
		view.tick_release(&call, true);
		assert_eq!(view.queued_release, Some(QueuedRelease::for_call(&call, true)));
		view.tick_release(&call, false);
		assert_eq!(view.queued_release.as_ref().map(|r| r.confirmed), Some(false), "the tick can be taken back");
		view.close_release();
		assert_eq!(view.queued_release, None);
	}

	#[test]
	fn a_tick_for_one_call_does_not_arm_a_later_call_on_the_channel() {
		let now = crate::format::now_secs() as i64;
		let retry = QueuedSpliceInfo { last_attempt_at: 2_130, ..queued_row("splicing", true, now) };
		let html = channels_html_with(retry, true, release_for("7", true));
		assert!(html.contains(">Release<") && !html.contains("confirm-box"), "the retry needs its own Release and tick");
		assert_eq!(final_release_disabled(&html), None);
		let requeued = QueuedSpliceInfo { created_at: 5_000, ..queued_row("splicing", true, now) };
		assert!(!channels_html_with(requeued, true, release_for("7", true)).contains("confirm-box"));
	}

	#[test]
	fn the_confirmation_shows_when_the_call_was_made_and_a_signed_splice() {
		let now = crate::format::now_secs() as i64;
		let call = QueuedSpliceInfo { last_attempt_at: now - 600, ..queued_row("splicing", true, now) };
		let html = channels_html_with(call.clone(), true, Some(QueuedRelease::for_call(&call, false)));
		assert!(html.contains("Call made 10m ago"), "{html}");
		assert!(!html.contains("as signed"));
		let negotiated = QueuedSpliceInfo { status: "negotiated".into(), funding_txo: "ab12:1".into(), ..call };
		let html = channels_html_with(negotiated.clone(), true, Some(QueuedRelease::for_call(&negotiated, false)));
		assert!(html.contains("splice as signed: ab12:1"), "{html}");
	}

	#[test]
	fn a_failed_row_shows_its_error_and_why_the_retry_could_not_reach_the_phone() {
		let failed = QueuedSpliceInfo {
			last_error: "splice-in refused".into(),
			wake_note: "no usable push token".into(),
			..queued_row("failed", false, 0)
		};
		assert_eq!(queued_splice_note(&failed), "splice-in refused · no usable push token");
		assert_eq!(queued_splice_note(&QueuedSpliceInfo { last_error: String::new(), ..failed.clone() }), "no usable push token");
		assert_eq!(queued_splice_note(&QueuedSpliceInfo { wake_note: String::new(), ..failed.clone() }), "splice-in refused");
		let html = channels_html(QueuedSpliceInfo { updated_at: crate::format::now_secs() as i64, ..failed }, true);
		assert!(html.contains("splice-in refused · no usable push token"));
	}

	#[component]
	fn ListedHarness(listed: QueuedSpliceInfo) -> Element {
		use crate::state::{Connection, ConnectionStatus, Forms, Nav, Prefs};
		use sc_rest_client::sc_protos::stable::ListQueuedSplicesResponse;
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut view = ctx.view;
			view.write().queued_release = release_for("7", true);
			actions::queued_splices_listed(ctx, ListQueuedSplicesResponse { splices: vec![listed.clone()] });
		});
		let rows = ctx.data.read().queued_splices.as_ref().map_or(0, |q| q.splices.len());
		let armed = ctx.view.read().queued_release.as_ref().map_or("none".to_owned(), |r| r.confirmed.to_string());
		rsx! { "rows={rows} armed={armed}" }
	}

	fn listed(row: QueuedSpliceInfo) -> String {
		let mut dom = VirtualDom::new_with_props(ListedHarness, ListedHarnessProps { listed: row });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn a_new_list_keeps_a_tick_only_while_its_call_is_still_pending() {
		assert_eq!(listed(queued_row("stalled", true, 9_000)), "rows=1 armed=true", "same call, now stalled");
		let answered = QueuedSpliceInfo { last_attempt_at: 2_050, ..queued_row("failed", false, 9_000) };
		assert_eq!(listed(answered), "rows=1 armed=none", "the node answered");
		let later = QueuedSpliceInfo { last_attempt_at: 2_200, ..queued_row("splicing", true, 9_000) };
		assert_eq!(listed(later), "rows=1 armed=none", "a later call");
	}

	#[component]
	fn ReleaseHarness(release: Option<QueuedRelease>) -> Element {
		use crate::state::{Connection, ConnectionStatus, Forms, Nav, Prefs};
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut view = ctx.view;
			view.write().queued_release = release.clone();
			actions::release_queued_splice(ctx);
		});
		let armed = ctx.view.read().queued_release.is_some();
		rsx! { "armed={armed}" }
	}

	fn release_pressed(release: Option<QueuedRelease>) -> String {
		let mut dom = VirtualDom::new_with_props(ReleaseHarness, ReleaseHarnessProps { release });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn release_goes_ahead_only_from_a_ticked_confirmation() {
		assert_eq!(release_pressed(release_for("7", false)), "armed=true", "unticked: nothing is sent and the box stays");
		assert_eq!(release_pressed(release_for("7", true)), "armed=false", "ticked: the box closes as the release goes out");
		assert_eq!(release_pressed(None), "armed=false");
	}

	#[test]
	fn release_is_sent_only_while_the_daemon_still_lists_the_checked_call() {
		use sc_rest_client::sc_protos::stable::ListQueuedSplicesResponse;
		let release = release_for("7", true).unwrap();
		let list = |row: QueuedSpliceInfo| ListQueuedSplicesResponse { splices: vec![row] };
		let request = actions::queued_release_request(&release, &list(queued_row("stalled", true, 9_000))).expect("same call, still pending");
		assert_eq!((request.user_channel_id.as_str(), request.force), ("7", true), "a release is the forced cancel for that channel");
		let retried = QueuedSpliceInfo { last_attempt_at: 2_130, ..queued_row("splicing", true, 9_000) };
		assert_eq!(actions::queued_release_request(&release, &list(retried)), None, "the daemon made a new call since the check");
		let answered = QueuedSpliceInfo { last_attempt_at: 2_050, ..queued_row("failed", false, 9_000) };
		assert_eq!(actions::queued_release_request(&release, &list(answered)), None, "the node answered: nothing left to release");
		assert_eq!(actions::queued_release_request(&release, &ListQueuedSplicesResponse::default()), None);
	}

	#[component]
	fn QueueActionHarness(failure: Option<String>) -> Element {
		use crate::state::{Connection, ConnectionStatus, Forms, Nav, Prefs};
		use sc_rest_client::sc_protos::stable::ListQueuedSplicesResponse;
		let ctx = use_context_provider(|| {
			let mut conn = Connection::new();
			conn.status = ConnectionStatus::Connected;
			AppCtx::new(conn, Forms::default(), Nav::default(), Prefs::default(), false)
		});
		use_hook(move || {
			let mut data = ctx.data;
			data.write().queued_splices = Some(ListQueuedSplicesResponse { splices: vec![queued_row("splicing", true, 0)] });
			let mut view = ctx.view;
			view.write().queued_release = release_for("7", true);
			match failure.clone() {
				Some(error) => actions::queued_splices_failed(ctx, error),
				None => actions::disconnect(ctx),
			}
		});
		let status = ctx.status.read().clone().map(|m| m.text).unwrap_or_default();
		let rows = ctx.data.read().queued_splices.as_ref().map_or(0, |q| q.splices.len());
		let armed = ctx.view.read().queued_release.is_some();
		rsx! { "status=[{status}] rows={rows} armed={armed}" }
	}

	fn queue_action(failure: Option<&str>) -> String {
		let mut dom = VirtualDom::new_with_props(QueueActionHarness, QueueActionHarnessProps { failure: failure.map(str::to_owned) });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn a_failed_queue_list_is_reported_unless_the_daemon_is_older_and_keeps_its_rows() {
		assert_eq!(queue_action(Some("HTTP 404 Not Found")), "status=[] rows=1 armed=true");
		assert_eq!(queue_action(Some("HTTP request failed: connection refused")), "status=[HTTP request failed: connection refused] rows=1 armed=true");
	}

	#[test]
	fn disconnecting_forgets_the_queue_and_closes_an_open_release() {
		assert_eq!(queue_action(None), "status=[Disconnected] rows=0 armed=false");
	}
}
