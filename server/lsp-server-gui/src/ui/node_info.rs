use egui::Ui;
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::config::ChainSourceConfig;
use crate::ui::layout::{card, kv_grid_custom, page, page_scrolled};
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

	// Chain source is local config — visible even while disconnected (as before the redesign).
	page(ui, |ui| {
		card(ui, "Chain Source", |ui| render_chain_source_body(ui, app));
	});

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

	page_scrolled(ui, |ui| {
		card(ui, "Node Details", |ui| render_node_details_body(ui, app, &row));
	});
}

fn render_node_details_body(ui: &mut Ui, app: &mut LspServerApp, row: &Option<NodeInfoRow>) {
	ui.horizontal(|ui| {
		widgets::status_pill(ui, "Online", egui::Color32::GREEN);
		if app.state.tasks.node_info.is_some() {
			ui.spinner();
		} else if ui.button("Refresh").clicked() {
			app.fetch_node_info();
		}
	});
	ui.add_space(5.0);

	let Some(r) = row else {
		ui.label("No node info available. Click Refresh to fetch.");
		return;
	};

	let mut rows: crate::ui::layout::KvRows = Vec::new();

	rows.push((
		"Node ID",
		Box::new(|ui: &mut Ui| {
			widgets::id_with_copy(ui, &r.node_id, &mut app.state.status_message);
		}),
	));

	if let (Some(hash), Some(height)) = (&r.best_block_hash, r.best_block_height) {
		rows.push((
			"Best Block",
			Box::new(move |ui: &mut Ui| {
				ui.monospace(format!("{} (height: {})", crate::ui::truncate_id(hash, 8, 8), height));
			}),
		));
	}

	if let Some(ts) = r.ln_sync_ts {
		rows.push((
			"Lightning Wallet Sync",
			Box::new(move |ui: &mut Ui| {
				ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
			}),
		));
	}

	if let Some(ts) = r.onchain_sync_ts {
		rows.push((
			"On-chain Wallet Sync",
			Box::new(move |ui: &mut Ui| {
				ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
			}),
		));
	}

	if let Some(ts) = r.fee_rate_ts {
		rows.push((
			"Fee Rate Cache Update",
			Box::new(move |ui: &mut Ui| {
				ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
			}),
		));
	}

	if let Some(ts) = r.rgs_ts {
		rows.push((
			"RGS Snapshot",
			Box::new(move |ui: &mut Ui| {
				ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
			}),
		));
	}

	if let Some(ts) = r.announcement_ts {
		rows.push((
			"Node Announcement",
			Box::new(move |ui: &mut Ui| {
				ui.label(format_timestamp(ts)).on_hover_text(format!("unix: {}", ts));
			}),
		));
	}

	kv_grid_custom(ui, "node_details_grid", rows);
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

fn render_chain_source_body(ui: &mut Ui, app: &LspServerApp) {
	// Only show if we have chain source info from config
	if matches!(app.state.chain_source, ChainSourceConfig::None) && app.state.network.is_empty() {
		ui.label("No chain source configured.");
		return;
	}

	let mut rows: crate::ui::layout::KvRows = Vec::new();

	if !app.state.network.is_empty() {
		let network = app.state.network.clone();
		rows.push(("Network", Box::new(move |ui: &mut Ui| { ui.monospace(&network); })));
	}

	match &app.state.chain_source {
		ChainSourceConfig::None => {},
		ChainSourceConfig::Bitcoind { rpc_address, rpc_user, rpc_password } => {
			rows.push((
				"Chain Source",
				Box::new(|ui: &mut Ui| {
					ui.colored_label(egui::Color32::from_rgb(255, 165, 0), "Bitcoin Core RPC");
				}),
			));

			let addr = rpc_address.clone();
			rows.push((
				"RPC Address",
				Box::new(move |ui: &mut Ui| {
					ui.horizontal(|ui| {
						ui.monospace(&addr);
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = addr.clone());
						}
					});
				}),
			));

			let user = rpc_user.clone();
			rows.push((
				"RPC User",
				Box::new(move |ui: &mut Ui| {
					ui.horizontal(|ui| {
						ui.monospace(&user);
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = user.clone());
						}
					});
				}),
			));

			let password = rpc_password.clone();
			rows.push((
				"RPC Password",
				Box::new(move |ui: &mut Ui| {
					ui.horizontal(|ui| {
						ui.monospace("********");
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = password.clone());
						}
					});
				}),
			));
		},
		ChainSourceConfig::Electrum { server_url } => {
			rows.push((
				"Chain Source",
				Box::new(|ui: &mut Ui| {
					ui.colored_label(egui::Color32::from_rgb(100, 149, 237), "Electrum");
				}),
			));

			let url = server_url.clone();
			rows.push((
				"Server URL",
				Box::new(move |ui: &mut Ui| {
					ui.horizontal(|ui| {
						ui.monospace(&url);
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = url.clone());
						}
					});
				}),
			));
		},
		ChainSourceConfig::Esplora { server_url } => {
			rows.push((
				"Chain Source",
				Box::new(|ui: &mut Ui| {
					ui.colored_label(egui::Color32::from_rgb(50, 205, 50), "Esplora");
				}),
			));

			let url = server_url.clone();
			rows.push((
				"Server URL",
				Box::new(move |ui: &mut Ui| {
					ui.horizontal(|ui| {
						ui.monospace(&url);
						if ui.small_button("Copy").clicked() {
							ui.output_mut(|o| o.copied_text = url.clone());
						}
					});
				}),
			));
		},
	}

	kv_grid_custom(ui, "chain_source_grid", rows);
}
