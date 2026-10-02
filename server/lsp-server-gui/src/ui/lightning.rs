use dioxus::prelude::*;

use crate::actions;
use crate::format::{amount_entry_preview, unit_label};
use crate::state::{AppCtx, LightningTab, Op, SendKind};
use crate::ui::widgets::{Bubble, Card, Field, Gate, Icon, LastId, Spinner, TextArea, TextInput};

const HELP_BOLT11_INVOICE: &str = "A one-time Lightning invoice that can include amount, description, expiry, routing hints, and payment hash. Enter an amount only when the invoice is zero-amount.";
const HELP_GENERATED_BOLT11_INVOICE: &str = "A one-time Lightning invoice another payer can settle before it expires.";
const HELP_BOLT12_OFFER: &str =
	"A reusable Lightning offer. The payer requests an invoice from the recipient and can include amount, quantity, or a payer note when the offer allows it.";
const HELP_GENERATED_BOLT12_OFFER: &str =
	"A reusable offer that another wallet can use to request an invoice and pay this node.";
const HELP_AMOUNT: &str =
	"The payment amount in the selected display unit. Lightning sends are tracked internally in millisatoshis.";
const HELP_ZERO_AMOUNT: &str = "Use this only when the BOLT11 invoice does not specify an amount.";
const HELP_DESCRIPTION: &str = "Human-readable payment description included in the invoice or offer.";
const HELP_EXPIRY: &str = "How long the invoice or offer should remain payable, in seconds.";
const HELP_QUANTITY: &str =
	"The number of offered items or units requested when the BOLT12 offer supports quantities.";
const HELP_PAYER_NOTE: &str =
	"Optional note sent with the BOLT12 payment request. Avoid secrets or sensitive information.";
const HELP_NODE_ID: &str = "The recipient's Lightning node public key in hex.";
const HELP_KEYSEND: &str =
	"A spontaneous Lightning payment that includes the payment secret material needed by the recipient to settle without a prior invoice.";
const HELP_LAST_PAYMENT_ID: &str = "The local identifier used to look up this payment record later.";

const TABS: [(LightningTab, &str, &str, &str); 5] = [
	(LightningTab::Bolt11Send, "BOLT11 Send", "arrow-up", "blue"),
	(LightningTab::Bolt11Receive, "BOLT11 Receive", "arrow-down", "green"),
	(LightningTab::Bolt12Send, "BOLT12 Send", "arrow-up-right", "orange"),
	(LightningTab::Bolt12Receive, "BOLT12 Receive", "arrow-down-left", "purple"),
	(LightningTab::SpontaneousSend, "Keysend", "zap", "gray"),
];

#[component]
pub fn Lightning() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let active = ctx.nav.read().lightning_tab;
	rsx! {
		div { class: "tiles",
			for (tab, label, icon, tone) in TABS {
				button {
					key: "{label}",
					class: if tab == active { "tile active" } else { "tile" },
					"aria-pressed": if tab == active { "true" } else { "false" },
					onclick: move |_| {
						let mut nav = ctx.nav;
						nav.write().lightning_tab = tab;
					},
					Bubble { icon, tone }
					"{label}"
				}
			}
		}
		div { class: "narrow",
			match active {
				LightningTab::Bolt11Send => rsx! { Bolt11Send {} },
				LightningTab::Bolt11Receive => rsx! { Bolt11Receive {} },
				LightningTab::Bolt12Send => rsx! { Bolt12Send {} },
				LightningTab::Bolt12Receive => rsx! { Bolt12Receive {} },
				LightningTab::SpontaneousSend => rsx! { SpontaneousSend {} },
			}
		}
	}
}

/// Primary submit button that shows a spinner and label while pending.
#[component]
fn Submit(pending: bool, label: String, pending_label: String, icon: &'static str, onclick: EventHandler<MouseEvent>) -> Element {
	rsx! {
		button { class: "btn lg primary block", disabled: pending, onclick: move |e| onclick.call(e),
			if pending {
				Spinner {}
				"{pending_label}"
			} else {
				Icon { name: icon, size: 18 }
				"{label}"
			}
		}
	}
}

#[component]
fn LastPayment() -> Element {
	let ctx = use_context::<AppCtx>();
	let last = ctx.results.read().last_payment_id.clone();
	rsx! {
		if let Some(payment_id) = last {
			LastId { label: "Last Payment ID:", help: HELP_LAST_PAYMENT_ID, value: payment_id }
		}
	}
}

/// Read-only generated invoice/offer with a copy button.
#[component]
fn Generated(label: String, help: String, value: String, copy_label: String) -> Element {
	let ctx = use_context::<AppCtx>();
	rsx! {
		div { class: "result",
			div { class: "row between",
				span { class: "field-label", "{label}" crate::ui::widgets::InfoTip { text: help } }
				button { class: "btn sm", onclick: move |_| crate::actions::copy(ctx, &value), Icon { name: "copy", size: 14 } "{copy_label}" }
			}
			div { class: "value", "{value}" }
		}
	}
}

#[component]
fn Bolt11Send() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().bolt11_send.clone();
	let unit = unit_label(ctx.unit());
	let preview = amount_entry_preview(&form.amount_msat, ctx.unit(), ctx.price_value());
	rsx! {
		Card { title: "Pay BOLT11 Invoice",
			div { class: "stack", style: "gap: 16px;",
				Field { label: "Invoice", help: HELP_BOLT11_INVOICE,
					TextArea { value: form.invoice.clone(), rows: 3, placeholder: "lnbc…", oninput: move |v| forms.write().bolt11_send.invoice = v }
				}
				Field { label: "Amount ({unit}, for zero-amount invoices)", help: HELP_ZERO_AMOUNT, preview,
					TextInput { value: form.amount_msat.clone(), oninput: move |v| forms.write().bolt11_send.amount_msat = v }
				}
				Submit { pending: ctx.busy(Op::Bolt11Send), label: "Pay Invoice", pending_label: "Sending...", icon: "arrow-up", onclick: move |_| actions::review_send(ctx, SendKind::Bolt11) }
				LastPayment {}
			}
		}
	}
}

#[component]
fn Bolt11Receive() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().bolt11_receive.clone();
	let unit = unit_label(ctx.unit());
	let preview = amount_entry_preview(&form.amount_msat, ctx.unit(), ctx.price_value());
	let invoice = ctx.results.read().generated_invoice.clone();
	rsx! {
		Card { title: "Generate BOLT11 Invoice",
			div { class: "stack", style: "gap: 16px;",
				Field { label: "Amount ({unit}, optional)", help: HELP_AMOUNT, preview,
					TextInput { value: form.amount_msat.clone(), oninput: move |v| forms.write().bolt11_receive.amount_msat = v }
				}
				Field { label: "Description", help: HELP_DESCRIPTION,
					TextInput { value: form.description.clone(), oninput: move |v| forms.write().bolt11_receive.description = v }
				}
				Field { label: "Expiry (seconds)", help: HELP_EXPIRY, hint: "Defaults to 86400 (one day)",
					TextInput { value: form.expiry_secs.clone(), placeholder: "86400", oninput: move |v| forms.write().bolt11_receive.expiry_secs = v }
				}
				Submit { pending: ctx.busy(Op::Bolt11Receive), label: "Generate Invoice", pending_label: "Generating...", icon: "arrow-down", onclick: move |_| actions::generate_bolt11_invoice(ctx) }
				if let Some(invoice) = invoice {
					Generated { label: "Generated Invoice", help: HELP_GENERATED_BOLT11_INVOICE, value: invoice, copy_label: "Copy Invoice" }
				}
			}
		}
	}
}

#[component]
fn Bolt12Send() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().bolt12_send.clone();
	let unit = unit_label(ctx.unit());
	let preview = amount_entry_preview(&form.amount_msat, ctx.unit(), ctx.price_value());
	rsx! {
		Card { title: "Pay BOLT12 Offer",
			div { class: "stack", style: "gap: 16px;",
				Field { label: "Offer", help: HELP_BOLT12_OFFER,
					TextArea { value: form.offer.clone(), rows: 3, placeholder: "lno…", oninput: move |v| forms.write().bolt12_send.offer = v }
				}
				div { class: "form-grid",
					Field { label: "Amount ({unit}, optional)", help: HELP_AMOUNT, preview,
						TextInput { value: form.amount_msat.clone(), oninput: move |v| forms.write().bolt12_send.amount_msat = v }
					}
					Field { label: "Quantity (optional)", help: HELP_QUANTITY,
						TextInput { value: form.quantity.clone(), oninput: move |v| forms.write().bolt12_send.quantity = v }
					}
				}
				Field { label: "Payer Note (optional)", help: HELP_PAYER_NOTE,
					TextInput { value: form.payer_note.clone(), oninput: move |v| forms.write().bolt12_send.payer_note = v }
				}
				Submit { pending: ctx.busy(Op::Bolt12Send), label: "Pay Offer", pending_label: "Sending...", icon: "arrow-up-right", onclick: move |_| actions::review_send(ctx, SendKind::Bolt12) }
				LastPayment {}
			}
		}
	}
}

#[component]
fn Bolt12Receive() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().bolt12_receive.clone();
	let unit = unit_label(ctx.unit());
	let preview = amount_entry_preview(&form.amount_msat, ctx.unit(), ctx.price_value());
	let offer = ctx.results.read().generated_offer.clone();
	rsx! {
		Card { title: "Generate BOLT12 Offer",
			div { class: "stack", style: "gap: 16px;",
				Field { label: "Description (required)", help: HELP_DESCRIPTION,
					TextInput { value: form.description.clone(), oninput: move |v| forms.write().bolt12_receive.description = v }
				}
				div { class: "form-grid",
					Field { label: "Amount ({unit}, optional)", help: HELP_AMOUNT, preview,
						TextInput { value: form.amount_msat.clone(), oninput: move |v| forms.write().bolt12_receive.amount_msat = v }
					}
					Field { label: "Expiry (seconds, optional)", help: HELP_EXPIRY,
						TextInput { value: form.expiry_secs.clone(), oninput: move |v| forms.write().bolt12_receive.expiry_secs = v }
					}
					Field { label: "Quantity (optional)", help: HELP_QUANTITY,
						TextInput { value: form.quantity.clone(), oninput: move |v| forms.write().bolt12_receive.quantity = v }
					}
				}
				Submit { pending: ctx.busy(Op::Bolt12Receive), label: "Generate Offer", pending_label: "Generating...", icon: "arrow-down-left", onclick: move |_| actions::generate_bolt12_offer(ctx) }
				if let Some(offer) = offer {
					Generated { label: "Generated Offer", help: HELP_GENERATED_BOLT12_OFFER, value: offer, copy_label: "Copy Offer" }
				}
			}
		}
	}
}

#[component]
fn SpontaneousSend() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().spontaneous_send.clone();
	let unit = unit_label(ctx.unit());
	let preview = amount_entry_preview(&form.amount_msat, ctx.unit(), ctx.price_value());
	rsx! {
		Card { title: "Spontaneous Payment (Keysend)", sub: "Payment type: Keysend", help: HELP_KEYSEND,
			div { class: "stack", style: "gap: 16px;",
				Field { label: "Node ID (hex)", help: HELP_NODE_ID,
					TextInput { value: form.node_id.clone(), mono: true, oninput: move |v| forms.write().spontaneous_send.node_id = v }
				}
				Field { label: "Amount ({unit})", help: HELP_AMOUNT, preview,
					TextInput { value: form.amount_msat.clone(), oninput: move |v| forms.write().spontaneous_send.amount_msat = v }
				}
				Submit { pending: ctx.busy(Op::SpontaneousSend), label: "Send Keysend", pending_label: "Sending...", icon: "zap", onclick: move |_| actions::review_send(ctx, SendKind::Keysend) }
				LastPayment {}
			}
		}
	}
}
