use egui::Ui;
#[cfg(target_arch = "wasm32")]
use web_sys::js_sys;

use crate::app::LspServerApp;
use crate::config::ChainSourceConfig;
use crate::ui::layout::{card, kv_grid_custom_info, page, page_scrolled};
use crate::ui::widgets;

const HELP_NODE_ID: &str =
    "The public key that identifies this Lightning node to peers and the network.";
const HELP_BEST_BLOCK: &str = "The best known block for this node's wallet, shown by block hash and height. If this height trails your chain source tip, the node may still be syncing.";
const HELP_NETWORK: &str =
    "The Bitcoin network this node is connected to, such as mainnet, testnet, signet, or regtest.";
const HELP_CHAIN_SOURCE: &str =
    "The blockchain backend used for headers, transactions, fee data, and wallet sync.";
const HELP_RPC_ADDRESS: &str =
    "The Bitcoin Core RPC endpoint used by the node for chain data and wallet-related checks.";
const HELP_ELECTRUM_URL: &str =
    "The Electrum server used by the node to scan and monitor the chain.";
const HELP_ESPLORA_URL: &str = "The Esplora API endpoint used by the node to query chain data.";
const HELP_LIGHTNING_WALLET_SYNC: &str =
    "The last time Lightning wallet state was synced against the chain source.";
const HELP_ONCHAIN_WALLET_SYNC: &str =
    "The last time the on-chain wallet scanned or synced against the chain source.";
const HELP_FEE_RATE_CACHE_UPDATE: &str =
    "The last time the node refreshed fee-rate estimates used when building on-chain transactions.";
const HELP_RGS_SNAPSHOT: &str =
	"The last Rapid Gossip Sync snapshot applied to update the Lightning network graph for route finding.";
const HELP_NODE_ANNOUNCEMENT: &str =
    "The last time this node broadcast its public node announcement to the Lightning network.";

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
        best_block_hash: info
            .current_best_block
            .as_ref()
            .map(|b| b.block_hash.clone()),
        best_block_height: info.current_best_block.as_ref().map(|b| b.height),
        ln_sync_ts: info.latest_lightning_wallet_sync_timestamp,
        onchain_sync_ts: info.latest_onchain_wallet_sync_timestamp,
        fee_rate_ts: info.latest_fee_rate_cache_update_timestamp,
        rgs_ts: info.latest_rgs_snapshot_timestamp,
        announcement_ts: info.latest_node_announcement_broadcast_timestamp,
    });

    page_scrolled(ui, |ui| {
        card(ui, "Node Details", |ui| {
            render_node_details_body(ui, app, &row)
        });
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

    let mut rows: crate::ui::layout::KvInfoRows = Vec::new();

    rows.push((
        "Node ID",
        Some(HELP_NODE_ID),
        Box::new(|ui: &mut Ui| {
            widgets::id_with_copy(ui, &r.node_id, &mut app.state.status_message);
        }),
    ));

    if let (Some(hash), Some(height)) = (&r.best_block_hash, r.best_block_height) {
        rows.push((
            "Best Block",
            Some(HELP_BEST_BLOCK),
            Box::new(move |ui: &mut Ui| {
                ui.monospace(format!(
                    "{} (height: {})",
                    crate::ui::truncate_id(hash, 8, 8),
                    height
                ));
            }),
        ));
    }

    if let Some(ts) = r.ln_sync_ts {
        rows.push((
            "Lightning Wallet Sync",
            Some(HELP_LIGHTNING_WALLET_SYNC),
            Box::new(move |ui: &mut Ui| {
                ui.label(format_timestamp(ts))
                    .on_hover_text(format!("unix: {}", ts));
            }),
        ));
    }

    if let Some(ts) = r.onchain_sync_ts {
        rows.push((
            "On-chain Wallet Sync",
            Some(HELP_ONCHAIN_WALLET_SYNC),
            Box::new(move |ui: &mut Ui| {
                ui.label(format_timestamp(ts))
                    .on_hover_text(format!("unix: {}", ts));
            }),
        ));
    }

    if let Some(ts) = r.fee_rate_ts {
        rows.push((
            "Fee Rate Cache Update",
            Some(HELP_FEE_RATE_CACHE_UPDATE),
            Box::new(move |ui: &mut Ui| {
                ui.label(format_timestamp(ts))
                    .on_hover_text(format!("unix: {}", ts));
            }),
        ));
    }

    if let Some(ts) = r.rgs_ts {
        rows.push((
            "RGS Snapshot",
            Some(HELP_RGS_SNAPSHOT),
            Box::new(move |ui: &mut Ui| {
                ui.label(format_timestamp(ts))
                    .on_hover_text(format!("unix: {}", ts));
            }),
        ));
    }

    if let Some(ts) = r.announcement_ts {
        rows.push((
            "Node Announcement",
            Some(HELP_NODE_ANNOUNCEMENT),
            Box::new(move |ui: &mut Ui| {
                ui.label(format_timestamp(ts))
                    .on_hover_text(format!("unix: {}", ts));
            }),
        ));
    }

    kv_grid_custom_info(ui, "node_details_grid", rows);
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

    let mut rows: crate::ui::layout::KvInfoRows = Vec::new();

    if !app.state.network.is_empty() {
        let network = app.state.network.clone();
        rows.push((
            "Network",
            Some(HELP_NETWORK),
            Box::new(move |ui: &mut Ui| {
                ui.monospace(&network);
            }),
        ));
    }

    match &app.state.chain_source {
        ChainSourceConfig::None => {}
        ChainSourceConfig::Bitcoind {
            rpc_address,
            rpc_user,
            rpc_password,
        } => {
            rows.push((
                "Chain Source",
                Some(HELP_CHAIN_SOURCE),
                Box::new(|ui: &mut Ui| {
                    ui.colored_label(egui::Color32::from_rgb(255, 165, 0), "Bitcoin Core RPC");
                }),
            ));

            let addr = rpc_address.clone();
            rows.push((
                "RPC Address",
                Some(HELP_RPC_ADDRESS),
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
                None,
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
                None,
                Box::new(move |ui: &mut Ui| {
                    ui.horizontal(|ui| {
                        ui.monospace("********");
                        if ui.small_button("Copy").clicked() {
                            ui.output_mut(|o| o.copied_text = password.clone());
                        }
                    });
                }),
            ));
        }
        ChainSourceConfig::Electrum { server_url } => {
            rows.push((
                "Chain Source",
                Some(HELP_CHAIN_SOURCE),
                Box::new(|ui: &mut Ui| {
                    ui.colored_label(egui::Color32::from_rgb(100, 149, 237), "Electrum");
                }),
            ));

            let url = server_url.clone();
            rows.push((
                "Server URL",
                Some(HELP_ELECTRUM_URL),
                Box::new(move |ui: &mut Ui| {
                    ui.horizontal(|ui| {
                        ui.monospace(&url);
                        if ui.small_button("Copy").clicked() {
                            ui.output_mut(|o| o.copied_text = url.clone());
                        }
                    });
                }),
            ));
        }
        ChainSourceConfig::Esplora { server_url } => {
            rows.push((
                "Chain Source",
                Some(HELP_CHAIN_SOURCE),
                Box::new(|ui: &mut Ui| {
                    ui.colored_label(egui::Color32::from_rgb(50, 205, 50), "Esplora");
                }),
            ));

            let url = server_url.clone();
            rows.push((
                "Server URL",
                Some(HELP_ESPLORA_URL),
                Box::new(move |ui: &mut Ui| {
                    ui.horizontal(|ui| {
                        ui.monospace(&url);
                        if ui.small_button("Copy").clicked() {
                            ui.output_mut(|o| o.copied_text = url.clone());
                        }
                    });
                }),
            ));
        }
    }

    kv_grid_custom_info(ui, "chain_source_grid", rows);
}
