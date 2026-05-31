mod auth;
mod config;
mod event_loop;
mod handlers;
mod price_task;
mod push;
mod stability_tick;
mod stable_manager;
mod state;
mod tls;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::routing::post;
use axum::Router;
use clap::Parser;
use ldk_server_client::client::LdkServerClient;
use ldk_server_client::config as ldk_config;
use sc_protos::stable::{
    AUDIT_LOG_PATH, EDIT_STABLE_CHANNEL_PATH, GET_PRICE_PATH, LDK_LOG_PATH,
    LIST_STABLE_CHANNELS_PATH, REGISTER_PUSH_PATH,
};
use ldk_server_client::ldk_server_grpc::endpoints::{
    BOLT11_RECEIVE_PATH, BOLT11_SEND_PATH, BOLT12_RECEIVE_PATH, BOLT12_SEND_PATH,
    CLOSE_CHANNEL_PATH, CONNECT_PEER_PATH, DISCONNECT_PEER_PATH, EXPORT_PATHFINDING_SCORES_PATH,
    FORCE_CLOSE_CHANNEL_PATH, GET_PAYMENT_DETAILS_PATH, GRAPH_GET_CHANNEL_PATH,
    GRAPH_GET_NODE_PATH, GRAPH_LIST_CHANNELS_PATH, GRAPH_LIST_NODES_PATH,
    LIST_FORWARDED_PAYMENTS_PATH, LIST_PAYMENTS_PATH, LIST_PEERS_PATH, ONCHAIN_RECEIVE_PATH,
    ONCHAIN_SEND_PATH, OPEN_CHANNEL_PATH, SIGN_MESSAGE_PATH, SPLICE_IN_PATH, SPLICE_OUT_PATH,
    SPONTANEOUS_SEND_PATH, UPDATE_CHANNEL_CONFIG_PATH, VERIFY_SIGNATURE_PATH,
};
use stable_channels::audit::set_audit_log_path;
use stable_channels::db::Database;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[derive(Parser, Debug)]
#[command(name = "stable-channels-lsp", about = "Stable Channels LSP daemon")]
struct Cli {
    /// Path to the SC daemon's config.toml.
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = Config::load(&cli.config).map_err(|e| anyhow::anyhow!(e))?;
    info!("loaded config from {}", cli.config.display());

    let data_dir = PathBuf::from(&cfg.storage.disk.dir_path);
    let network_dir = data_dir.join(&cfg.node.network);
    std::fs::create_dir_all(&network_dir)
        .with_context(|| format!("Failed to create network dir {}", network_dir.display()))?;

    let tls_config = tls::get_or_generate_tls_config(None, &cfg.storage.disk.dir_path)
        .map_err(|e| anyhow::anyhow!(e))?;

    let api_key = ensure_local_api_key(&cfg)?;
    info!(
        "SC daemon api_key located at {}",
        cfg.local_api_key_path().display()
    );

    set_audit_log_path(
        cfg.audit_log_path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("audit_log_path is not valid UTF-8"))?,
    );

    let (ldk_server, ldk_addr) = build_ldk_server_client(&cfg)?;
    info!("LDK Server gRPC endpoint: {}", ldk_addr);

    let ldk_log_file = cfg.resolve_ldk_log_file();
    match &ldk_log_file {
        Some(p) => info!("LDK Server log file path: {}", p.display()),
        None => info!("LDK Server log file path not configured; /LdkLog will return empty"),
    }

    let db = Database::open(&data_dir).map_err(|e| anyhow::anyhow!("DB open failed: {}", e))?;
    let channel_count = db
        .load_all_channels()
        .map_err(|e| anyhow::anyhow!("load_all_channels failed: {}", e))?
        .len();
    info!("loaded {} stable channel records from sqlite", channel_count);
    let db_arc = Arc::new(db);

    let push_cfg = cfg.push.clone().unwrap_or_default();
    let push_service = crate::push::PushService::new(&push_cfg, &data_dir);
    info!("push service initialized");

    let stable_manager = crate::stable_manager::StableChannelManager::new(
        Arc::clone(&db_arc),
        data_dir.clone(),
    );

    let state = AppState {
        ldk_server: Arc::new(ldk_server),
        api_key: Arc::new(api_key),
        data_dir: data_dir.clone(),
        network: cfg.node.network.clone(),
        db: db_arc,
        push: Arc::new(tokio::sync::Mutex::new(push_service)),
        stable_manager: Arc::new(tokio::sync::Mutex::new(stable_manager)),
        ldk_log_file,
    };

    tokio::spawn(async move {
        price_task::run().await;
    });

    // One-shot reconcile from gRPC to catch up snapshots changed while the daemon was down. Skipped if the price cache is cold.
    {
        let btc_price = stable_channels::price_feeds::get_cached_price();
        if btc_price > 0.0 {
            let mut mgr = state.stable_manager.lock().await;
            mgr.reconcile_from_grpc(
                state.ldk_server.as_ref() as &dyn crate::stable_manager::LdkServerCalls,
                btc_price,
            )
            .await;
        } else {
            tracing::warn!("startup reconcile skipped: price cache cold");
        }
    }
    event_loop::spawn(state.clone());
    stability_tick::spawn(state.clone());

    let router = Router::new()
        .route("/GetNodeInfo", post(handlers::proxy::get_node_info))
        .route("/GetBalances", post(handlers::proxy::get_balances))
        .route("/ListChannels", post(handlers::proxy::list_channels))
        // peers
        .route(&format!("/{}", LIST_PEERS_PATH), post(handlers::peers::list_peers))
        .route(&format!("/{}", CONNECT_PEER_PATH), post(handlers::peers::connect_peer))
        .route(&format!("/{}", DISCONNECT_PEER_PATH), post(handlers::peers::disconnect_peer))
        // payments
        .route(&format!("/{}", LIST_PAYMENTS_PATH), post(handlers::payments::list_payments))
        .route(&format!("/{}", GET_PAYMENT_DETAILS_PATH), post(handlers::payments::get_payment_details))
        .route(&format!("/{}", LIST_FORWARDED_PAYMENTS_PATH), post(handlers::payments::list_forwarded_payments))
        // lightning
        .route(&format!("/{}", BOLT11_RECEIVE_PATH), post(handlers::lightning::bolt11_receive))
        .route(&format!("/{}", BOLT11_SEND_PATH), post(handlers::lightning::bolt11_send))
        .route(&format!("/{}", BOLT12_RECEIVE_PATH), post(handlers::lightning::bolt12_receive))
        .route(&format!("/{}", BOLT12_SEND_PATH), post(handlers::lightning::bolt12_send))
        .route(&format!("/{}", SPONTANEOUS_SEND_PATH), post(handlers::lightning::spontaneous_send))
        // onchain
        .route(&format!("/{}", ONCHAIN_RECEIVE_PATH), post(handlers::onchain::onchain_receive))
        .route(&format!("/{}", ONCHAIN_SEND_PATH), post(handlers::onchain::onchain_send))
        // channels
        .route(&format!("/{}", OPEN_CHANNEL_PATH), post(handlers::channels::open_channel))
        .route(&format!("/{}", CLOSE_CHANNEL_PATH), post(handlers::channels::close_channel))
        .route(&format!("/{}", FORCE_CLOSE_CHANNEL_PATH), post(handlers::channels::force_close_channel))
        .route(&format!("/{}", SPLICE_IN_PATH), post(handlers::channels::splice_in))
        .route(&format!("/{}", SPLICE_OUT_PATH), post(handlers::channels::splice_out))
        .route(&format!("/{}", UPDATE_CHANNEL_CONFIG_PATH), post(handlers::channels::update_channel_config))
        // graph
        .route(&format!("/{}", GRAPH_LIST_CHANNELS_PATH), post(handlers::graph::graph_list_channels))
        .route(&format!("/{}", GRAPH_GET_CHANNEL_PATH), post(handlers::graph::graph_get_channel))
        .route(&format!("/{}", GRAPH_LIST_NODES_PATH), post(handlers::graph::graph_list_nodes))
        .route(&format!("/{}", GRAPH_GET_NODE_PATH), post(handlers::graph::graph_get_node))
        // tools
        .route(&format!("/{}", SIGN_MESSAGE_PATH), post(handlers::tools::sign_message))
        .route(&format!("/{}", VERIFY_SIGNATURE_PATH), post(handlers::tools::verify_signature))
        .route(&format!("/{}", EXPORT_PATHFINDING_SCORES_PATH), post(handlers::tools::export_pathfinding_scores))
        // SC-specific
        .route(
            &format!("/{}", GET_PRICE_PATH),
            post(handlers::price::get_price),
        )
        .route(
            &format!("/{}", LIST_STABLE_CHANNELS_PATH),
            post(handlers::stable_channels::list_stable_channels),
        )
        .route(
            &format!("/{}", EDIT_STABLE_CHANNEL_PATH),
            post(handlers::stable_channels::edit_stable_channel),
        )
        .route(
            &format!("/{}", REGISTER_PUSH_PATH),
            post(handlers::register_push::register_push),
        )
        .route(
            &format!("/{}", AUDIT_LOG_PATH),
            post(handlers::audit_log::audit_log),
        )
        .route(
            &format!("/{}", LDK_LOG_PATH),
            post(handlers::ldk_log::ldk_log),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listen_addr: std::net::SocketAddr =
        cfg.node.rest_service_address.parse().with_context(|| {
            format!(
                "Failed to parse listen address '{}'",
                cfg.node.rest_service_address
            )
        })?;

    info!("listening on https://{}", listen_addr);

    let rustls_config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls_config));
    axum_server::bind_rustls(listen_addr, rustls_config)
        .serve(router.into_make_service())
        .await
        .context("axum_server crashed")?;

    Ok(())
}

/// Read or auto-generate the api_key file. Returns the hex-encoded bytes used as the HMAC key.
fn ensure_local_api_key(cfg: &Config) -> Result<Vec<u8>> {
    let path = cfg.local_api_key_path();
    let raw = if path.exists() {
        std::fs::read(&path)
            .with_context(|| format!("Failed to read api_key file {}", path.display()))?
    } else {
        let mut key = vec![0u8; 32];
        use ring::rand::SecureRandom;
        let rng = ring::rand::SystemRandom::new();
        rng.fill(&mut key)
            .map_err(|_| anyhow::anyhow!("SecureRandom::fill failed"))?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create parent dir for api_key: {}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(&path, &key)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        info!("generated new SC api_key at {}", path.display());
        key
    };

    let mut hex = String::with_capacity(raw.len() * 2);
    for b in &raw {
        hex.push_str(&format!("{:02x}", b));
    }
    Ok(hex.into_bytes())
}

/// Build the gRPC client and return it together with the resolved base URL.
fn build_ldk_server_client(cfg: &Config) -> Result<(LdkServerClient, String)> {
    let ldk_cfg_opt = if let Some(p) = &cfg.ldk_server.config_path {
        Some(ldk_config::load_config(&PathBuf::from(p)).map_err(|e| anyhow::anyhow!(e))?)
    } else {
        None
    };
    let ldk_cfg_ref = ldk_cfg_opt.as_ref();

    let base_url = if let Some(addr) = &cfg.ldk_server.grpc_address {
        addr.clone()
    } else {
        ldk_config::resolve_base_url(None, ldk_cfg_ref)
    };

    let cert_path = if let Some(p) = &cfg.ldk_server.cert_path {
        PathBuf::from(p)
    } else {
        ldk_config::resolve_cert_path(None, ldk_cfg_ref)
            .ok_or_else(|| anyhow::anyhow!("Could not resolve LDK Server TLS cert path"))?
    };
    let cert_pem = std::fs::read(&cert_path)
        .with_context(|| format!("Failed to read LDK Server cert at {}", cert_path.display()))?;

    let api_key = if let Some(p) = &cfg.ldk_server.api_key_path {
        let bytes = std::fs::read(p)
            .with_context(|| format!("Failed to read api_key file {}", p))?;
        bytes_to_lower_hex(&bytes)
    } else {
        ldk_config::resolve_api_key(None, ldk_cfg_ref)
            .ok_or_else(|| anyhow::anyhow!("Could not resolve LDK Server api_key"))?
    };

    let client = LdkServerClient::new(base_url.clone(), api_key, &cert_pem)
        .map_err(|e| anyhow::anyhow!("LdkServerClient::new failed: {}", e))?;
    Ok((client, base_url))
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
