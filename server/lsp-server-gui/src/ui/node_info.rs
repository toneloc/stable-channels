use dioxus::prelude::*;

use crate::actions;
use crate::config::ChainSourceConfig;
use crate::format::{relative_long, truncate_id};
use crate::state::{AppCtx, Op};
use crate::ui::widgets::{Card, CopyBtn, Empty, Gate, Hover, IdCopy, Kv, Pill, RefreshBtn};

const HELP_NODE_ID: &str = "The public key that identifies this Lightning node to peers and the network.";
const HELP_BEST_BLOCK: &str = "The best known block for this node's wallet, shown by block hash and height. If this height trails your chain source tip, the node may still be syncing.";
const HELP_NETWORK: &str =
	"The Bitcoin network this node is connected to, such as mainnet, testnet, signet, or regtest.";
const HELP_CHAIN_SOURCE: &str = "The blockchain backend used for headers, transactions, fee data, and wallet sync.";
const HELP_RPC_ADDRESS: &str =
	"The Bitcoin Core RPC endpoint used by the node for chain data and wallet-related checks.";
const HELP_ELECTRUM_URL: &str = "The Electrum server used by the node to scan and monitor the chain.";
const HELP_ESPLORA_URL: &str = "The Esplora API endpoint used by the node to query chain data.";
const HELP_LIGHTNING_WALLET_SYNC: &str = "The last time Lightning wallet state was synced against the chain source.";
const HELP_ONCHAIN_WALLET_SYNC: &str = "The last time the on-chain wallet scanned or synced against the chain source.";
const HELP_FEE_RATE_CACHE_UPDATE: &str =
	"The last time the node refreshed fee-rate estimates used when building on-chain transactions.";
const HELP_RGS_SNAPSHOT: &str =
	"The last Rapid Gossip Sync snapshot applied to update the Lightning network graph for route finding.";
const HELP_NODE_ANNOUNCEMENT: &str =
	"The last time this node broadcast its public node announcement to the Lightning network.";

#[component]
pub fn NodeInfo() -> Element {
	let ctx = use_context::<AppCtx>();
	let connected = ctx.is_connected();
	rsx! {
		div { class: "grid-2",
			if connected {
				NodeDetails {}
			}
			// Chain source is local config — visible even while disconnected.
			ChainSource {}
		}
		if !connected {
			Gate {}
		}
	}
}

#[component]
fn NodeDetails() -> Element {
	let ctx = use_context::<AppCtx>();
	let now = *ctx.now.read();
	let info = ctx.data.read().node_info.clone();
	let loading = ctx.busy(Op::NodeInfo);
	let sync_rows = info
		.as_ref()
		.map(|info| {
			[
				("Lightning Wallet Sync", HELP_LIGHTNING_WALLET_SYNC, info.latest_lightning_wallet_sync_timestamp),
				("On-chain Wallet Sync", HELP_ONCHAIN_WALLET_SYNC, info.latest_onchain_wallet_sync_timestamp),
				("Fee Rate Cache Update", HELP_FEE_RATE_CACHE_UPDATE, info.latest_fee_rate_cache_update_timestamp),
				("RGS Snapshot", HELP_RGS_SNAPSHOT, info.latest_rgs_snapshot_timestamp),
				("Node Announcement", HELP_NODE_ANNOUNCEMENT, info.latest_node_announcement_broadcast_timestamp),
			]
		})
		.unwrap_or_default();
	rsx! {
		Card {
			title: "Node Details",
			actions: rsx! {
				Pill { tone: "success", span { class: "dot success", style: "width: 6px; height: 6px; box-shadow: none;" } "Online" }
				RefreshBtn { busy: loading, onclick: move |_| actions::fetch_node_info(ctx), op: Op::NodeInfo }
			},
			match info {
				None => rsx! {
					Empty { icon: "home", title: "No node info available", hint: "Click Refresh to fetch." }
				},
				Some(info) => rsx! {
					div { class: "kv",
						Kv { label: "Node ID", help: HELP_NODE_ID, IdCopy { value: info.node_id.clone(), head: 10, tail: 10 } }
						if let Some(block) = info.current_best_block.as_ref() {
							Kv { label: "Best Block", help: HELP_BEST_BLOCK,
								Hover { tip: block.block_hash.clone(), span { class: "mono", "{truncate_id(&block.block_hash, 8, 8)}" } }
								Pill { tone: "info", "height {block.height}" }
							}
						}
						for (label, help, ts) in sync_rows {
							if let Some(ts) = ts {
								Kv { key: "{label}", label, help,
									Hover { tip: format!("unix: {ts}"), span { "{relative_long(ts, now)}" } }
								}
							}
						}
					}
				},
			}
		}
	}
}

#[component]
fn ChainSource() -> Element {
	let ctx = use_context::<AppCtx>();
	let conn = ctx.conn.read();
	let network = conn.network.clone();
	let chain_source = conn.chain_source.clone();
	drop(conn);
	let empty = matches!(chain_source, ChainSourceConfig::None) && network.is_empty();
	rsx! {
		Card { title: "Chain Source", sub: "From the loaded daemon config",
			if empty {
				Empty { icon: "cube", title: "No chain source configured." }
			} else {
				div { class: "kv",
					if !network.is_empty() {
						Kv { label: "Network", help: HELP_NETWORK, span { class: "mono", "{network}" } }
					}
					match chain_source {
						ChainSourceConfig::None => rsx! {},
						ChainSourceConfig::Bitcoind { rpc_address, rpc_user, rpc_password } => rsx! {
							Kv { label: "Chain Source", help: HELP_CHAIN_SOURCE, Pill { tone: "orange", "Bitcoin Core RPC" } }
							Kv { label: "RPC Address", help: HELP_RPC_ADDRESS, span { class: "mono", "{rpc_address}" } CopyBtn { value: rpc_address.clone() } }
							Kv { label: "RPC User", span { class: "mono", "{rpc_user}" } CopyBtn { value: rpc_user.clone() } }
							Kv { label: "RPC Password", span { class: "mono", "********" } CopyBtn { value: rpc_password.clone() } }
						},
						ChainSourceConfig::Electrum { server_url } => rsx! {
							Kv { label: "Chain Source", help: HELP_CHAIN_SOURCE, Pill { tone: "info", "Electrum" } }
							Kv { label: "Server URL", help: HELP_ELECTRUM_URL, span { class: "mono break", "{server_url}" } CopyBtn { value: server_url.clone() } }
						},
						ChainSourceConfig::Esplora { server_url } => rsx! {
							Kv { label: "Chain Source", help: HELP_CHAIN_SOURCE, Pill { tone: "success", "Esplora" } }
							Kv { label: "Server URL", help: HELP_ESPLORA_URL, span { class: "mono break", "{server_url}" } CopyBtn { value: server_url.clone() } }
						},
					}
				}
			}
		}
	}
}
