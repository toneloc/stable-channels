use egui::Ui;
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::config::ChainSourceConfig;
use crate::state::ConnectionStatus;
use crate::ui::connection;

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Node Information");
	ui.add_space(10.0);

	connection::render_settings(ui, app);
	ui.add_space(10.0);

	// Show chain source config if loaded
	render_chain_source_info(ui, app);
	ui.add_space(10.0);

	if !matches!(app.state.connection_status, ConnectionStatus::Connected) {
		ui.label("Connect to a server to view node information.");
		return;
	}

	ui.group(|ui| {
		ui.horizontal(|ui| {
			ui.heading("Node Details");
			if app.state.tasks.node_info.is_some() {
				ui.spinner();
			} else if ui.button("Refresh").clicked() {
				app.fetch_node_info();
			}
		});
		ui.add_space(5.0);

		if let Some(info) = &app.state.node_info {
			egui::Grid::new("node_info_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
				ui.label("Node ID:");
				ui.horizontal(|ui| {
					let node_id = &info.node_id;
					ui.monospace(crate::ui::truncate_id(node_id, 12, 12));
					if ui.small_button("Copy").clicked() {
						ui.output_mut(|o| o.copied_text = node_id.clone());
					}
				});
				ui.end_row();

				if let Some(block) = &info.current_best_block {
					ui.label("Best Block:");
					ui.monospace(format!(
						"{} (height: {})",
						crate::ui::truncate_id(&block.block_hash, 8, 8),
						block.height
					));
					ui.end_row();
				}

				if let Some(ts) = info.latest_lightning_wallet_sync_timestamp {
					ui.label("Lightning Wallet Sync:");
					ui.label(format_timestamp(ts));
					ui.end_row();
				}

				if let Some(ts) = info.latest_onchain_wallet_sync_timestamp {
					ui.label("On-chain Wallet Sync:");
					ui.label(format_timestamp(ts));
					ui.end_row();
				}

				if let Some(ts) = info.latest_fee_rate_cache_update_timestamp {
					ui.label("Fee Rate Cache Update:");
					ui.label(format_timestamp(ts));
					ui.end_row();
				}

				if let Some(ts) = info.latest_rgs_snapshot_timestamp {
					ui.label("RGS Snapshot:");
					ui.label(format_timestamp(ts));
					ui.end_row();
				}

				if let Some(ts) = info.latest_node_announcement_broadcast_timestamp {
					ui.label("Node Announcement:");
					ui.label(format_timestamp(ts));
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
