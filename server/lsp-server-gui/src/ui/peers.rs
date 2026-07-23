use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::app::LspServerApp;
use crate::ui::layout::page_scrolled;
use crate::ui::widgets;

const HELP_PEER_NODE_ID: &str = "The node public key identifying the connected or target peer.";
const HELP_PEER_ADDRESS: &str = "Network address used to reach the peer.";
const HELP_PEER_STATUS: &str = "Whether the peer is currently connected or disconnected.";

// Per-row snapshot extracted from state.peers so the state borrow is released
// before widgets::id_with_copy (needs &mut app.state.status_message) runs in the table body.
struct PeerRow {
    node_id: String,
    address: String,
    is_connected: bool,
}

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
    ui.heading("Peers");
    ui.add_space(5.0);

    ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            app.fetch_peers();
        }
        ui.separator();
        if ui.button("Connect Peer").clicked() {
            app.state.show_connect_peer_dialog = true;
        }
    });

    ui.add_space(10.0);

    let mut disconnect_node_id = None;

    // Pre-extract peer rows so the &app.state.peers borrow ends before the table
    // body calls widgets::id_with_copy (needs &mut app.state.status_message).
    let rows: Option<Vec<PeerRow>> = app.state.peers.as_ref().map(|resp| {
        resp.peers
            .iter()
            .map(|p| PeerRow {
                node_id: p.node_id.clone(),
                address: p.address.clone(),
                is_connected: p.is_connected,
            })
            .collect()
    });

    match rows {
        Some(peers) => {
            if peers.is_empty() {
                widgets::empty_state(ui, "👥", "No peers connected", "Click Refresh to load");
            } else {
                // Summary line: count connected peers
                let connected_count = peers.iter().filter(|p| p.is_connected).count();
                ui.label(format!("{} peers connected", connected_count));
                ui.add_space(5.0);

                page_scrolled(ui, |ui| {
                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(false)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .auto_shrink([false, true])
                        .column(Column::remainder().at_least(64.0).clip(true)) // Node ID
                        .column(Column::remainder().at_least(64.0).clip(true)) // Address
                        .column(Column::auto()) // Status
                        .column(Column::auto()) // Actions
                        .header(22.0, |mut header| {
                            header.col(|ui| {
                                widgets::table_header_with_info(ui, "Node ID", HELP_PEER_NODE_ID);
                            });
                            header.col(|ui| {
                                widgets::table_header_with_info(ui, "Address", HELP_PEER_ADDRESS);
                            });
                            header.col(|ui| {
                                widgets::table_header_with_info(ui, "Status", HELP_PEER_STATUS);
                            });
                            header.col(|ui| {
                                ui.strong("Actions");
                            });
                        })
                        .body(|mut body| {
                            for peer in &peers {
                                body.row(24.0, |mut row| {
                                    row.col(|ui| {
                                        widgets::id_with_copy(
                                            ui,
                                            &peer.node_id,
                                            &mut app.state.status_message,
                                        );
                                    });
                                    row.col(|ui| {
                                        ui.monospace(&peer.address);
                                    });
                                    row.col(|ui| {
                                        if peer.is_connected {
                                            widgets::status_pill(
                                                ui,
                                                "Connected",
                                                egui::Color32::GREEN,
                                            );
                                        } else {
                                            widgets::status_pill(
                                                ui,
                                                "Disconnected",
                                                egui::Color32::GRAY,
                                            );
                                        }
                                    });
                                    row.col(|ui| {
                                        // Destructive disconnect: red outline button
                                        if widgets::danger_button(ui, "Disconnect").clicked() {
                                            disconnect_node_id = Some(peer.node_id.clone());
                                        }
                                    });
                                });
                            }
                        });
                });
            }
        }
        None => {
            widgets::empty_state(ui, "👥", "No peers connected", "Click Refresh to load");
        }
    }

    if let Some(node_id) = disconnect_node_id {
        app.disconnect_peer(node_id);
    }
}

pub fn render_dialogs(ctx: &egui::Context, app: &mut LspServerApp) {
    render_connect_peer_dialog(ctx, app);
}

fn render_connect_peer_dialog(ctx: &egui::Context, app: &mut LspServerApp) {
    if !app.state.show_connect_peer_dialog {
        return;
    }

    egui::Window::new("Connect Peer")
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            let form = &mut app.state.forms.connect_peer;

            ui.label("Connect to a Lightning Network peer");
            ui.add_space(5.0);

            egui::Grid::new("connect_peer_grid")
                .num_columns(2)
                .spacing([10.0, 5.0])
                .show(ui, |ui| {
                    widgets::label_with_info(ui, "Node Pubkey:", HELP_PEER_NODE_ID);
                    ui.text_edit_singleline(&mut form.node_pubkey);
                    ui.end_row();

                    widgets::label_with_info(ui, "Address:", HELP_PEER_ADDRESS);
                    ui.text_edit_singleline(&mut form.address);
                    ui.end_row();

                    ui.label("Persist Connection:");
                    ui.checkbox(&mut form.persist, "");
                    ui.end_row();
                });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                let is_pending = app.state.tasks.connect_peer.is_some();
                if is_pending {
                    ui.spinner();
                } else if ui.button("Connect").clicked() {
                    app.connect_peer();
                }
                if ui.button("Cancel").clicked() {
                    app.state.show_connect_peer_dialog = false;
                    app.state.forms.connect_peer = Default::default();
                }
            });
        });
}
