use dioxus::prelude::*;

use crate::actions;
use crate::format::{amount_entry_preview, unit_label};
use crate::state::{AppCtx, OnchainTab, Op, SendKind};
use crate::ui::widgets::{Bubble, Card, Check, Field, Gate, Icon, InfoTip, Kv, LastId, Pill, Spinner, TextInput};

const HELP_ADDRESS: &str =
	"The Bitcoin address for the current network. Verify it belongs to the intended recipient and network before sending.";
const HELP_AMOUNT: &str = "The on-chain amount to send. Miner fees are separate unless Send All is selected.";
const HELP_SEND_ALL: &str = "Spend the wallet's available on-chain balance, subtracting the miner fee from the output. LDK keeps the reserve it needs to fee-bump anchor channels, so a small amount can stay in the wallet.";
const HELP_FEE_RATE: &str = "Optional miner fee rate in sat/vB. Higher rates can confirm faster but cost more.";
const HELP_LAST_TXID: &str = "The transaction id for the most recently created or broadcast on-chain transaction.";
const HELP_TXID: &str = "The Bitcoin transaction id. Use it to inspect the transaction in a block explorer or node.";
const HELP_ONCHAIN_TOTAL: &str = "The total balance tracked by the node's on-chain wallet.";
const HELP_SPENDABLE: &str =
	"Currently spendable on-chain funds. This excludes funds still waiting on confirmations or kept as reserve.";
const HELP_ANCHOR_RESERVE: &str =
	"Emergency on-chain reserve kept so the node can spend anchor outputs if one of its channels closes.";
const HELP_PENDING_BROADCAST: &str = "The sweep transaction has been prepared or queued but is not yet confirmed on-chain.";
const HELP_BROADCAST_AWAITING_CONFIRMATION: &str =
	"The sweep transaction was broadcast and is waiting for its first confirmation.";
const HELP_AWAITING_THRESHOLD_CONFIRMATIONS: &str =
	"The sweep transaction is confirmed but needs more confirmations before the balance is considered safe or spendable.";

const TABS: [(OnchainTab, &str, &str, &str); 3] = [
	(OnchainTab::Send, "Send", "arrow-up", "blue"),
	(OnchainTab::Receive, "Receive", "arrow-down", "green"),
	(OnchainTab::History, "History", "clock", "gray"),
];

#[component]
pub fn Onchain() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let active = ctx.nav.read().onchain_tab;
	rsx! {
		div { class: "tiles",
			for (tab, label, icon, tone) in TABS {
				button {
					key: "{label}",
					class: if tab == active { "tile active" } else { "tile" },
					"aria-pressed": if tab == active { "true" } else { "false" },
					onclick: move |_| {
						let mut nav = ctx.nav;
						nav.write().onchain_tab = tab;
						// Fetch payments if not already loaded
						if tab == OnchainTab::History && ctx.data.peek().payments.is_none() {
							actions::fetch_payments(ctx, false);
						}
					},
					Bubble { icon, tone }
					"{label}"
				}
			}
		}
		match active {
			OnchainTab::Send => rsx! { div { class: "narrow", SendCard {} } },
			OnchainTab::Receive => rsx! { div { class: "narrow", ReceiveCard {} } },
			OnchainTab::History => rsx! { History {} },
		}
	}
}

#[component]
fn SendCard() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let mut view = ctx.view;
	let form = forms.read().onchain_send.clone();
	let confirmed = view.read().send_all_confirm;
	let unit = unit_label(ctx.unit());
	let preview = amount_entry_preview(&form.amount_sats, ctx.unit(), ctx.price_value());
	let pending = ctx.busy(Op::OnchainSend);
	let last_txid = ctx.results.read().last_txid.clone();
	rsx! {
		Card { title: "Send On-chain",
			div { class: "stack", style: "gap: 16px;",
				Field { label: "Address", help: HELP_ADDRESS,
					TextInput { value: form.address.clone(), mono: true, placeholder: "bc1…", oninput: move |v| forms.write().onchain_send.address = v }
				}
				div { class: "form-grid",
					Field { label: "Amount ({unit})", help: HELP_AMOUNT, preview,
						TextInput { value: form.amount_sats.clone(), disabled: form.send_all, oninput: move |v| forms.write().onchain_send.amount_sats = v }
					}
					Field { label: "Fee Rate (sat/vB, optional)", help: HELP_FEE_RATE,
						TextInput { value: form.fee_rate_sat_per_vb.clone(), oninput: move |v| forms.write().onchain_send.fee_rate_sat_per_vb = v }
					}
				}
				div { class: "toggle-row",
					Check {
						checked: form.send_all,
						label: "Send entire balance",
						help: HELP_SEND_ALL,
						onchange: move |v| {
							forms.write().onchain_send.send_all = v;
							view.write().send_all_confirm = false;
						},
					}
				}
				if form.send_all {
					// Confirm gate for send-all: require a second checkbox before enabling
					div { class: "confirm-box",
						Check { checked: confirmed, danger: true, label: "I understand this sends my entire spendable on-chain balance", onchange: move |v| view.write().send_all_confirm = v }
						button { class: "btn lg danger solid block", disabled: !confirmed || pending, onclick: move |_| actions::review_send(ctx, SendKind::Onchain),
							if pending { Spinner {} "Sending..." } else { Icon { name: "arrow-up", size: 18 } "Send All" }
						}
					}
				} else {
					button { class: "btn lg primary block", disabled: pending, onclick: move |_| actions::review_send(ctx, SendKind::Onchain),
						if pending { Spinner {} "Sending..." } else { Icon { name: "arrow-up", size: 18 } "Send" }
					}
				}
				if let Some(txid) = last_txid {
					LastId { label: "Last TXID:", help: HELP_LAST_TXID, value: txid, keep: 12 }
				}
			}
		}
	}
}

#[component]
fn ReceiveCard() -> Element {
	let ctx = use_context::<AppCtx>();
	let pending = ctx.busy(Op::OnchainReceive);
	let address = ctx.results.read().onchain_address.clone();
	rsx! {
		Card { title: "Receive On-chain",
			div { class: "stack", style: "gap: 16px;",
				button { class: "btn lg primary block", disabled: pending, onclick: move |_| actions::generate_onchain_address(ctx),
					if pending { Spinner {} "Generating..." } else { Icon { name: "arrow-down", size: 18 } "Generate Address" }
				}
				if let Some(address) = address {
					div { class: "result",
						div { class: "row between",
							span { class: "field-label", "Address" InfoTip { text: HELP_ADDRESS } }
							button { class: "btn sm", onclick: {
								let address = address.clone();
								move |_| actions::copy(ctx, &address)
							}, Icon { name: "copy", size: 14 } "Copy Address" }
						}
						div { class: "value", style: "font-size: 15px;", "{address}" }
					}
				}
			}
		}
	}
}

#[component]
fn History() -> Element {
	use sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;

	let ctx = use_context::<AppCtx>();
	let balances = ctx.data.read().balances.clone();
	let last_txid = ctx.results.read().last_txid.clone();
	rsx! {
		if let Some(b) = balances {
			div { class: "grid-2",
				Card { title: "Summary",
					div { class: "kv",
						Kv { label: "Total Balance", help: HELP_ONCHAIN_TOTAL, span { class: "num strong", "{ctx.fmt_sats(b.total_onchain_balance_sats)}" } }
						Kv { label: "Spendable", help: HELP_SPENDABLE, span { class: "num", "{ctx.fmt_sats(b.spendable_onchain_balance_sats)}" } }
						if b.total_anchor_channels_reserve_sats > 0 {
							Kv { label: "Anchor Reserve", help: HELP_ANCHOR_RESERVE, span { class: "num", "{ctx.fmt_sats(b.total_anchor_channels_reserve_sats)}" } }
						}
					}
				}
				if !b.pending_balances_from_channel_closures.is_empty() {
					Card { title: "Pending Sweeps",
						div { class: "stack",
							for (i, sweep) in b.pending_balances_from_channel_closures.iter().filter_map(|s| s.balance_type.clone()).enumerate() {
								div { key: "{i}", class: "card inner stack tight",
									match sweep {
										BalanceType::PendingBroadcast(s) => rsx! {
											div { class: "row between",
												span { class: "row nowrap", style: "gap: 6px;", Pill { tone: "warning", "Pending Broadcast" } InfoTip { text: HELP_PENDING_BROADCAST } }
												span { class: "num strong", "{ctx.fmt_sats(s.amount_satoshis)}" }
											}
										},
										BalanceType::BroadcastAwaitingConfirmation(s) => rsx! {
											div { class: "row between",
												span { class: "row nowrap", style: "gap: 6px;", Pill { tone: "warning", "Awaiting Confirmation" } InfoTip { text: HELP_BROADCAST_AWAITING_CONFIRMATION } }
												span { class: "num strong", "{ctx.fmt_sats(s.amount_satoshis)}" }
											}
											LastId { label: "TXID:", help: HELP_TXID, value: s.latest_spending_txid.clone() }
										},
										BalanceType::AwaitingThresholdConfirmations(s) => rsx! {
											div { class: "row between",
												span { class: "row nowrap", style: "gap: 6px;", Pill { tone: "success", "Awaiting Threshold" } InfoTip { text: HELP_AWAITING_THRESHOLD_CONFIRMATIONS } }
												span { class: "num strong", "{ctx.fmt_sats(s.amount_satoshis)}" }
											}
											span { class: "small muted", "Confirmed at height {s.confirmation_height}" }
										},
									}
								}
							}
						}
					}
				}
			}
		}
		// Neutral informational callout — not an error.
		Card { title: "Transaction History",
			div { class: "notice",
				Icon { name: "info", size: 16 }
				div { class: "stack tight",
					span { "Full on-chain transaction history is not yet available." }
					span { "ldk-node does not currently expose BDK wallet transaction history." }
				}
			}
			if let Some(txid) = last_txid {
				div { style: "margin-top: 12px;",
					LastId { label: "Last Sent TXID:", help: HELP_LAST_TXID, value: txid }
				}
			}
		}
	}
}
