use egui::Ui;
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::config::ChainSourceConfig;
use crate::ui::widgets;

// Snapshot of node_info fields extracted before any &mut app borrow.
struct NodeInfoRow {
	node_id: String,
	best_block_hash: Option<String>,
	best_block_height: Option<u32>,
	ln_sync_ts: Option<u64>,
	onchain_sync_ts: Option<u64>,
	fee_rate_ts: Option<u64>,
	rgs_ts: Option<u64>,
	announcement_ts: Option<u64>,
}

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Node Information");
	ui.add_space(10.0);

	// Show chain source config if loaded
	render_chain_source_info(ui, app);
	ui.add_space(10.0);

	if app.render_disconnected_gate(ui) {
		return;
	}

	// Pre-extract node_info fields so the immutable borrow ends before the grid
	// calls widgets::id_with_copy (which needs &mut app.state.status_message).
	let row: Option<NodeInfoRow> = app.state.node_info.as_ref().map(|info| NodeInfoRow {
		node_id: info.node_id.clone(),
		best_block_hash: info.current_best_block.as_ref().map(|b| b.block_hash.clone()),
		best_block_height: info.current_best_block.as_ref().map(|b| b.height),
		ln_sync_ts: info.latest_lightning_wallet_sync_timestamp,
		onchain_sync_ts: info.latest_onchain_wallet_sync_timestamp,
		fee_rate_ts: info.latest_fee_rate_cache_update_timestamp,
		rgs_ts: info.latest_rgs_snapshot_timestamp,
		announcement_ts: info.latest_node_announcement_broadcast_timestamp,
	});

	ui.group(|ui| {
		ui.horizontal(|ui| {
			ui.heading("Node Details");
			widgets::status_pill(ui, "Online", egui::Color32::GREEN);
			if app.state.tasks.node_info.is_some() {
				ui.spinner();
			} else if ui.button("Refresh").clicked() {
				app.fetch_node_info();
			}
		});
		ui.add_space(5.0);

		if let Some(r) = row {
			egui::Grid::new("node_info_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
				ui.label("Node ID:");
				widgets::id_with_copy(ui, &r.node_id, &mut app.state.status_message);
				ui.end_row();

				if let (Some(hash), Some(height)) = (r.best_block_hash, r.best_block_height) {
					ui.label("Best Block:");
					ui.monospace(format!(
						"{} (height: {})",
						crate::ui::truncate_id(&hash, 8, 8),
						height
					));
					ui.end_row();
				}

				if let Some(ts) = r.ln_sync_ts {
					ui.label("Lightning Wallet Sync:");
					ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
					ui.end_row();
				}

				if let Some(ts) = r.onchain_sync_ts {
					ui.label("On-chain Wallet Sync:");
					ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
					ui.end_row();
				}

				if let Some(ts) = r.fee_rate_ts {
					ui.label("Fee Rate Cache Update:");
					ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
					ui.end_row();
				}

				if let Some(ts) = r.rgs_ts {
					ui.label("RGS Snapshot:");
					ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
					ui.end_row();
				}

				if let Some(ts) = r.announcement_ts {
					ui.label("Node Announcement:");
					ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
					ui.end_row();
				}
			});
		} else {
			ui.label("No node info available. Click Refresh to fetch.");
		}
	});
}

fn format_timestamp(ts: u64) -> String {
	// Get current time - use js_sys on WASM, SystemTime on native
	#[cfg(target_arch = "wasm32")]
	let now_secs = (js_sys::Date::now() / 1000.0) as u64;

	#[cfg(not(target_arch = "wasm32"))]
	let now_secs = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);

	// Calculate how long ago
	if now_secs >= ts {
		let secs = now_secs - ts;
		if secs < 60 {
			format!("{} seconds ago", secs)
		} else if secs < 3600 {
			format!("{} minutes ago", secs / 60)
		} else if secs < 86400 {
			format!("{} hours ago", secs / 3600)
		} else {
			format!("{} days ago", secs / 86400)
		}
	} else {
		// Future timestamp or error - just show relative to epoch
		format!("timestamp: {}", ts)
	}
}

fn render_chain_source_info(ui: &mut Ui, app: &LspServerApp) {
	// Only show if we have chain source info from config
	if matches!(app.state.chain_source, ChainSourceConfig::None) && app.state.network.is_empty() {
		return;
	}

	ui.group(|ui| {
		ui.heading("Chain Source Configuration");
		ui.add_space(5.0);

		egui::Grid::new("chain_source_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			if !app.state.network.is_empty() {
				ui.label("Network:");
				ui.monospace(&app.state.network);
				ui.end_row();
			}

			match &app.state.chain_source {
				ChainSourceConfig::None => {},
				ChainSourceConfig::Bitcoind { rpc_address, rpc_user, rpc_password } => {
					ui.label("Chain Source:");
					ui.colored_label(egui::Color32::from_rgb(255, 165, 0), "Bitcoin Core RPC");
					ui.end_row();

					ui.label("RPC Address:");
					ui.horizontal(|ui| {
						ui.monospace(rpc_address);
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = rpc_address.clone());
						}
					});
					ui.end_row();

					ui.label("RPC User:");
					ui.horizontal(|ui| {
						ui.monospace(rpc_user);
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = rpc_user.clone());
						}
					});
					ui.end_row();

					ui.label("RPC Password:");
					ui.horizontal(|ui| {
						ui.monospace("********");
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = rpc_password.clone());
						}
					});
					ui.end_row();
				},
				ChainSourceConfig::Electrum { server_url } => {
					ui.label("Chain Source:");
					ui.colored_label(egui::Color32::from_rgb(100, 149, 237), "Electrum");
					ui.end_row();

					ui.label("Server URL:");
					ui.horizontal(|ui| {
						ui.monospace(server_url);
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = server_url.clone());
						}
					});
					ui.end_row();
				},
				ChainSourceConfig::Esplora { server_url } => {
					ui.label("Chain Source:");
					ui.colored_label(egui::Color32::from_rgb(50, 205, 50), "Esplora");
					ui.end_row();

					ui.label("Server URL:");
					ui.horizontal(|ui| {
						ui.monospace(server_url);
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = server_url.clone());
						}
					});
					ui.end_row();
				},
			}
		});
	});
}
