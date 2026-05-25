use eframe::egui;

use crate::app::LspServerApp;
use crate::ui::truncate_id;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Peers");
	ui.add_space(5.0);

	if ui.button("Refresh").clicked() {
		app.fetch_peers();
	}

	ui.add_space(10.0);

	let mut disconnect_node_id = None;

	match &app.state.peers {
		Some(resp) => {
			if resp.peers.is_empty() {
				ui.label("No peers connected.");
			} else {
				egui::Grid::new("peers_table").striped(true).min_col_width(80.0).show(ui, |ui| {
					ui.label(egui::RichText::new("Node ID").strong());
					ui.label(egui::RichText::new("Address").strong());
					ui.label(egui::RichText::new("Connected").strong());
					ui.label(egui::RichText::new("Actions").strong());
					ui.end_row();

					for peer in &resp.peers {
						ui.horizontal(|ui| {
							ui.label(
								egui::RichText::new(truncate_id(&peer.node_id, 8, 4)).monospace(),
							);
							if ui.small_button("Copy").clicked() {
								ui.output_mut(|o| o.copied_text = peer.node_id.clone());
							}
						});
						ui.label(&peer.address);
						ui.label(if peer.is_connected { "Yes" } else { "No" });
						if ui.small_button("Disconnect").clicked() {
							disconnect_node_id = Some(peer.node_id.clone());
						}
						ui.end_row();
					}
				});
			}
		},
		None => {
			ui.label("Not loaded yet. Click Refresh.");
		},
	}

	if let Some(node_id) = disconnect_node_id {
		app.disconnect_peer(node_id);
	}
}
