use dioxus::prelude::*;

use crate::actions;
use crate::format::{format_amount_msat, format_sats, truncate_id};
use crate::state::{AppCtx, DisplayUnit, Op, SendKind};
use crate::ui::close_dialog;
use crate::ui::widgets::{Bubble, Hover, Icon, Kv, Modal, Peer, Spinner};

/// Amount line for the review: display unit, then sats and ≈USD.
fn describe_msat(ctx: AppCtx, msat: u64) -> String {
	let price = ctx.price_value();
	let primary = ctx.fmt_msat(msat);
	let sats = format!("{} sats", format_sats(msat / 1000));
	match (ctx.unit(), price.filter(|p| *p > 0.0)) {
		(DisplayUnit::Sats, Some(_)) => format!("{primary} · ≈ {}", format_amount_msat(msat, DisplayUnit::Usd, price)),
		(DisplayUnit::Sats, None) => primary,
		(DisplayUnit::Usd, _) => format!("{primary} · {sats}"),
		(DisplayUnit::Btc, Some(_)) => format!("{primary} · {sats} · ≈ {}", format_amount_msat(msat, DisplayUnit::Usd, price)),
		(DisplayUnit::Btc, None) => format!("{primary} · {sats}"),
	}
}

/// Review step shown before any payment leaves the node.
#[component]
pub fn ConfirmSendDialog(kind: SendKind) -> Element {
	let ctx = use_context::<AppCtx>();
	let forms = ctx.forms.read().clone();
	let (title, op, icon, tone) = match kind {
		SendKind::Bolt11 => ("Review BOLT11 payment", Op::Bolt11Send, "arrow-up", "blue"),
		SendKind::Bolt12 => ("Review BOLT12 payment", Op::Bolt12Send, "arrow-up-right", "orange"),
		SendKind::Keysend => ("Review keysend", Op::SpontaneousSend, "zap", "gray"),
		SendKind::Onchain => ("Review on-chain send", Op::OnchainSend, "arrow-up", "blue"),
	};
	let pending = ctx.busy(op);
	let msat = |input: &str| ctx.parse_amount_msat(input);
	let rows: Vec<(&'static str, String, Option<String>)> = match kind {
		SendKind::Bolt11 => {
			let f = &forms.bolt11_send;
			let invoice = f.invoice.trim().to_string();
			vec![
				("Invoice", truncate_id(&invoice, 16, 12), Some(invoice)),
				(
					"Amount",
					msat(&f.amount_msat).map(|m| describe_msat(ctx, m)).unwrap_or_else(|| "Set by the invoice".into()),
					None,
				),
			]
		},
		SendKind::Bolt12 => {
			let f = &forms.bolt12_send;
			let offer = f.offer.trim().to_string();
			let mut rows = vec![
				("Offer", truncate_id(&offer, 16, 12), Some(offer)),
				("Amount", msat(&f.amount_msat).map(|m| describe_msat(ctx, m)).unwrap_or_else(|| "Set by the offer".into()), None),
			];
			if !f.quantity.trim().is_empty() {
				rows.push(("Quantity", f.quantity.trim().to_string(), None));
			}
			if !f.payer_note.trim().is_empty() {
				rows.push(("Payer note", f.payer_note.trim().to_string(), None));
			}
			rows
		},
		SendKind::Keysend => {
			let f = &forms.spontaneous_send;
			vec![("Amount", msat(&f.amount_msat).map(|m| describe_msat(ctx, m)).unwrap_or_default(), None)]
		},
		SendKind::Onchain => {
			let f = &forms.onchain_send;
			let address = f.address.trim().to_string();
			vec![
				("Address", address.clone(), None),
				(
					"Amount",
					if f.send_all {
						"Entire spendable on-chain balance (the fee is deducted from it)".to_string()
					} else {
						ctx.parse_amount_sats(&f.amount_sats).map(|s| describe_msat(ctx, s * 1000)).unwrap_or_default()
					},
					None,
				),
				(
					"Fee rate",
					match f.fee_rate_sat_per_vb.trim() {
						"" => "Node default".to_string(),
						rate => format!("{rate} sat/vB"),
					},
					None,
				),
			]
		},
	};
	let recipient = (kind == SendKind::Keysend).then(|| forms.spontaneous_send.node_id.trim().to_string());
	rsx! {
		Modal {
			title,
			sub: "Check the details. Sent payments cannot be reversed.",
			icon: rsx! { Bubble { icon, tone } },
			onclose: move |_| close_dialog(ctx),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| close_dialog(ctx), "Back" }
				button { class: "btn primary", disabled: pending, onclick: move |_| actions::confirm_send(ctx, kind),
					if pending { Spinner {} "Sending..." } else { Icon { name: "check", size: 16 } "Confirm & send" }
				}
			},
			div { class: "kv",
				if let Some(node_id) = recipient {
					Kv { label: "Recipient", Peer { node_id, keep: 10 } }
				}
				for (label, value, full) in rows {
					Kv { key: "{label}", label,
						match full {
							Some(full) => rsx! { Hover { tip: full, span { class: "mono break", "{value}" } } },
							None => rsx! { span { class: "break strong", "{value}" } },
						}
					}
				}
			}
		}
	}
}
