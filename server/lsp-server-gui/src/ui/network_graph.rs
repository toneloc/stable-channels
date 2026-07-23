use eframe::egui;
use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::app::LspServerApp;
use crate::ui::layout::{self, card, kv_grid_custom_info, page_scrolled};
use crate::ui::widgets;

const HELP_SHORT_CHANNEL_ID: &str =
    "Compact channel locator based on block height, transaction index, and output index.";
const HELP_GRAPH_CHANNEL: &str =
    "A public channel known through network gossip or Rapid Gossip Sync.";
const HELP_GRAPH_NODE: &str = "A public Lightning node known through network gossip.";
const HELP_NODE_ID: &str =
    "The public key that identifies this Lightning node to peers and the network.";
const HELP_NODE_ONE: &str = "One endpoint of the public channel.";
const HELP_NODE_TWO: &str = "The other endpoint of the public channel.";
const HELP_CAPACITY: &str = "The total channel size currently tracked for this channel.";
const HELP_CLTV_DELTA: &str =
    "The additional block delay required by this channel's routing policy for forwarded HTLCs.";
const HELP_HTLC_MIN: &str = "Minimum HTLC amount allowed by this channel direction.";
const HELP_HTLC_MAX: &str = "Maximum HTLC amount allowed by this channel direction.";
const HELP_CHANNELS: &str = "Number of public channels associated with this graph node.";
const HELP_ADDRESSES: &str = "Network addresses advertised for this graph node.";

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
    ui.heading("Network Graph");
    ui.add_space(10.0);

    if app.render_disconnected_gate(ui) {
        return;
    }

    page_scrolled(ui, |ui| {
        let n = layout::columns_for_width(ui.available_width()).min(2);
        ui.columns(n, |cols| {
            cols[0 % n].push_id("ng_channels", |ui| {
                card(ui, "Graph Channels", |ui| render_channels_section(ui, app));
            });
            cols[1 % n].push_id("ng_nodes", |ui| {
                card(ui, "Graph Nodes", |ui| render_nodes_section(ui, app));
            });
        });
    });
}

fn render_channels_section(ui: &mut egui::Ui, app: &mut LspServerApp) {
    ui.horizontal(|ui| {
        let is_pending = app.state.tasks.graph_list_channels.is_some();
        if is_pending {
            ui.spinner();
            ui.label("Loading...");
        } else if ui.button("List Channels").clicked() {
            app.fetch_graph_channels();
        }
    });

    if let Some(resp) = &app.state.graph_channels {
        ui.add_space(5.0);
        ui.label(format!(
            "{} channels in network graph",
            resp.short_channel_ids.len()
        ))
        .on_hover_text(HELP_GRAPH_CHANNEL);

        if !resp.short_channel_ids.is_empty() {
            ui.add_space(5.0);

            // Filter box for scid list (temp memory, no AppState field)
            let filter_id = ui.id().with("scid_filter");
            let mut filter =
                ui.memory_mut(|m| m.data.get_temp::<String>(filter_id).unwrap_or_default());
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.text_edit_singleline(&mut filter);
            });
            ui.memory_mut(|m| m.data.insert_temp(filter_id, filter.clone()));

            let max_display = 100.min(resp.short_channel_ids.len());
            // Apply filter to the already-capped slice
            let visible: Vec<&u64> = resp
                .short_channel_ids
                .iter()
                .take(max_display)
                .filter(|scid| filter.is_empty() || scid.to_string().contains(&filter))
                .collect();

            TableBuilder::new(ui)
                .striped(true)
                .resizable(false)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .auto_shrink([false, true])
                .column(Column::remainder().at_least(64.0).clip(true))
                .header(22.0, |mut header| {
                    header.col(|ui| {
                        widgets::table_header_with_info(
                            ui,
                            "Short Channel ID",
                            HELP_SHORT_CHANNEL_ID,
                        );
                    });
                })
                .body(|mut body| {
                    for scid in &visible {
                        let scid_str = scid.to_string();
                        body.row(24.0, |mut row| {
                            row.col(|ui| {
                                widgets::id_with_copy(ui, &scid_str, &mut app.state.status_message);
                            });
                        });
                    }
                });

            if resp.short_channel_ids.len() > max_display {
                ui.label(format!(
                    "... and {} more",
                    resp.short_channel_ids.len() - max_display
                ));
            }
        }
    }

    ui.add_space(10.0);
    ui.separator();

    card(ui, "Lookup Channel", |ui| render_channel_lookup(ui, app));
}

fn render_channel_lookup(ui: &mut egui::Ui, app: &mut LspServerApp) {
    let form = &mut app.state.forms.graph_get_channel;
    ui.horizontal(|ui| {
        widgets::label_with_info(ui, "Short Channel ID:", HELP_SHORT_CHANNEL_ID);
        ui.text_edit_singleline(&mut form.short_channel_id);
    });

    ui.horizontal(|ui| {
        let is_pending = app.state.tasks.graph_get_channel.is_some();
        if is_pending {
            ui.spinner();
            ui.label("Loading...");
        } else if ui.button("Lookup").clicked() {
            app.fetch_graph_channel();
        }
    });

    if let Some(resp) = &app.state.graph_channel_detail {
        if let Some(ch) = &resp.channel {
            ui.add_space(5.0);
            // Pre-extract values before mixed borrows
            let node_one = ch.node_one.clone();
            let node_two = ch.node_two.clone();
            let capacity = ch.capacity_sats;
            let one_to_two = ch.one_to_two.clone();
            let two_to_one = ch.two_to_one.clone();

            let cap_str = capacity.map(|c| app.fmt_sats(c));

            // Node One/Two each need their own copy button, so they can't share one kv_grid_custom row vec (two closures can't both hold &mut app.state.status_message at once); rendered as their own rows instead.
            ui.horizontal(|ui| {
                ui.label(RichText::new("Node One:").color(layout::SECONDARY));
                widgets::info_icon(ui, HELP_NODE_ONE);
                widgets::id_with_copy(ui, &node_one, &mut app.state.status_message);
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Node Two:").color(layout::SECONDARY));
                widgets::info_icon(ui, HELP_NODE_TWO);
                widgets::id_with_copy(ui, &node_two, &mut app.state.status_message);
            });
            ui.add_space(4.0);

            let mut rows: crate::ui::layout::KvInfoRows = Vec::new();

            if let Some(cap_fmt) = cap_str {
                rows.push((
                    "Capacity",
                    Some(HELP_CAPACITY),
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(format!("{} sats", cap_fmt));
                    }),
                ));
            }

            if let Some(update) = &one_to_two {
                let enabled = update.enabled;
                let cltv = update.cltv_expiry_delta;
                let min = update.htlc_minimum_msat;
                let max = update.htlc_maximum_msat;
                rows.push((
                    "1->2 Enabled",
                    None,
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(if enabled { "Yes" } else { "No" });
                    }),
                ));
                rows.push((
                    "1->2 CLTV Delta",
                    Some(HELP_CLTV_DELTA),
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(format!("{}", cltv));
                    }),
                ));
                rows.push((
                    "1->2 HTLC Min",
                    Some(HELP_HTLC_MIN),
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(format!("{} msat", min));
                    }),
                ));
                rows.push((
                    "1->2 HTLC Max",
                    Some(HELP_HTLC_MAX),
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(format!("{} msat", max));
                    }),
                ));
            }

            if let Some(update) = &two_to_one {
                let enabled = update.enabled;
                let cltv = update.cltv_expiry_delta;
                let min = update.htlc_minimum_msat;
                let max = update.htlc_maximum_msat;
                rows.push((
                    "2->1 Enabled",
                    None,
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(if enabled { "Yes" } else { "No" });
                    }),
                ));
                rows.push((
                    "2->1 CLTV Delta",
                    Some(HELP_CLTV_DELTA),
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(format!("{}", cltv));
                    }),
                ));
                rows.push((
                    "2->1 HTLC Min",
                    Some(HELP_HTLC_MIN),
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(format!("{} msat", min));
                    }),
                ));
                rows.push((
                    "2->1 HTLC Max",
                    Some(HELP_HTLC_MAX),
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(format!("{} msat", max));
                    }),
                ));
            }

            if !rows.is_empty() {
                kv_grid_custom_info(ui, "graph_channel_detail", rows);
            }
        }
    }
}

fn render_nodes_section(ui: &mut egui::Ui, app: &mut LspServerApp) {
    ui.horizontal(|ui| {
        let is_pending = app.state.tasks.graph_list_nodes.is_some();
        if is_pending {
            ui.spinner();
            ui.label("Loading...");
        } else if ui.button("List Nodes").clicked() {
            app.fetch_graph_nodes();
        }
    });

    if let Some(resp) = &app.state.graph_nodes {
        ui.add_space(5.0);
        ui.label(format!("{} nodes in network graph", resp.node_ids.len()))
            .on_hover_text(HELP_GRAPH_NODE);

        if !resp.node_ids.is_empty() {
            ui.add_space(5.0);

            // Filter box for node list (temp memory, no AppState field)
            let filter_id = ui.id().with("node_filter");
            let mut filter =
                ui.memory_mut(|m| m.data.get_temp::<String>(filter_id).unwrap_or_default());
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.text_edit_singleline(&mut filter);
            });
            ui.memory_mut(|m| m.data.insert_temp(filter_id, filter.clone()));

            let max_display = 100.min(resp.node_ids.len());
            // Apply filter to the already-capped slice
            let visible: Vec<&String> = resp
                .node_ids
                .iter()
                .take(max_display)
                .filter(|id| filter.is_empty() || id.contains(&filter))
                .collect();

            TableBuilder::new(ui)
                .striped(true)
                .resizable(false)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .auto_shrink([false, true])
                .column(Column::remainder().at_least(64.0).clip(true))
                .header(22.0, |mut header| {
                    header.col(|ui| {
                        widgets::table_header_with_info(ui, "Node ID", HELP_NODE_ID);
                    });
                })
                .body(|mut body| {
                    for node_id in &visible {
                        let node_id_str = node_id.to_string();
                        body.row(24.0, |mut row| {
                            row.col(|ui| {
                                widgets::id_with_copy(
                                    ui,
                                    &node_id_str,
                                    &mut app.state.status_message,
                                );
                            });
                        });
                    }
                });

            if resp.node_ids.len() > max_display {
                ui.label(format!(
                    "... and {} more",
                    resp.node_ids.len() - max_display
                ));
            }
        }
    }

    ui.add_space(10.0);
    ui.separator();

    card(ui, "Lookup Node", |ui| render_node_lookup(ui, app));
}

fn render_node_lookup(ui: &mut egui::Ui, app: &mut LspServerApp) {
    let form = &mut app.state.forms.graph_get_node;
    ui.horizontal(|ui| {
        widgets::label_with_info(ui, "Node ID:", HELP_NODE_ID);
        ui.text_edit_singleline(&mut form.node_id);
    });

    ui.horizontal(|ui| {
        let is_pending = app.state.tasks.graph_get_node.is_some();
        if is_pending {
            ui.spinner();
            ui.label("Loading...");
        } else if ui.button("Lookup").clicked() {
            app.fetch_graph_node();
        }
    });

    if let Some(resp) = &app.state.graph_node_detail {
        if let Some(node) = &resp.node {
            ui.add_space(5.0);
            let channel_count = node.channels.len();

            let mut rows: crate::ui::layout::KvInfoRows = vec![(
                "Channels",
                Some(HELP_CHANNELS),
                Box::new(move |ui: &mut egui::Ui| {
                    ui.label(format!("{}", channel_count));
                }),
            )];

            if let Some(ann) = &node.announcement_info {
                let alias = ann.alias.clone();
                rows.push((
                    "Alias",
                    None,
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(alias);
                    }),
                ));

                let color = format!("#{}", ann.rgb);
                rows.push((
                    "Color",
                    None,
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(color);
                    }),
                ));

                let ts = ann.last_update;
                rows.push((
                    "Last Update",
                    None,
                    Box::new(move |ui: &mut egui::Ui| {
                        ui.label(format!("{}", ts))
                            .on_hover_text(format!("unix: {}", ts));
                    }),
                ));

                if !ann.addresses.is_empty() {
                    let addresses = ann.addresses.clone();
                    rows.push((
                        "Addresses",
                        Some(HELP_ADDRESSES),
                        Box::new(move |ui: &mut egui::Ui| {
                            ui.vertical(|ui| {
                                for addr in &addresses {
                                    ui.label(addr);
                                }
                            });
                        }),
                    ));
                }
            }

            kv_grid_custom_info(ui, "graph_node_detail", rows);
        }
    }
}
