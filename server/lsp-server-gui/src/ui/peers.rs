use eframe::egui;

use crate::app::LspServerApp;
use crate::ui::widgets;

// Per-row snapshot extracted from state.peers so the state borrow is released
// before widgets::id_with_copy (needs &mut app.state.status_message) runs in the grid body.
struct PeerRow {
	node_id: String,
	address: String,
	is_connected: bool,
}

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Peers");
	ui.add_space(5.0);

	if ui.button("Refresh").clicked() {
		app.fetch_peers();
	}

	ui.add_space(10.0);

	let mut disconnect_node_id = None;

	// Pre-extract peer rows so the &app.state.peers borrow ends before the grid
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

				egui::Grid::new("peers_table").striped(true).min_col_width(80.0).show(ui, |ui| {
					ui.label(egui::RichText::new("Node ID").strong());
					ui.label(egui::RichText::new("Address").strong());
					ui.label(egui::RichText::new("Status").strong());
					ui.label(egui::RichText::new("Actions").strong());
					ui.end_row();

					for peer in &peers {
						widgets::id_with_copy(ui, &peer.node_id, &mut app.state.status_message);
						ui.monospace(&peer.address);
						if peer.is_connected {
							widgets::status_pill(ui, "Connected", egui::Color32::GREEN);
						} else {
							widgets::status_pill(ui, "Disconnected", egui::Color32::GRAY);
						}
						// Destructive disconnect: red text small button
						let btn = egui::Button::new(
							egui::RichText::new("Disconnect").color(egui::Color32::RED),
						)
						.small();
						if ui.add(btn).clicked() {
							disconnect_node_id = Some(peer.node_id.clone());
						}
						ui.end_row();
					}
				});
			}
		},
		None => {
			widgets::empty_state(ui, "👥", "No peers connected", "Click Refresh to load");
		},
	}

	if let Some(node_id) = disconnect_node_id {
		app.disconnect_peer(node_id);
	}
}
