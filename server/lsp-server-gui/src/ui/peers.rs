use dioxus::prelude::*;

use crate::actions;
use crate::state::{AppCtx, Dialog, Op};
use crate::ui::widgets::{Bubble, Card, Check, Empty, Field, Gate, Icon, Modal, Peer, Pill, RefreshBtn, Spinner, TextInput, Th};
use crate::ui::{close_dialog, open_dialog};

const HELP_PEER_NODE_ID: &str = "The node public key identifying the connected or target peer.";
const HELP_PEER_ADDRESS: &str = "Network address used to reach the peer.";
const HELP_PEER_STATUS: &str = "Whether the peer is currently connected or disconnected.";
const HELP_PERSIST: &str = "Reconnect to this peer automatically after restarts.";

#[derive(Clone, PartialEq)]
struct PeerRow {
	node_id: String,
	address: String,
	is_connected: bool,
}

#[component]
pub fn Peers() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	let loading = ctx.busy(Op::Peers);
	let disconnecting = ctx.busy(Op::DisconnectPeer);
	let rows: Option<Vec<PeerRow>> = ctx.data.read().peers.as_ref().map(|resp| {
		resp.peers
			.iter()
			.map(|p| PeerRow { node_id: p.node_id.clone(), address: p.address.clone(), is_connected: p.is_connected })
			.collect()
	});
	let connected_count = rows.as_ref().map(|r| r.iter().filter(|p| p.is_connected).count()).unwrap_or(0);
	rsx! {
		Card { class: "flush",
			div { class: "toolbar",
				span { class: "count", "{connected_count} peers connected" }
				div { class: "row", style: "margin-left: auto;",
					RefreshBtn { busy: loading, onclick: move |_| actions::fetch_peers(ctx), op: Op::Peers }
					button { class: "btn sm accent", onclick: move |_| open_dialog(ctx, Dialog::ConnectPeer), Icon { name: "plus", size: 14 } "Connect Peer" }
				}
			}
			match rows {
				Some(peers) if !peers.is_empty() => rsx! {
					div { class: "table-wrap",
						table { class: "table", style: "min-width: 720px;",
							thead {
								tr {
									Th { label: "Node ID", help: HELP_PEER_NODE_ID }
									Th { label: "Address", help: HELP_PEER_ADDRESS }
									Th { label: "Status", help: HELP_PEER_STATUS }
									th { class: "right", "Actions" }
								}
							}
							tbody {
								for peer in peers {
									tr { key: "{peer.node_id}",
										td { Peer { node_id: peer.node_id.clone(), keep: 8 } }
										td { span { class: "mono", "{peer.address}" } }
										td {
											if peer.is_connected {
												Pill { tone: "success", "Connected" }
											} else {
												Pill { tone: "muted", "Disconnected" }
											}
										}
										td { class: "right",
											button {
												class: "btn sm danger",
												disabled: disconnecting,
												onclick: {
													let node_id = peer.node_id.clone();
													move |_| actions::disconnect_peer(ctx, node_id.clone())
												},
												"Disconnect"
											}
										}
									}
								}
							}
						}
					}
				},
				_ => rsx! {
					if loading {
						Empty { icon: "users", title: "Loading peers...", Spinner { large: true } }
					} else {
						Empty { icon: "users", title: "No peers connected", hint: "Click Refresh to load" }
					}
				},
			}
		}
	}
}

#[component]
pub fn ConnectPeerDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().connect_peer.clone();
	let pending = ctx.busy(Op::ConnectPeer);
	let mut cancel = move || {
		forms.write().connect_peer = Default::default();
		close_dialog(ctx);
	};
	rsx! {
		Modal {
			title: "Connect Peer",
			sub: "Connect to a Lightning Network peer",
			icon: rsx! { Bubble { icon: "users", tone: "blue" } },
			onclose: move |_| cancel(),
			footer: rsx! {
				button { class: "btn ghost", onclick: move |_| cancel(), "Cancel" }
				button { class: "btn primary", disabled: pending, onclick: move |_| actions::connect_peer(ctx),
					if pending { Spinner {} }
					"Connect"
				}
			},
			Field { label: "Node Pubkey", help: HELP_PEER_NODE_ID, TextInput { value: form.node_pubkey.clone(), mono: true, oninput: move |v| forms.write().connect_peer.node_pubkey = v } }
			Field { label: "Address", help: HELP_PEER_ADDRESS, TextInput { value: form.address.clone(), mono: true, placeholder: "host:port", oninput: move |v| forms.write().connect_peer.address = v } }
			div { class: "toggle-row",
				Check { checked: form.persist, label: "Persist Connection", help: HELP_PERSIST, onchange: move |v| forms.write().connect_peer.persist = v }
			}
		}
	}
}
