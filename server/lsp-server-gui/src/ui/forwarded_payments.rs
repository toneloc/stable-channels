use std::collections::HashMap;

use dioxus::prelude::*;
use crate::actions;
use crate::format::{csv_row, truncate_id};
use crate::state::{AppCtx, Op};
use crate::ui::widgets::{Amount, Card, Empty, Gate, Icon, IdCopy, Pill, RefreshBtn, Spinner, Stat, Th};

const HELP_TOTAL_FEE: &str = "Fees earned by this node for forwarding payments.";
const HELP_AMOUNT_FORWARDED: &str = "Total value forwarded through this node.";
const HELP_INCOMING: &str = "The channel (and its peer) the payment arrived on.";
const HELP_OUTGOING: &str = "The channel (and its peer) the payment was forwarded to.";
const HELP_BY_CHANNEL: &str = "Routing fees grouped by the outgoing channel, highest first.";
// Totals and rows cover only what LDK Server still holds individually.
const RECENT_WINDOW_NOTE: &str = "LDK Server keeps individual forwards for about two hours, and only in detailed mode; older ones survive only as hourly totals.";

/// One side of a forward: the channel and, when known, the peer behind it.
#[derive(Clone, PartialEq)]
struct Leg {
	channel_id: String,
	node_id: Option<String>,
}

#[derive(Clone, PartialEq)]
struct ForwardRow {
	incoming: Option<Leg>,
	outgoing: Option<Leg>,
	fee_msat: u64,
	skimmed_msat: u64,
	amount_msat: u64,
	onchain_claim: bool,
}

/// One side of a forward, filling the peer from our channel list when the record lacks it.
fn leg(channel_id: &str, node_id: Option<&String>, peers: &HashMap<String, String>) -> Option<Leg> {
	if channel_id.is_empty() {
		return None;
	}
	let node_id = node_id.filter(|n| !n.is_empty()).cloned().or_else(|| peers.get(channel_id).cloned());
	Some(Leg { channel_id: channel_id.to_owned(), node_id })
}

/// CSV of forwards (msat amounts, channel and node ids).
fn forwards_csv(rows: &[ForwardRow]) -> String {
	let mut out = String::from(
		"incoming_channel_id,incoming_node_id,outgoing_channel_id,outgoing_node_id,fee_earned_msat,skimmed_fee_msat,amount_forwarded_msat,claimed_onchain\n",
	);
	let side = |leg: &Option<Leg>| {
		leg.as_ref()
			.map(|l| (l.channel_id.clone(), l.node_id.clone().unwrap_or_default()))
			.unwrap_or_default()
	};
	for r in rows {
		let (in_channel, in_node) = side(&r.incoming);
		let (out_channel, out_node) = side(&r.outgoing);
		out.push_str(&csv_row(&[
			in_channel,
			in_node,
			out_channel,
			out_node,
			r.fee_msat.to_string(),
			r.skimmed_msat.to_string(),
			r.amount_msat.to_string(),
			r.onchain_claim.to_string(),
		]));
		out.push('\n');
	}
	out
}

/// Revenue per outgoing channel, highest first: (leg, forwards, fee_msat, amount_msat).
fn revenue_by_channel(rows: &[ForwardRow]) -> Vec<(Leg, usize, u64, u64)> {
	let mut by: HashMap<String, (Leg, usize, u64, u64)> = HashMap::new();
	for r in rows {
		let Some(out) = &r.outgoing else { continue };
		let entry = by.entry(out.channel_id.clone()).or_insert_with(|| (out.clone(), 0, 0, 0));
		entry.1 += 1;
		entry.2 += r.fee_msat;
		entry.3 += r.amount_msat;
	}
	let mut list: Vec<_> = by.into_values().collect();
	list.sort_by(|a, b| b.2.cmp(&a.2).then(b.3.cmp(&a.3)).then(a.0.channel_id.cmp(&b.0.channel_id)));
	list
}

#[component]
pub fn ForwardedPayments() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let loading = ctx.busy(Op::ForwardedPayments);
	let data = ctx.data.read();
	let peers: HashMap<String, String> = data
		.channels
		.as_ref()
		.map(|c| c.channels.iter().map(|ch| (ch.channel_id.clone(), ch.counterparty_node_id.clone())).collect())
		.unwrap_or_default();
	let rows: Option<Vec<ForwardRow>> = data.forwarded_payments.as_ref().map(|resp| {
		resp.forwarded_payments
			.iter()
			.map(|fp| ForwardRow {
				incoming: leg(&fp.prev_channel_id, fp.prev_node_id.as_ref(), &peers),
				outgoing: leg(&fp.next_channel_id, fp.next_node_id.as_ref(), &peers),
				fee_msat: fp.total_fee_earned_msat.unwrap_or(0),
				skimmed_msat: fp.skimmed_fee_msat.unwrap_or(0),
				amount_msat: fp.outbound_amount_forwarded_msat.unwrap_or(0),
				onchain_claim: fp.claim_from_onchain_tx,
			})
			.collect()
	});
	drop(data);
	let refresh = move |_| {
		actions::fetch_forwarded_payments(ctx);
		actions::fetch_channels(ctx);
	};
	let Some(rows) = rows.filter(|r| !r.is_empty()) else {
		return rsx! {
			Card {
				if loading {
					Empty { icon: "forward", title: "Loading...", Spinner { large: true } }
				} else {
					Empty { icon: "forward", title: "No recent forwarded payments", hint: RECENT_WINDOW_NOTE,
						RefreshBtn { busy: loading, onclick: refresh }
					}
				}
			}
		};
	};
	let total_fee: u64 = rows.iter().map(|r| r.fee_msat).sum();
	let total_forwarded: u64 = rows.iter().map(|r| r.amount_msat).sum();
	let by_channel = revenue_by_channel(&rows);
	let count = rows.len();
	let csv = forwards_csv(&rows);
	let export = move |_| actions::save_export(ctx, "forwards.csv", "text/csv", csv.clone(), format!("{count} forwards"));
	rsx! {
		div { class: "grid-3",
			Stat { title: "Forwards", value: format!("{count}"), sub: "Last ~2 hours" }
			Stat { title: "Routing fees", help: HELP_TOTAL_FEE, value: ctx.fmt_msat(total_fee), sub: "Last ~2 hours · all-time on Revenue" }
			Stat { title: "Forwarded", help: HELP_AMOUNT_FORWARDED, value: ctx.fmt_msat(total_forwarded), sub: "Outbound volume, last ~2 hours" }
		}
		div { class: "grid-main",
			Card { class: "flush",
				div { class: "toolbar",
					span { class: "count", title: RECENT_WINDOW_NOTE, "{count} recent forwards" }
					div { class: "row", style: "margin-left: auto;",
						button { class: "btn sm", disabled: ctx.busy(Op::Export), onclick: export, Icon { name: "csv", size: 14 } "Export CSV" }
						RefreshBtn { busy: loading, onclick: refresh, op: Op::ForwardedPayments }
					}
				}
				div { class: "table-wrap",
					table { class: "table", style: "min-width: 620px;",
						thead {
							tr {
								Th { label: "Incoming", help: HELP_INCOMING }
								Th { label: "Outgoing", help: HELP_OUTGOING }
								Th { label: "Amount", help: HELP_AMOUNT_FORWARDED, class: "right" }
								Th { label: "Fee earned", help: HELP_TOTAL_FEE, class: "right" }
							}
						}
						tbody {
							for (i, row) in rows.into_iter().enumerate() {
								tr { key: "{i}",
									td { LegCell { leg: row.incoming.clone() } }
									td { LegCell { leg: row.outgoing.clone() } }
									td { class: "right", Amount { msat: row.amount_msat } }
									td { class: "right",
										span { style: "color: var(--orange-text);", Amount { msat: row.fee_msat, strong: true } }
										if row.onchain_claim {
											div { Pill { tone: "warning", "claimed on-chain" } }
										}
									}
								}
							}
						}
					}
				}
			}
			Card { title: "Routing fees by outgoing channel", help: HELP_BY_CHANNEL, class: "flush",
				div { class: "table-wrap",
					table { class: "table",
						thead {
							tr {
								Th { label: "Channel" }
								Th { label: "Forwards", class: "right" }
								Th { label: "Fees", class: "right" }
							}
						}
						tbody {
							for (leg, forwards, fee, _amount) in by_channel {
								tr { key: "{leg.channel_id}",
									td { LegCell { leg: Some(leg.clone()) } }
									td { class: "right num", "{forwards}" }
									td { class: "right", Amount { msat: fee, strong: true } }
								}
							}
						}
					}
				}
			}
		}
	}
}

/// Peer name (or node id) with the channel id underneath.
#[component]
fn LegCell(leg: Option<Leg>) -> Element {
	let ctx = use_context::<AppCtx>();
	let Some(leg) = leg else { return rsx! { span { class: "faint", "—" } } };
	let alias = leg.node_id.as_deref().and_then(|n| ctx.data.read().alias(n));
	rsx! {
		span { class: "amount",
			match (alias, leg.node_id.clone()) {
				(Some(alias), _) => rsx! { span { class: "peer-alias", "{alias}" } },
				(None, Some(node_id)) => rsx! { IdCopy { value: node_id, head: 6, tail: 6 } },
				(None, None) => rsx! {},
			}
			span { class: "amount-sub mono", title: "{leg.channel_id}", "ch {truncate_id(&leg.channel_id, 6, 6)}" }
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn row(out: &str, fee: u64, amount: u64) -> ForwardRow {
		ForwardRow {
			incoming: Some(Leg { channel_id: "in".into(), node_id: Some("02in".into()) }),
			outgoing: Some(Leg { channel_id: out.into(), node_id: None }),
			fee_msat: fee,
			skimmed_msat: 0,
			amount_msat: amount,
			onchain_claim: false,
		}
	}

	#[test]
	fn legs_fall_back_to_our_channel_list_for_the_peer() {
		let peers: HashMap<String, String> = [("c1".to_string(), "02peer".to_string())].into();
		let resolved = leg("c1", None, &peers).unwrap();
		assert_eq!(resolved.node_id.as_deref(), Some("02peer"));
		assert_eq!(leg("c1", Some(&"02own".to_string()), &peers).unwrap().node_id.as_deref(), Some("02own"));
		assert!(leg("", None, &peers).is_none());
	}

	#[test]
	fn revenue_groups_by_outgoing_channel_highest_first() {
		let rows = vec![row("a", 100, 1_000), row("b", 500, 2_000), row("a", 450, 3_000)];
		let grouped = revenue_by_channel(&rows);
		assert_eq!(grouped[0].0.channel_id, "a");
		assert_eq!((grouped[0].1, grouped[0].2, grouped[0].3), (2, 550, 4_000));
		assert_eq!(grouped[1].0.channel_id, "b");
	}

	#[test]
	fn forwards_csv_has_one_line_per_forward() {
		let csv = forwards_csv(&[row("a", 100, 1_000)]);
		let lines: Vec<&str> = csv.lines().collect();
		assert_eq!(lines.len(), 2);
		assert_eq!(lines[1], "in,02in,a,,100,0,1000,false");
	}
}
