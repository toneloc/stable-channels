use dioxus::prelude::*;

use crate::actions;
use crate::format::{format_sats, format_usd};
use crate::state::{AppCtx, DisplayUnit, Op};
use crate::ui::widgets::{Card, Empty, Gate, Hover, InfoTip, Kv, RefreshBtn, Stat, Th};

const HELP_ONCHAIN_TOTAL: &str = "The total balance tracked by the node's on-chain wallet.";
const HELP_ONCHAIN_SPENDABLE: &str =
	"The on-chain funds that are currently spendable after confirmation requirements and reserves.";
const HELP_SPENDABLE: &str =
	"Currently spendable on-chain funds. This excludes funds still waiting on confirmations or kept as reserve.";
const HELP_ANCHOR_RESERVE: &str =
	"Emergency on-chain reserve kept so the node can spend anchor outputs if one of its channels closes.";
const HELP_LIGHTNING_TOTAL: &str =
	"Total balance claimable across Lightning channels. This is not the same as immediately sendable capacity.";
const HELP_BALANCE_TYPE: &str = "The claim or balance state for this Lightning balance row.";
const HELP_BALANCE_CHANNEL: &str =
	"The channel ID associated with this balance or sweep when one is available.";
const HELP_BALANCE_AMOUNT: &str =
	"The funds represented by this row, shown in the selected display unit.";
const HELP_BALANCE_EXTRA: &str =
	"Additional block heights, txids, or timing details needed to understand when funds become spendable.";
const HELP_PENDING_SWEEP_BALANCES: &str =
	"On-chain outputs the node is sweeping from channel closures or claim transactions.";
const HELP_CLAIMABLE_ON_CHANNEL_CLOSE: &str =
	"Funds that could be claimed if the channel were force-closed now, less on-chain fees. This does not include unconfirmed splice changes.";
const HELP_AWAITING_CONFIRMATIONS: &str =
	"The channel is closed and this balance is ours, but it needs enough on-chain confirmations before becoming spendable.";
const HELP_CONTENTIOUS_CLAIMABLE: &str =
	"The channel is closed and this balance should be ours, but our spending transaction must confirm before a timeout that could let the counterparty claim it.";
const HELP_MAYBE_TIMEOUT_CLAIMABLE_HTLC: &str =
	"An HTLC we sent that may become claimable after its timeout if the counterparty does not claim it first with the preimage.";
const HELP_MAYBE_PREIMAGE_CLAIMABLE_HTLC: &str =
	"An HTLC we received that is claimable only if we learn and use the payment preimage before the timeout.";
const HELP_COUNTERPARTY_REVOKED_OUTPUT: &str =
	"The counterparty broadcast a revoked commitment transaction, allowing this node to claim penalty outputs from it.";
const HELP_PENDING_BROADCAST: &str =
	"The sweep transaction has been prepared or queued but is not yet confirmed on-chain.";
const HELP_BROADCAST_AWAITING_CONFIRMATION: &str =
	"The sweep transaction was broadcast and is waiting for its first confirmation.";
const HELP_AWAITING_THRESHOLD_CONFIRMATIONS: &str =
	"The sweep transaction is confirmed but needs more confirmations before the balance is considered safe or spendable.";

/// Split a total into (number, unit, secondary line) for the hero figure.
fn hero_parts(sats: u64, unit: DisplayUnit, price: Option<f64>) -> (String, &'static str, String) {
	let btc = sats as f64 / 100_000_000.0;
	match (unit, price.filter(|p| *p > 0.0)) {
		(DisplayUnit::Usd, Some(p)) => (format_usd(btc * p), "USD", format!("{:.8} BTC", btc)),
		(DisplayUnit::Btc, p) => (
			format!("{:.8}", btc),
			"BTC",
			match p {
				Some(p) => format!("{} sats · ≈ {}", format_sats(sats), format_usd(btc * p)),
				None => format!("{} sats", format_sats(sats)),
			},
		),
		(_, p) => (
			format_sats(sats),
			"sats",
			match p {
				Some(p) => format!("≈ {} · {:.8} BTC", format_usd(btc * p), btc),
				None => format!("{:.8} BTC", btc),
			},
		),
	}
}

#[component]
pub fn Balances() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let loading = ctx.busy(Op::Balances);
	let Some(b) = ctx.data.read().balances.clone() else {
		return rsx! {
			Card {
				if loading {
					Empty { icon: "wallet", title: "Loading balances...", crate::ui::widgets::Spinner { large: true } }
				} else {
					Empty { icon: "wallet", title: "No balance data", hint: "Click Refresh to load",
						RefreshBtn { busy: loading, onclick: move |_| actions::fetch_balances(ctx) }
					}
				}
			}
		};
	};
	let fmt = |sats: u64| ctx.fmt_sats(sats);
	let total_onchain = b.total_onchain_balance_sats;
	let spendable = b.spendable_onchain_balance_sats;
	let reserve = b.total_anchor_channels_reserve_sats;
	let total_lightning = b.total_lightning_balance_sats;
	let total = total_onchain.saturating_add(total_lightning);
	let (hero_value, hero_unit, hero_sub) = hero_parts(total, ctx.unit(), ctx.price_value());
	let onchain_frac = if total == 0 { 0.5 } else { total_onchain as f64 / total as f64 };
	let onchain_pct = (onchain_frac * 100.0).round();
	let lightning_pct = 100.0 - onchain_pct;
	let lightning_rows: Vec<[String; 4]> = b
		.lightning_balances
		.iter()
		.filter_map(|balance| balance.balance_type.as_ref().map(|bt| lightning_balance_row(bt, &fmt)))
		.collect();
	let pending_sweeps: Vec<(usize, String, &'static str)> = b
		.pending_balances_from_channel_closures
		.iter()
		.enumerate()
		.filter_map(|(i, sweep)| {
			sweep.balance_type.as_ref().map(|bt| {
				let (text, help) = pending_sweep_text(bt, &fmt);
				(i, text, help)
			})
		})
		.collect();

	rsx! {
		Card { class: "hero",
			span { class: "hero-label", "Total Balance" }
			div { class: "hero-value", "{hero_value}" span { class: "hero-unit", "{hero_unit}" } }
			span { class: "hero-sub", "{hero_sub}" }
			div { class: "split",
				div { class: "split-bar", "aria-hidden": "true",
					div { class: "seg-a", style: "width: {onchain_pct}%;" }
					div { class: "seg-b", style: "width: {lightning_pct}%;" }
				}
				div { class: "split-legend",
					div { class: "stack tight", style: "align-items: flex-start; gap: 2px;",
						span { class: "label", style: "color: var(--orange-text);", span { class: "coin sm", "₿" } "On-chain" }
						span { class: "num", "{fmt(total_onchain)}" }
					}
					div { class: "stack tight", style: "align-items: flex-end; gap: 2px;",
						span { class: "label", style: "color: var(--blue-text);", "Lightning" crate::ui::widgets::Icon { name: "zap", size: 14 } }
						span { class: "num", "{fmt(total_lightning)}" }
					}
				}
			}
		}
		div { class: "row between",
			span { class: "muted small", "Totals include funds that are not yet spendable." }
			div { class: "row", RefreshBtn { busy: loading, onclick: move |_| actions::fetch_balances(ctx), op: Op::Balances } }
		}
		div { class: "grid-3",
			Stat { title: "On-chain Spendable", help: HELP_ONCHAIN_SPENDABLE, value: fmt(spendable), sub: format!("reserve {} | total {}", fmt(reserve), fmt(total_onchain)) }
			Stat { title: "Lightning Total", help: HELP_LIGHTNING_TOTAL, value: fmt(total_lightning) }
			Stat { title: "Anchor Reserve", help: HELP_ANCHOR_RESERVE, value: fmt(reserve), sub: "Kept for anchor-output fee bumping" }
		}
		div { class: "grid-2",
			Card { title: "On-chain Balance",
				div { class: "kv",
					Kv { label: "Total", help: HELP_ONCHAIN_TOTAL, Hover { tip: format!("{} sats", format_sats(total_onchain)), span { class: "num", "{fmt(total_onchain)}" } } }
					Kv { label: "Spendable", help: HELP_SPENDABLE, Hover { tip: format!("{} sats", format_sats(spendable)), span { class: "num", "{fmt(spendable)}" } } }
					Kv { label: "Anchor Reserve", help: HELP_ANCHOR_RESERVE, Hover { tip: format!("{} sats", format_sats(reserve)), span { class: "num", "{fmt(reserve)}" } } }
				}
			}
			Card { title: "Lightning Balance", class: "flush",
				div { style: "padding: 0 20px 6px;",
					div { class: "kv",
						Kv { label: "Total", help: HELP_LIGHTNING_TOTAL, Hover { tip: format!("{} sats", format_sats(total_lightning)), span { class: "num", "{fmt(total_lightning)}" } } }
					}
				}
				if !lightning_rows.is_empty() {
					div { class: "toolbar", style: "border-top: 1px solid var(--border);", span { class: "count", "Details ({lightning_rows.len()} items)" } }
					div { class: "table-wrap",
						table { class: "table", style: "min-width: 600px;",
							thead {
								tr {
									Th { label: "Type", help: HELP_BALANCE_TYPE }
									Th { label: "Channel", help: HELP_BALANCE_CHANNEL }
									Th { label: "Amount", help: HELP_BALANCE_AMOUNT, class: "right" }
									Th { label: "Extra", help: HELP_BALANCE_EXTRA }
								}
							}
							tbody {
								for (i, row) in lightning_rows.into_iter().enumerate() {
									tr { key: "{i}",
										td {
											span { class: "row nowrap", style: "gap: 6px;",
												"{row[0]}"
												if let Some(help) = lightning_balance_help(&row[0]) {
													InfoTip { text: help }
												}
											}
										}
										td { span { class: "mono", "{row[1]}" } }
										td { class: "right num", "{row[2]}" }
										td { class: "muted", "{row[3]}" }
									}
								}
							}
						}
					}
				}
			}
		}
		if !pending_sweeps.is_empty() {
			Card { title: "Pending Sweep Balances", help: HELP_PENDING_SWEEP_BALANCES, sub: "Sweep outputs",
				div { class: "grid-3",
					for (i, text, help) in pending_sweeps {
						div { key: "{i}", class: "card inner stack tight",
							div { class: "row nowrap", style: "gap: 6px;", span { class: "strong", "Sweep #{i + 1}" } InfoTip { text: help } }
							for (j, line) in text.lines().map(str::to_string).enumerate() {
								span { key: "{j}", class: "small", "{line}" }
							}
						}
					}
				}
			}
		}
	}
}

fn lightning_balance_row(
	balance: &sc_rest_client::ldk_server_grpc::types::lightning_balance::BalanceType,
	fmt: &dyn Fn(u64) -> String,
) -> [String; 4] {
	use sc_rest_client::ldk_server_grpc::types::lightning_balance::BalanceType;

	match balance {
		BalanceType::ClaimableOnChannelClose(b) => [
			"Claimable on Channel Close".to_string(),
			crate::format::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			String::new(),
		],
		BalanceType::ClaimableAwaitingConfirmations(b) => [
			"Awaiting Confirmations".to_string(),
			crate::format::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			format!("Confirmation Height: {}", b.confirmation_height),
		],
		BalanceType::ContentiousClaimable(b) => [
			"Contentious Claimable".to_string(),
			crate::format::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			format!("Timeout Height: {}", b.timeout_height),
		],
		BalanceType::MaybeTimeoutClaimableHtlc(b) => [
			"Maybe Timeout Claimable HTLC".to_string(),
			crate::format::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			format!("Claimable Height: {}", b.claimable_height),
		],
		BalanceType::MaybePreimageClaimableHtlc(b) => [
			"Maybe Preimage Claimable HTLC".to_string(),
			crate::format::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			format!("Expiry Height: {}", b.expiry_height),
		],
		BalanceType::CounterpartyRevokedOutputClaimable(b) => [
			"Counterparty Revoked Output".to_string(),
			crate::format::truncate_id(&b.channel_id, 8, 8),
			fmt(b.amount_satoshis),
			String::new(),
		],
	}
}

fn pending_sweep_text(
	balance: &sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType,
	fmt: &dyn Fn(u64) -> String,
) -> (String, &'static str) {
	use sc_rest_client::ldk_server_grpc::types::pending_sweep_balance::BalanceType;

	match balance {
		BalanceType::PendingBroadcast(b) => {
			let ch_line = b
			    .channel_id
			    .as_ref()
			    .map(|c| format!("Channel: {}\n", crate::format::truncate_id(c, 8, 8)))
			    .unwrap_or_default();
			(
			    format!(
			        "Type: Pending Broadcast\n{}Amount: {}",
			        ch_line,
			        fmt(b.amount_satoshis)
			    ),
			    HELP_PENDING_BROADCAST,
			)
		}
		BalanceType::BroadcastAwaitingConfirmation(b) => {
			let ch_line = b
			    .channel_id
			    .as_ref()
			    .map(|c| format!("Channel: {}\n", crate::format::truncate_id(c, 8, 8)))
			    .unwrap_or_default();
			(
			format!(
				"Type: Broadcast Awaiting Confirmation\n{}Amount: {}\nTXID: {}",
				ch_line,
				fmt(b.amount_satoshis),
				crate::format::truncate_id(&b.latest_spending_txid, 8, 8)
			    ),
			    HELP_BROADCAST_AWAITING_CONFIRMATION,
			)
		}
		BalanceType::AwaitingThresholdConfirmations(b) => {
			let ch_line = b
			    .channel_id
			    .as_ref()
			    .map(|c| format!("Channel: {}\n", crate::format::truncate_id(c, 8, 8)))
			    .unwrap_or_default();
			(
			format!(
				"Type: Awaiting Threshold Confirmations\n{}Amount: {}\nConfirmed at height: {}",
				ch_line,
				fmt(b.amount_satoshis),
				b.confirmation_height
			    ),
			    HELP_AWAITING_THRESHOLD_CONFIRMATIONS,
			)
		}
	}
}

fn lightning_balance_help(label: &str) -> Option<&'static str> {
	match label {
		"Claimable on Channel Close" => Some(HELP_CLAIMABLE_ON_CHANNEL_CLOSE),
		"Awaiting Confirmations" => Some(HELP_AWAITING_CONFIRMATIONS),
		"Contentious Claimable" => Some(HELP_CONTENTIOUS_CLAIMABLE),
		"Maybe Timeout Claimable HTLC" => Some(HELP_MAYBE_TIMEOUT_CLAIMABLE_HTLC),
		"Maybe Preimage Claimable HTLC" => Some(HELP_MAYBE_PREIMAGE_CLAIMABLE_HTLC),
		"Counterparty Revoked Output" => Some(HELP_COUNTERPARTY_REVOKED_OUTPUT),
		_ => None,
	}
}
