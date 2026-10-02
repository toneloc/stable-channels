use dioxus::prelude::*;
use hex::DisplayHex;

use crate::actions;
use crate::format::{csv_row, local_datetime, relative_short, truncate_id};
use crate::state::{AppCtx, Op, PaymentSortColumn, SettlementKind};
use crate::ui::widgets::{
	Amount, Bubble, Card, CopyBtn, Empty, Gate, Hover, Icon, IdCopy, Kv, Pill, RefreshBtn, SegBtn, SidePanel, SortTh,
	Spinner, TextInput, Th, When,
};
use crate::ui::close_drawer;
use sc_rest_client::ldk_server_grpc::types::PaymentKind;

const HELP_PAYMENT_ID: &str =
	"The server-side identifier for this payment record. It is distinct from a payment hash or on-chain transaction id.";
const HELP_PAYMENT_TYPE: &str = "The payment protocol or kind, such as on-chain, BOLT11, BOLT12 offer, BOLT12 refund, spontaneous, trade, stability, or sync.";
const HELP_PAYMENT_AMOUNT: &str =
	"The payment amount recorded for this payment. Any fee paid by this node is shown separately when known.";
const HELP_PAYMENT_FEE: &str =
	"The routing or transaction fee paid by this node when known. Inbound payments usually have no fee paid by this node.";
const HELP_PAYMENT_DIRECTION: &str = "Inbound means received by this node. Outbound means sent by this node.";
const HELP_PAYMENT_STATUS: &str = "The payment lifecycle state, such as pending, succeeded, or failed.";
const HELP_PAYMENT_TIMESTAMP: &str =
	"The latest update time for this payment. Hover the value for the raw Unix timestamp.";
const HELP_PAYMENT_HASH: &str = "The hash locking a Lightning payment. The matching preimage proves settlement.";
const HELP_PREIMAGE: &str =
	"The secret value that satisfies the payment hash and proves the Lightning payment settled.";
const HELP_SECRET: &str =
	"Payment secret material used to bind or authorize the payment request and protect the recipient.";
const HELP_OFFER_ID: &str = "Identifier for the BOLT12 offer involved in this payment.";
const HELP_PAYER_NOTE: &str = "Optional note supplied by the payer in a BOLT12 flow.";
const HELP_QUANTITY: &str = "Quantity requested from a BOLT12 offer when present.";
const HELP_TXID: &str = "The Bitcoin transaction identifier for an on-chain payment.";

// Per-row snapshot extracted from state.payments.
#[derive(Clone, PartialEq)]
struct PaymentRow {
	id: String,
	hash: String,
	type_label: String,
	amount_msat: Option<u64>,
	fee_paid_msat: Option<u64>,
	direction: i32,
	status: i32,
	timestamp: u64,
}

const TYPE_FILTERS: [&str; 15] = [
	"BOLT11", "BOLT12 Offer", "BOLT12 Refund", "Spontaneous", "On-chain", "Funding", "Splice", "Co-op close", "Force close",
	"Fee bump", "Claim", "Sweep", "Stability", "Trade", "Sync",
];

fn direction_label(direction: i32) -> &'static str {
	match direction {
		0 => "Inbound",
		1 => "Outbound",
		_ => "Unknown",
	}
}

/// CSV of the given rows (amounts in msat, times in unix seconds and UTC).
fn payments_csv(rows: &[PaymentRow]) -> String {
	let mut out = String::from("payment_id,type,direction,status,amount_msat,fee_paid_msat,updated_unix,updated_utc,hash\n");
	for r in rows {
		let utc = chrono::DateTime::from_timestamp(r.timestamp as i64, 0).map(|t| t.to_rfc3339()).unwrap_or_default();
		out.push_str(&csv_row(&[
			r.id.clone(),
			r.type_label.clone(),
			direction_label(r.direction).to_string(),
			status_style(r.status).0.to_string(),
			r.amount_msat.map(|a| a.to_string()).unwrap_or_default(),
			r.fee_paid_msat.map(|f| f.to_string()).unwrap_or_default(),
			r.timestamp.to_string(),
			utc,
			r.hash.clone(),
		]));
		out.push('\n');
	}
	out
}

#[component]
pub fn Payments() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let loading = ctx.busy(Op::Payments);
	let loading_all = ctx.busy(Op::PaymentsAll);
	let data = ctx.data.read();
	let settlement_kinds = data.settlement_kinds.as_ref();
	let rows: Option<Vec<PaymentRow>> = data.payments.as_ref().map(|resp| {
		resp.payments
			.iter()
			.map(|p| PaymentRow {
				id: p.payment_id.clone(),
				hash: p.kind.as_ref().map(payment_hash).unwrap_or_default(),
				type_label: payment_type_label_str(
					&p.kind.as_ref().map(format_payment_kind).unwrap_or_else(|| "Unknown".to_string()),
					settlement_kinds.and_then(|m| m.get(&p.payment_id).copied()),
				),
				amount_msat: p.amount_msat,
				fee_paid_msat: p.fee_paid_msat,
				direction: p.direction,
				status: p.status,
				timestamp: p.latest_update_timestamp,
			})
			.collect()
	});
	let has_more = data.payments_page_token.is_some();
	let progress = data.payments_load_all_progress;
	drop(data);
	let mut view = ctx.view;
	let v = view.read();
	let filter = v.payment_filter.clone();
	let status_filter = v.payment_status;
	let dir_filter = v.payment_direction;
	let type_filter = v.payment_type.clone();
	let sort = v.payment_sort;
	drop(v);
	let refresh = move |_| actions::fetch_payments(ctx, false);

	let Some(rows) = rows else {
		return rsx! {
			Card {
				if loading {
					Empty { icon: "receipt", title: "Loading...", Spinner { large: true } }
				} else {
					Empty { icon: "receipt", title: "No payment data available", hint: "Click Refresh to fetch.",
						RefreshBtn { busy: loading, onclick: refresh }
					}
				}
			}
		};
	};

	let total = rows.len();
	// Build the rendered view by filtering the loaded rows.
	let needle = filter.trim().to_lowercase();
	let mut view_rows: Vec<PaymentRow> = rows
		.into_iter()
		.filter(|r| {
			let matches_text = needle.is_empty()
				|| r.id.to_lowercase().contains(&needle)
				|| r.hash.to_lowercase().contains(&needle)
				|| r.type_label.to_lowercase().contains(&needle);
			let matches_status = status_filter < 0 || r.status == status_filter;
			let matches_dir = dir_filter < 0 || r.direction == dir_filter;
			let matches_type = type_filter.is_empty() || r.type_label == type_filter;
			matches_text && matches_status && matches_dir && matches_type
		})
		.collect();
	// Sort the view (purely a display ordering; underlying list untouched).
	view_rows.sort_by(|ra, rb| {
		let ord = match sort.column {
			PaymentSortColumn::Amount => ra.amount_msat.unwrap_or(0).cmp(&rb.amount_msat.unwrap_or(0)),
			PaymentSortColumn::Timestamp => ra.timestamp.cmp(&rb.timestamp),
		};
		if sort.descending {
			ord.reverse()
		} else {
			ord
		}
	});
	let shown = view_rows.len();
	let narrowed = !needle.is_empty()
		|| status_filter >= 0
		|| dir_filter >= 0
		|| !type_filter.is_empty()
		|| sort.column == PaymentSortColumn::Amount;
	let export_rows = view_rows.clone();
	// Built on click, not per render: the table re-renders on every pending-op change.
	let export = move |_| {
		actions::save_export(ctx, "payments.csv", "text/csv", payments_csv(&export_rows), format!("{shown} payments"));
	};
	let mut sort_by = move |column: PaymentSortColumn| {
		let current = view.peek().payment_sort;
		view.write().payment_sort = current.toggled(column);
	};
	let busy_any = loading || loading_all;

	rsx! {
		Card { class: "flush",
			div { class: "toolbar",
				div { class: "search",
					Icon { name: "search", size: 15 }
					TextInput { value: filter, small: true, placeholder: "id, hash or type", oninput: move |v| view.write().payment_filter = v }
				}
				select {
					class: "select sm",
					style: "width: 150px;",
					"aria-label": "Status filter",
					onchange: move |e| view.write().payment_status = e.value().parse().unwrap_or(-1),
					for (value, label) in [(-1, "All statuses"), (0, "Pending"), (1, "Succeeded"), (2, "Failed")] {
						option { value: "{value}", selected: value == status_filter, "{label}" }
					}
				}
				select {
					class: "select sm",
					style: "width: 150px;",
					"aria-label": "Type filter",
					onchange: move |e| view.write().payment_type = e.value(),
					option { value: "", selected: type_filter.is_empty(), "All types" }
					for label in TYPE_FILTERS {
						option { key: "{label}", value: "{label}", selected: type_filter == label, "{label}" }
					}
				}
				div { class: "seg",
					SegBtn { active: dir_filter == -1, onclick: move |_| view.write().payment_direction = -1, "All" }
					SegBtn { active: dir_filter == 0, onclick: move |_| view.write().payment_direction = 0, "In" }
					SegBtn { active: dir_filter == 1, onclick: move |_| view.write().payment_direction = 1, "Out" }
				}
				span { class: "count", "{shown}/{total} payment(s)" }
				div { class: "row", style: "margin-left: auto;",
					button { class: "btn sm", disabled: shown == 0 || ctx.busy(Op::Export), title: "Export the rows shown", onclick: export,
						Icon { name: "csv", size: 14 }
						"Export CSV"
					}
					RefreshBtn { busy: busy_any, onclick: refresh, op: Op::Payments }
				}
			}
			if has_more && narrowed {
				div { class: "banner",
					Icon { name: "info", size: 16 }
					span { class: "grow",
						"Filters and sorting only cover the {total} payments loaded so far. More exist on the daemon."
					}
					LoadAllBtn { busy: busy_any, progress }
				}
			}
			if total == 0 {
				if !loading {
					Empty { icon: "receipt", title: "No payments found." }
				}
			} else if shown == 0 {
				Empty { icon: "search", title: "No loaded payments match these filters." }
			} else {
				div { class: "table-wrap",
					table { class: "table clickable", style: "min-width: 900px;",
						thead {
							tr {
								SortTh { label: "Date", help: HELP_PAYMENT_TIMESTAMP, active: sort.column == PaymentSortColumn::Timestamp, descending: sort.descending, onclick: move |_| sort_by(PaymentSortColumn::Timestamp) }
								Th { label: "Type", help: HELP_PAYMENT_TYPE }
								Th { label: "Direction", help: HELP_PAYMENT_DIRECTION }
								SortTh { label: "Amount", class: "right", help: HELP_PAYMENT_AMOUNT, active: sort.column == PaymentSortColumn::Amount, descending: sort.descending, onclick: move |_| sort_by(PaymentSortColumn::Amount) }
								Th { label: "Fee", help: HELP_PAYMENT_FEE, class: "right" }
								Th { label: "Status", help: HELP_PAYMENT_STATUS }
								th { "" }
							}
						}
						tbody {
							for row in view_rows {
								PaymentRowView { key: "{row.id}", row }
							}
						}
					}
				}
			}
			if has_more {
				div { class: "table-foot",
					span { "More payments available on the daemon." }
					div { class: "row",
						button { class: "btn sm", disabled: busy_any, onclick: move |_| actions::fetch_payments(ctx, true),
							if loading { Spinner {} } else { Icon { name: "chevron-down", size: 14 } }
							"Load More"
						}
						LoadAllBtn { busy: busy_any, progress }
					}
				}
			}
		}
	}
}

/// Fetch every remaining page, showing progress while it runs.
#[component]
fn LoadAllBtn(busy: bool, progress: Option<usize>) -> Element {
	let ctx = use_context::<AppCtx>();
	rsx! {
		button { class: "btn sm", disabled: busy, onclick: move |_| actions::load_all_payments(ctx),
			match progress {
				Some(pages) => rsx! { Spinner {} "Loading… {pages} more pages" },
				None => rsx! { Icon { name: "download", size: 14 } "Load all" },
			}
		}
	}
}

#[component]
fn PaymentRowView(row: PaymentRow) -> Element {
	let ctx = use_context::<AppCtx>();
	let (status_text, status_tone) = status_style(row.status);
	let id = row.id.clone();
	rsx! {
		tr { onclick: move |_| actions::open_payment_details(ctx, id.clone()),
			td { When { ts: row.timestamp } }
			td { "{row.type_label}" }
			td {
				match row.direction {
					0 => rsx! { Pill { tone: "info", Icon { name: "arrow-down", size: 12 } "In" } },
					1 => rsx! { Pill { tone: "gold", Icon { name: "arrow-up", size: 12 } "Out" } },
					_ => rsx! { Pill { tone: "muted", "Unknown" } },
				}
			}
			td { class: "right",
				match row.amount_msat {
					Some(amount) => rsx! { Amount { msat: amount, strong: true } },
					None => rsx! { span { class: "faint", "-" } },
				}
			}
			td { class: "right",
				match row.fee_paid_msat {
					Some(fee) => rsx! { Amount { msat: fee } },
					None => rsx! { span { class: "faint", "-" } },
				}
			}
			td { Pill { tone: status_tone, "{status_text}" } }
			td { class: "right", span { class: "faint", Icon { name: "chevron-right", size: 16 } } }
		}
	}
}

// Text and tone for the status pill.
fn status_style(status: i32) -> (&'static str, &'static str) {
	match status {
		0 => ("Pending", "warning"),
		1 => ("Succeeded", "success"),
		2 => ("Failed", "danger"),
		_ => ("Unknown", "muted"),
	}
}

// Best-effort payment hash string for substring filtering (empty if none).
fn payment_hash(kind: &PaymentKind) -> String {
	use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

	match &kind.kind {
		Some(Kind::Onchain(o)) => o.txid.clone(),
		Some(Kind::Bolt11(b)) => b.hash.clone(),
		Some(Kind::Bolt12Offer(o)) => o.hash.clone().unwrap_or_default(),
		Some(Kind::Bolt12Refund(r)) => r.hash.clone().unwrap_or_default(),
		Some(Kind::Spontaneous(s)) => s.hash.clone(),
		None => String::new(),
	}
}

fn format_payment_kind(kind: &PaymentKind) -> String {
	use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

	use sc_rest_client::ldk_server_grpc::types::transaction_type::Kind as Tx;

	match &kind.kind {
		// LDK's classification of a channel transaction; plain sends and older servers leave it unset.
		Some(Kind::Onchain(o)) => match o.tx_type.as_ref().and_then(|t| t.kind.as_ref()) {
			Some(Tx::Funding(_)) => "Funding".to_string(),
			Some(Tx::InteractiveFunding(_)) => "Splice".to_string(),
			Some(Tx::CooperativeClose(_)) => "Co-op close".to_string(),
			Some(Tx::UnilateralClose(_)) => "Force close".to_string(),
			Some(Tx::AnchorBump(_)) => "Fee bump".to_string(),
			Some(Tx::Claim(_)) => "Claim".to_string(),
			Some(Tx::Sweep(_)) => "Sweep".to_string(),
			None => "On-chain".to_string(),
		},
		Some(Kind::Bolt11(_)) => "BOLT11".to_string(),
		Some(Kind::Bolt12Offer(_)) => "BOLT12 Offer".to_string(),
		Some(Kind::Bolt12Refund(_)) => "BOLT12 Refund".to_string(),
		Some(Kind::Spontaneous(_)) => "Spontaneous".to_string(),
		None => "Unknown".to_string(),
	}
}

/// Grid display wrapper: the kind label is precomputed into `type_label`; apply only the override.
fn payment_type_label_str(type_label: &str, settlement: Option<SettlementKind>) -> String {
	match settlement {
		Some(SettlementKind::Stability) => "Stability".to_string(),
		Some(SettlementKind::Trade) => "Trade".to_string(),
		Some(SettlementKind::Sync) => "Sync".to_string(),
		None => type_label.to_string(),
	}
}

/// One labelled, copyable identifier in the details grid.
#[component]
fn DetailId(label: String, help: String, value: String) -> Element {
	rsx! {
		Kv { label, help,
			Hover { tip: value.clone(), span { class: "mono", "{truncate_id(&value, 8, 8)}" } }
			CopyBtn { value: value.clone() }
		}
	}
}

#[component]
pub fn PaymentDrawer(payment_id: String) -> Element {
	let ctx = use_context::<AppCtx>();
	let loading = ctx.busy(Op::PaymentDetails);
	let now = *ctx.now.read();
	// Only show details that belong to this panel; anything else is a stale reply.
	let details = ctx
		.data
		.read()
		.payment_details
		.clone()
		.filter(|d| d.payment.as_ref().map(|p| p.payment_id == payment_id).unwrap_or(true));
	rsx! {
		SidePanel {
			title: "Payment",
			sub: truncate_id(&payment_id, 10, 10),
			icon: rsx! { Bubble { icon: "receipt", tone: "blue" } },
			onclose: move |_| close_drawer(ctx),
			if loading {
				crate::ui::widgets::Loading { label: "Loading payment details..." }
			} else {
				match details {
					None => rsx! { Empty { icon: "receipt", title: "No payment data." } },
					Some(response) => match response.payment {
						None => rsx! { Empty { icon: "receipt", title: "Payment not found." } },
						Some(payment) => {
							let (status_text, status_tone) = status_style(payment.status);
							let kind_label = payment.kind.as_ref().map(format_payment_kind).unwrap_or_else(|| "Unknown".to_string());
							let ts = payment.latest_update_timestamp;
							rsx! {
								div { class: "drawer-hero",
									match payment.amount_msat {
										Some(a) => rsx! { span { class: "hero-amount", Amount { msat: a, strong: true } } },
										None => rsx! { span { class: "faint", "-" } },
									}
									div { class: "row", style: "gap: 6px;",
										Pill { tone: status_tone, "{status_text}" }
										span { class: "muted small", "{direction_label(payment.direction)} · {kind_label}" }
									}
								}
								div { class: "card inner",
									div { class: "field-label", style: "margin-bottom: 4px;", "Summary" }
									div { class: "kv",
										Kv { label: "Payment ID", help: HELP_PAYMENT_ID, IdCopy { value: payment.payment_id.clone(), head: 10, tail: 10 } }
										Kv { label: "Type", help: HELP_PAYMENT_TYPE, "{kind_label}" }
										Kv { label: "Fee Paid", help: HELP_PAYMENT_FEE,
											match payment.fee_paid_msat {
												Some(f) => rsx! { Amount { msat: f } },
												None => rsx! { "-" },
											}
										}
										Kv { label: "Direction", help: HELP_PAYMENT_DIRECTION, "{direction_label(payment.direction)}" }
										Kv { label: "Last Updated", help: HELP_PAYMENT_TIMESTAMP, Hover { tip: format!("unix: {ts}"), "{local_datetime(ts)} ({relative_short(ts, now)})" } }
									}
								}
								if let Some(kind) = payment.kind {
									div { class: "card inner",
										div { class: "field-label", style: "margin-bottom: 4px;", "Details" }
										PaymentKindDetails { kind }
									}
								}
							}
						},
					},
				}
			}
		}
	}
}

#[component]
fn PaymentKindDetails(kind: PaymentKind) -> Element {
	use sc_rest_client::ldk_server_grpc::types::payment_kind::Kind;

	match kind.kind {
		Some(Kind::Onchain(onchain)) => rsx! {
			div { class: "kv",
				if onchain.txid.is_empty() {
					Kv { label: "Txid", help: HELP_TXID, "-" }
				} else {
					DetailId { label: "Txid", help: HELP_TXID, value: onchain.txid.clone() }
				}
			}
		},
		Some(Kind::Bolt11(bolt11)) => rsx! {
			div { class: "kv",
				DetailId { label: "Payment Hash", help: HELP_PAYMENT_HASH, value: bolt11.hash.clone() }
				if let Some(preimage) = bolt11.preimage.clone() {
					DetailId { label: "Preimage", help: HELP_PREIMAGE, value: preimage }
				}
				if let Some(secret) = bolt11.secret.as_ref() {
					DetailId { label: "Secret", help: HELP_SECRET, value: secret.to_lower_hex_string() }
				}
			}
		},
		Some(Kind::Bolt12Offer(offer)) => rsx! {
			div { class: "kv",
				if let Some(hash) = offer.hash.clone() {
					DetailId { label: "Payment Hash", help: HELP_PAYMENT_HASH, value: hash }
				}
				if let Some(preimage) = offer.preimage.clone() {
					DetailId { label: "Preimage", help: HELP_PREIMAGE, value: preimage }
				}
				if let Some(secret) = offer.secret.as_ref() {
					DetailId { label: "Secret", help: HELP_SECRET, value: secret.to_lower_hex_string() }
				}
				if !offer.offer_id.is_empty() {
					DetailId { label: "Offer ID", help: HELP_OFFER_ID, value: offer.offer_id.clone() }
				}
				if let Some(payer_note) = offer.payer_note.clone() {
					Kv { label: "Payer Note", help: HELP_PAYER_NOTE, "{payer_note}" }
				}
				if let Some(quantity) = offer.quantity {
					Kv { label: "Quantity", help: HELP_QUANTITY, "{quantity}" }
				}
			}
		},
		Some(Kind::Bolt12Refund(refund)) => rsx! {
			div { class: "kv",
				if let Some(hash) = refund.hash.clone() {
					DetailId { label: "Payment Hash", help: HELP_PAYMENT_HASH, value: hash }
				}
				if let Some(preimage) = refund.preimage.clone() {
					DetailId { label: "Preimage", help: HELP_PREIMAGE, value: preimage }
				}
				if let Some(secret) = refund.secret.as_ref() {
					DetailId { label: "Secret", help: HELP_SECRET, value: secret.to_lower_hex_string() }
				}
			}
		},
		Some(Kind::Spontaneous(spontaneous)) => rsx! {
			div { class: "kv",
				DetailId { label: "Payment Hash", help: HELP_PAYMENT_HASH, value: spontaneous.hash.clone() }
				if let Some(preimage) = spontaneous.preimage.clone() {
					DetailId { label: "Preimage", help: HELP_PREIMAGE, value: preimage }
				}
			}
		},
		None => rsx! {},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn csv_export_lists_every_row_with_raw_units() {
		let rows = vec![PaymentRow {
			id: "p1".into(),
			hash: "h1".into(),
			type_label: "BOLT11".into(),
			amount_msat: Some(5_000),
			fee_paid_msat: None,
			direction: 1,
			status: 2,
			timestamp: 1_700_000_000,
		}];
		let csv = payments_csv(&rows);
		let lines: Vec<&str> = csv.lines().collect();
		assert_eq!(lines.len(), 2);
		assert!(lines[0].starts_with("payment_id,type,direction,status"));
		assert_eq!(lines[1], "p1,BOLT11,Outbound,Failed,5000,,1700000000,2023-11-14T22:13:20+00:00,h1");
	}

	#[test]
	fn settlement_overrides_label() {
		assert_eq!(payment_type_label_str("Spontaneous", Some(SettlementKind::Stability)), "Stability");
		assert_eq!(payment_type_label_str("Spontaneous", Some(SettlementKind::Trade)), "Trade");
		assert_eq!(payment_type_label_str("Spontaneous", Some(SettlementKind::Sync)), "Sync");
	}

	#[test]
	fn non_settlement_passes_through() {
		assert_eq!(payment_type_label_str("Spontaneous", None), "Spontaneous");
	}
}
