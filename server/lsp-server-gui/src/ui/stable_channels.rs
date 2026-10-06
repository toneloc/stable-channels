use dioxus::prelude::*;

use crate::actions;
use crate::format::format_usd;
use crate::health::{stable_drift, Drift};
use crate::state::{AppCtx, Drawer, Op};
use crate::ui::channels::drift_pill;
use crate::ui::open_drawer;
use crate::ui::widgets::{Amount, Card, Empty, Field, Gate, Icon, Peer, Pill, RefreshBtn, Spinner, TextInput, Th};

const HELP_CHANNEL_ID: &str = "The Lightning channel tied to this stable-channel record.";
const HELP_DRIFT: &str =
	"Backing valued at the latest price, and how far that is from the USD target: green above it, red below. Open the channel to see when a stability payment happens.";
const HELP_COUNTERPARTY: &str = "The peer node public key for the stable-channel counterparty.";
const HELP_LOCATION: &str = "Where this user's wallet last connected from: country from the LSP's offline IP database, and the IP. Tor and VPN users show their exit, not their real location.";
const HELP_STABLE_USD: &str = "The USD target value this stable channel is intended to maintain.";
const HELP_BACKING: &str = "Bitcoin backing currently associated with the stable-channel target.";
const HELP_ROLE: &str = "Whether this side is acting as the stable-value receiver or provider.";
const HELP_NOTE: &str = "Operator note stored with the stable-channel record. Do not use this for secrets.";

#[derive(Clone, PartialEq)]
struct StableRow {
	channel_id: String,
	user_channel_id: String,
	counterparty: String,
	expected_usd: f64,
	backing_sats: u64,
	is_stable_receiver: bool,
	note: String,
	drift: Option<Drift>,
	/// Where the wallet last connected from (newest recorded location).
	location: Option<sc_rest_client::sc_protos::stable::PeerLocation>,
	/// Live connection state from the loaded peer list; None when it is not loaded.
	connected: Option<bool>,
}

/// Rows to list, positions only unless `show_all`, and how many channels without a target are hidden.
fn visible_rows(rows: Vec<StableRow>, show_all: bool) -> (Vec<StableRow>, usize) {
	if show_all {
		return (rows, 0);
	}
	let total = rows.len();
	let shown: Vec<StableRow> = rows.into_iter().filter(|r| r.expected_usd > 0.0).collect();
	let hidden = total - shown.len();
	(shown, hidden)
}

#[component]
pub fn StableChannels() -> Element {
	let ctx = use_context::<AppCtx>();
	// The Location column needs connection state; load peers on entry, refresh keeps them current.
	use_hook(move || {
		if ctx.is_connected_peek() && ctx.data.peek().peers.is_none() {
			actions::fetch_peers(ctx);
		}
	});
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let loading = ctx.busy(Op::ListStableChannels);
	let price = ctx.price.read().as_ref().map(|p| p.price);
	let peers = ctx.data.read().peers.as_ref().map(|r| r.peers.clone());
	let rows: Option<Vec<StableRow>> = ctx.data.read().stable_channels.as_ref().map(|resp| {
		resp.channels
			.iter()
			.map(|ch| StableRow {
				channel_id: ch.channel_id.clone(),
				user_channel_id: ch.user_channel_id.clone(),
				counterparty: ch.counterparty.clone(),
				expected_usd: ch.expected_usd,
				backing_sats: ch.expected_msats / 1000,
				is_stable_receiver: ch.is_stable_receiver,
				note: ch.note.clone(),
				drift: stable_drift(ch, price),
				location: ch.recent_locations.first().cloned(),
				connected: peers.as_ref().and_then(|list| list.iter().find(|p| p.node_id == ch.counterparty).map(|p| p.is_connected)),
			})
			.collect()
	});
	let total_usd: f64 = rows.as_ref().map(|r| r.iter().map(|r| r.expected_usd).sum()).unwrap_or(0.0);
	let users = rows.as_ref().map(|r| r.iter().filter(|r| r.expected_usd > 0.0).count()).unwrap_or(0);
	let unpositioned = rows.as_ref().map(|r| r.len() - users).unwrap_or(0);
	let show_all = ctx.view.read().stable_show_unpositioned;
	let rows = rows.map(|r| visible_rows(r, show_all).0);
	let toggle_unpositioned = move |_| {
		let mut view = ctx.view;
		let show = !view.peek().stable_show_unpositioned;
		view.write().stable_show_unpositioned = show;
	};
	let refresh = move |_| {
		actions::fetch_price(ctx);
		actions::fetch_stable_channels(ctx);
		actions::fetch_peers(ctx);
	};
	rsx! {
		div { class: "grid-3",
			div { class: "card stat",
				div { class: "stat-title", span { class: "coin sm", "₿" } "BTC/USD" }
				div { class: "stat-value",
					match price {
						Some(p) if p > 0.0 => rsx! { "{format_usd(p)}" },
						Some(_) => rsx! { span { class: "muted", "fetching..." } },
						None => rsx! { span { class: "muted", "--" } },
					}
				}
				div { class: "stat-sub", "Live daemon price feed" }
			}
			div { class: "card stat",
				div { class: "stat-title", "Stabilized value" }
				div { class: "stat-value", style: "color: var(--green-text);", "{format_usd(total_usd)}" }
				div { class: "stat-sub", "Sum of USD targets" }
			}
			div { class: "card stat",
				div { class: "stat-title", "Stable users" }
				div { class: "stat-value", "{users}" }
				div { class: "stat-sub", "Channels with a USD target" }
			}
		}
		Card { class: "flush",
			div { class: "toolbar",
				span { class: "card-title", "Stable channels" }
				div { class: "row", style: "margin-left: auto;", RefreshBtn { busy: loading, onclick: refresh, op: Op::ListStableChannels } }
			}
			match rows {
				Some(rows) if !rows.is_empty() => rsx! {
					div { class: "table-wrap",
						table { class: "table clickable", style: "min-width: 1000px;",
							thead {
								tr {
									Th { label: "Peer", help: HELP_COUNTERPARTY }
									Th { label: "Location", help: HELP_LOCATION }
									Th { label: "Target", help: HELP_STABLE_USD, class: "right" }
									Th { label: "Backing", help: HELP_BACKING, class: "right" }
									Th { label: "Value now", help: HELP_DRIFT, class: "right" }
									Th { label: "Role", help: HELP_ROLE }
									Th { label: "Note", help: HELP_NOTE }
									th { class: "right", "Actions" }
								}
							}
							tbody {
								for row in rows {
									StableRowView { key: "{row.user_channel_id}", row }
								}
							}
						}
					}
				},
				Some(_) => rsx! { Empty { icon: "shield", title: "No stable users yet.", hint: "A channel becomes a stable user once it has a USD target." } },
				None => rsx! {
					if loading {
						Empty { icon: "shield", title: "Loading...", Spinner { large: true } }
					} else {
						Empty { icon: "shield", title: "Not loaded yet.", hint: "Click Refresh." }
					}
				},
			}
			if unpositioned > 0 {
				div { class: "toolbar muted small",
					span {
						if unpositioned == 1 { "1 channel has no stable position" } else { "{unpositioned} channels have no stable position" }
						" (routing peers or bitcoin-only wallets)"
					}
					button { class: "btn sm ghost", onclick: toggle_unpositioned, if show_all { "hide" } else { "show" } }
				}
			}
		}
		EditCard {}
	}
}

#[component]
fn StableRowView(row: StableRow) -> Element {
	let ctx = use_context::<AppCtx>();
	let (role_text, role_tone) = if row.is_stable_receiver { ("Receiver", "orange") } else { ("Provider", "info") };
	let ledger_id = row.user_channel_id.clone();
	let drawer_id = row.user_channel_id.clone();
	let edit_row = row.clone();
	rsx! {
		tr { onclick: move |_| open_drawer(ctx, Drawer::Channel(drawer_id.clone())),
			td { Peer { node_id: row.counterparty.clone() } }
			td {
				match row.location.clone() {
					Some(loc) => rsx! {
						span { class: "amount",
							span { "{crate::format::location_label(&loc)}" }
							span { class: "amount-sub mono", "{loc.ip}" }
							if !crate::format::location_is_live(row.connected, loc.last_seen_at, chrono::Utc::now().timestamp()) {
								span { class: "amount-sub", "seen {crate::ledger::relative_timestamp(loc.last_seen_at * 1000)}" }
							}
						}
					},
					None => rsx! { span { class: "faint", "—" } },
				}
			}
			// Expected USD — always dollars (USD target)
			td { class: "right num", style: "color: var(--green-text); font-weight: 700;", "{format_usd(row.expected_usd)}" }
			td { class: "right", Amount { msat: row.backing_sats * 1000 } }
			td { class: "right",
				match row.drift {
					Some(drift) => {
						let (text, tone) = drift_pill(&drift);
						rsx! {
							span { class: "amount",
								span { class: "num", "{format_usd(drift.value_usd)}" }
								span { class: "pill {tone}", style: "height: 18px; padding: 0 6px; margin-top: 2px;", "{text}" }
							}
						}
					},
					None => rsx! { span { class: "faint", "—" } },
				}
			}
			td { Pill { tone: role_tone, "{role_text}" } }
			td { style: "max-width: 160px; overflow: hidden; text-overflow: ellipsis;", title: "{row.note}",
				if row.note.is_empty() { span { class: "faint", "---" } } else { "{row.note}" }
			}
			// Open the exact stable identity or prefill the edit form.
			td { class: "right",
				// Buttons act on their own; they must not also open the row's side panel.
				div { class: "row nowrap end", style: "gap: 6px;", onclick: move |e| e.stop_propagation(),
					button { class: "btn sm", title: "Open this stable user_channel_id", onclick: move |_| actions::open_channel_ledger(ctx, ledger_id.clone()),
						Icon { name: "list", size: 14 } "Ledger"
					}
					button { class: "btn sm", title: "Edit this channel's stable target", onclick: move |_| {
						let mut forms = ctx.forms;
						let mut f = forms.write();
						f.edit_stable_channel.channel_id = edit_row.channel_id.clone();
						f.edit_stable_channel.expected_usd = format!("{:.2}", edit_row.expected_usd);
						f.edit_stable_channel.note = edit_row.note.clone();
					}, Icon { name: "edit", size: 14 } "Edit" }
				}
			}
		}
	}
}

/// Edit a channel's stable target. "Edit" on a row prefills; submitting sets
/// expected_usd (and note) via EditStableChannel on the daemon.
#[component]
fn EditCard() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().edit_stable_channel.clone();
	let loading = ctx.busy(Op::EditStableChannel);
	rsx! {
		div { class: "narrow",
			Card { title: "Edit Stable Channel", sub: "Use Edit on a row to prefill",
				div { class: "stack", style: "gap: 14px;",
					Field { label: "Channel ID", help: HELP_CHANNEL_ID,
						TextInput { value: form.channel_id.clone(), mono: true, oninput: move |v| forms.write().edit_stable_channel.channel_id = v }
					}
					div { class: "form-grid",
						Field { label: "Target USD", help: HELP_STABLE_USD, hint: "0 = stop stabilizing",
							TextInput { value: form.expected_usd.clone(), placeholder: "0.00", oninput: move |v| forms.write().edit_stable_channel.expected_usd = v }
						}
						Field { label: "Note", help: HELP_NOTE,
							TextInput { value: form.note.clone(), oninput: move |v| forms.write().edit_stable_channel.note = v }
						}
					}
					button { class: "btn primary", style: "align-self: flex-start;", disabled: loading, onclick: move |_| actions::edit_stable_channel(ctx),
						if loading { Spinner {} "Submitting..." } else { "Submit" }
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn row(uid: &str, expected_usd: f64) -> StableRow {
		StableRow {
			channel_id: format!("c{uid}"),
			user_channel_id: uid.into(),
			counterparty: "02aa".into(),
			expected_usd,
			backing_sats: 0,
			is_stable_receiver: true,
			note: String::new(),
			drift: None,
			location: None,
			connected: None,
		}
	}

	#[test]
	fn channels_without_a_target_are_hidden_until_asked_for() {
		let rows = vec![row("8", 0.0), row("9", 43.63)];
		let (shown, hidden) = visible_rows(rows.clone(), false);
		assert_eq!((shown.len(), hidden), (1, 1));
		assert_eq!(shown[0].user_channel_id, "9");
		let (shown, hidden) = visible_rows(rows, true);
		assert_eq!((shown.len(), hidden), (2, 0));
	}
}
