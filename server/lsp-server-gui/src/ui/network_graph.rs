use dioxus::prelude::*;

use crate::actions;
use crate::state::{AppCtx, Op};
use crate::ui::widgets::{Card, Empty, Field, Gate, Hover, Icon, IdCopy, InfoTip, Kv, Pill, Spinner, TextInput, Th};

const HELP_SHORT_CHANNEL_ID: &str =
	"Compact channel locator based on block height, transaction index, and output index.";
const HELP_GRAPH_CHANNEL: &str = "A public channel known through network gossip or Rapid Gossip Sync.";
const HELP_GRAPH_NODE: &str = "A public Lightning node known through network gossip.";
const HELP_NODE_ID: &str = "The public key that identifies this Lightning node to peers and the network.";
const HELP_NODE_ONE: &str = "One endpoint of the public channel.";
const HELP_NODE_TWO: &str = "The other endpoint of the public channel.";
const HELP_CAPACITY: &str = "The total channel size currently tracked for this channel.";
const HELP_CLTV_DELTA: &str =
	"The additional block delay required by this channel's routing policy for forwarded HTLCs.";
const HELP_HTLC_MIN: &str = "Minimum HTLC amount allowed by this channel direction.";
const HELP_HTLC_MAX: &str = "Maximum HTLC amount allowed by this channel direction.";
const HELP_CHANNELS: &str = "Number of public channels associated with this graph node.";
const HELP_ADDRESSES: &str = "Network addresses advertised for this graph node.";

/// Rows shown per graph list.
const MAX_DISPLAY: usize = 100;

#[component]
pub fn NetworkGraph() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	rsx! {
		div { class: "grid-2",
			div { class: "stack", style: "gap: 18px;",
				ChannelsSection {}
				ChannelLookup {}
			}
			div { class: "stack", style: "gap: 18px;",
				NodesSection {}
				NodeLookup {}
			}
		}
	}
}

/// Filter first, then cap, so matches beyond the first rows are still found.
fn visible<'a>(items: impl Iterator<Item = String> + 'a, filter: &str) -> (Vec<String>, usize) {
	let matching: Vec<String> = items.filter(|item| filter.is_empty() || item.contains(filter)).collect();
	let total = matching.len();
	(matching.into_iter().take(MAX_DISPLAY).collect(), total)
}

#[component]
fn ChannelsSection() -> Element {
	let ctx = use_context::<AppCtx>();
	let loading = ctx.busy(Op::GraphListChannels);
	let mut view = ctx.view;
	let filter = view.read().scid_filter.clone();
	let listing = ctx.data.read().graph_channels.as_ref().map(|resp| {
		let count = resp.short_channel_ids.len();
		let (rows, matching) = visible(resp.short_channel_ids.iter().map(|scid| scid.to_string()), &filter);
		(count, rows, matching)
	});
	rsx! {
		Card { class: "flush", title: "Graph Channels", sub: "Public channels from gossip",
			actions: rsx! {
				button { class: "btn sm", disabled: loading, onclick: move |_| actions::fetch_graph_channels(ctx),
					if loading { Spinner {} "Loading..." } else { Icon { name: "list", size: 14 } "List Channels" }
				}
			},
			GraphList { listing, filter, noun: "channels", help: HELP_GRAPH_CHANNEL, column: "Short Channel ID", column_help: HELP_SHORT_CHANNEL_ID, oninput: move |v| view.write().scid_filter = v }
		}
	}
}

#[component]
fn NodesSection() -> Element {
	let ctx = use_context::<AppCtx>();
	let loading = ctx.busy(Op::GraphListNodes);
	let mut view = ctx.view;
	let filter = view.read().node_filter.clone();
	let listing = ctx.data.read().graph_nodes.as_ref().map(|resp| {
		let count = resp.node_ids.len();
		let (rows, matching) = visible(resp.node_ids.iter().cloned(), &filter);
		(count, rows, matching)
	});
	rsx! {
		Card { class: "flush", title: "Graph Nodes", sub: "Public nodes from gossip",
			actions: rsx! {
				button { class: "btn sm", disabled: loading, onclick: move |_| actions::fetch_graph_nodes(ctx),
					if loading { Spinner {} "Loading..." } else { Icon { name: "list", size: 14 } "List Nodes" }
				}
			},
			GraphList { listing, filter, noun: "nodes", help: HELP_GRAPH_NODE, column: "Node ID", column_help: HELP_NODE_ID, oninput: move |v| view.write().node_filter = v }
		}
	}
}

/// Filterable, capped list of graph identifiers.
#[component]
fn GraphList(
	listing: Option<(usize, Vec<String>, usize)>, filter: String, noun: &'static str, help: &'static str,
	column: &'static str, column_help: &'static str, oninput: EventHandler<String>,
) -> Element {
	let Some((count, rows, matching)) = listing else {
		return rsx! { Empty { icon: "graph", title: "Not loaded", hint: "List the network graph to browse it." } };
	};
	rsx! {
		div { class: "toolbar",
			span { class: "count", "{count} {noun} in network graph" }
			InfoTip { text: help }
			if count > 0 {
				div { class: "search", style: "margin-left: auto; max-width: 240px;",
					Icon { name: "search", size: 15 }
					TextInput { value: filter, small: true, placeholder: "Filter", oninput: move |v| oninput.call(v) }
				}
			}
		}
		if !rows.is_empty() {
			div { class: "table-wrap", style: "max-height: 420px; overflow-y: auto;",
				table { class: "table",
					thead { tr { Th { label: column, help: column_help } } }
					tbody {
						for id in rows.iter() {
							tr { key: "{id}", td { IdCopy { value: id.clone(), head: 12, tail: 12 } } }
						}
					}
				}
			}
		}
		if matching > rows.len() {
			div { class: "table-foot", "... and {matching - rows.len()} more" }
		}
	}
}

#[component]
fn ChannelLookup() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let scid = forms.read().graph_get_channel.short_channel_id.clone();
	let pending = ctx.busy(Op::GraphGetChannel);
	let channel = ctx.data.read().graph_channel_detail.as_ref().and_then(|r| r.channel.clone());
	rsx! {
		Card { title: "Lookup Channel",
			div { class: "stack", style: "gap: 14px;",
				div { class: "row nowrap", style: "align-items: flex-end;",
					div { class: "grow",
						Field { label: "Short Channel ID", help: HELP_SHORT_CHANNEL_ID,
							TextInput { value: scid, mono: true, oninput: move |v| forms.write().graph_get_channel.short_channel_id = v }
						}
					}
					button { class: "btn", disabled: pending, onclick: move |_| actions::fetch_graph_channel(ctx),
						if pending { Spinner {} } else { Icon { name: "search", size: 16 } }
						"Lookup"
					}
				}
				if let Some(ch) = channel {
					div { class: "kv",
						Kv { label: "Node One", help: HELP_NODE_ONE, IdCopy { value: ch.node_one.clone() } }
						Kv { label: "Node Two", help: HELP_NODE_TWO, IdCopy { value: ch.node_two.clone() } }
						if let Some(capacity) = ch.capacity_sats {
							Kv { label: "Capacity", help: HELP_CAPACITY, Hover { tip: format!("{} sats", crate::format::format_sats(capacity)), span { class: "num", "{ctx.fmt_sats(capacity)}" } } }
						}
					}
					div { class: "grid-2", style: "gap: 12px;",
						for (label, update) in [("1 → 2", ch.one_to_two.clone()), ("2 → 1", ch.two_to_one.clone())] {
							if let Some(update) = update {
								div { key: "{label}", class: "card inner stack tight",
									div { class: "row between",
										span { class: "strong", "{label}" }
										if update.enabled { Pill { tone: "success", "Enabled" } } else { Pill { tone: "muted", "Disabled" } }
									}
									div { class: "kv",
										crate::ui::widgets::Kv { label: "CLTV Delta", help: HELP_CLTV_DELTA, span { class: "num", "{update.cltv_expiry_delta}" } }
										crate::ui::widgets::Kv { label: "HTLC Min", help: HELP_HTLC_MIN, span { class: "num", "{update.htlc_minimum_msat} msat" } }
										crate::ui::widgets::Kv { label: "HTLC Max", help: HELP_HTLC_MAX, span { class: "num", "{update.htlc_maximum_msat} msat" } }
									}
								}
							}
						}
					}
				}
			}
		}
	}
}

#[component]
fn NodeLookup() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let node_id = forms.read().graph_get_node.node_id.clone();
	let pending = ctx.busy(Op::GraphGetNode);
	let node = ctx.data.read().graph_node_detail.as_ref().and_then(|r| r.node.clone());
	rsx! {
		Card { title: "Lookup Node",
			div { class: "stack", style: "gap: 14px;",
				div { class: "row nowrap", style: "align-items: flex-end;",
					div { class: "grow",
						Field { label: "Node ID", help: HELP_NODE_ID,
							TextInput { value: node_id, mono: true, oninput: move |v| forms.write().graph_get_node.node_id = v }
						}
					}
					button { class: "btn", disabled: pending, onclick: move |_| actions::fetch_graph_node(ctx),
						if pending { Spinner {} } else { Icon { name: "search", size: 16 } }
						"Lookup"
					}
				}
				if let Some(node) = node {
					div { class: "kv",
						Kv { label: "Channels", help: HELP_CHANNELS, span { class: "num", "{node.channels.len()}" } }
						if let Some(ann) = node.announcement_info {
							Kv { label: "Alias", span { class: "strong", "{ann.alias}" } }
							Kv { label: "Color",
								span { style: "width: 14px; height: 14px; border-radius: 4px; background: #{ann.rgb}; box-shadow: inset 0 0 0 1px var(--border-strong);" }
								span { class: "mono", "#{ann.rgb}" }
							}
							Kv { label: "Last Update", Hover { tip: format!("unix: {}", ann.last_update), span { class: "num", "{ann.last_update}" } } }
							if !ann.addresses.is_empty() {
								Kv { label: "Addresses", help: HELP_ADDRESSES,
									div { class: "stack tight",
										for addr in ann.addresses.iter() {
											span { key: "{addr}", class: "mono", "{addr}" }
										}
									}
								}
							}
						}
					}
				}
			}
		}
	}
}
