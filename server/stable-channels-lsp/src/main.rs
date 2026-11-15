mod auth;
mod backfill;
mod channel_audit;
mod channel_close;
mod config;
mod event_loop;
mod geoip;
mod handlers;
pub mod messages;
mod observability;
mod price_task;
mod payment_filter;
mod push;
mod record_limits;
mod revenue;
mod secret_file;
mod stability_tick;
mod stable_manager;
mod state;
mod tls;
mod trade_response_retry;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::routing::post;
use axum::Router;
use clap::Parser;
use ldk_server_client::client::LdkServerClient;
use ldk_server_client::config as ldk_config;
use sc_protos::revenue::{GET_REVENUE_PATH, REFUND_TRADE_FEE_PATH};
use sc_protos::stable::{
    AUDIT_LOG_PATH, EDIT_STABLE_CHANNEL_PATH, GET_PRICE_PATH, LDK_LOG_PATH,
    LIST_CHANNEL_LEDGER_EVENTS_PATH, LIST_SETTLEMENT_PAYMENTS_PATH, LIST_STABLE_CHANNELS_PATH,
    REGISTER_PUSH_PATH, RELEASE_STABILITY_PAYMENT_PATH,
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
use stable_channels::audit::{set_audit_ledger, set_audit_log_path};
use stable_channels::db::Database;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

/// Longest wait between attempts to load the channel list at startup.
const STARTUP_RECONCILE_MAX_RETRY_SECS: u64 = 60;

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
    secret_file::create_private_dir_all(&network_dir)
        .with_context(|| format!("Failed to create network dir {}", network_dir.display()))?;

    let tls_opts = tls::TlsConfig { cert_path: None, key_path: None, hosts: cfg.tls.hosts.clone() };
    let tls_config = tls::get_or_generate_tls_config(Some(tls_opts), &cfg.storage.disk.dir_path)
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
    match db.import_legacy_audit_log(&cfg.audit_log_path()) {
        Ok(import) => info!(
            "channel ledger legacy import: imported={}, skipped={}, operational={}, already_imported={}",
            import.imported, import.skipped, import.operational, import.already_imported
        ),
        Err(error) => warn!(
            "channel ledger legacy import skipped after error; continuing with SQLite ledger: {}",
            error
        ),
    }
    set_audit_ledger(db.clone());
    // Persist the restart gap before any stream/reconciliation tasks start. Audit traffic is not
    // evidence of stream health; a failed checkpoint must not silently invent startup coverage.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let last_event_before_start_ms = Some(db.begin_event_stream_gap(now_ms)
        .context("failed to persist event-stream startup gap")?.0);
    let channel_count = db
        .load_all_channels()
        .map_err(|e| anyhow::anyhow!("load_all_channels failed: {}", e))?
        .len();
    info!("loaded {} stable channel records from sqlite", channel_count);
    let db_arc = Arc::new(db);

    let push_cfg = cfg.push.clone().unwrap_or_default();
    let push_service = crate::push::PushService::new(&push_cfg, &data_dir);
    info!("push service initialized");

    let mut stable_manager = crate::stable_manager::StableChannelManager::new(
        Arc::clone(&db_arc),
        data_dir.clone(),
    );
    stable_manager.enforce_max_stabilization = cfg.enforce_max_stabilization;

    let state = AppState {
        ldk_server: Arc::new(ldk_server),
        api_key: Arc::new(api_key),
        data_dir: data_dir.clone(),
        network: cfg.node.network.clone(),
        db: db_arc,
        push: Arc::new(tokio::sync::Mutex::new(push_service)),
        push_registrations: Arc::new(crate::push::registrations::Registrations::new(&data_dir)),
        stable_manager: Arc::new(tokio::sync::Mutex::new(stable_manager)),
        geoip: geoip::load(cfg.geoip.country_db.as_deref()),
        geoip_record_ips: cfg.geoip.record_ips.unwrap_or(cfg.geoip.country_db.is_some()),
        geoip_retention_secs: cfg.geoip.retention_days.unwrap_or(90) as i64 * 86_400,
        revenue: Arc::new(revenue::RevenueStore::default()),
        funding_outpoints: Arc::default(),
        ldk_log_file,
        last_event_before_start_ms,
    };

    let (price_tx, _price_rx) = tokio::sync::watch::channel(0.0_f64);
    tokio::spawn(async move {
        price_task::run(price_tx).await;
    });

    // Load the channel list from the database and the node before events, ticks or edits act on it. The books need no price, so this does not wait for one; it retries until it succeeds.
    {
        let state = state.clone();
        tokio::spawn(async move {
            let ldk: &dyn crate::stable_manager::LdkServerCalls = state.ldk_server.as_ref();
            let mut retry = Duration::from_secs(1);
            loop {
                let peers = {
                    let mut mgr = state.stable_manager.lock().await;
                    if !mgr.is_hydrated() {
                        let btc_price = stable_channels::price_feeds::get_fresh_cached_price_no_fetch();
                        mgr.reconcile_from_grpc(ldk, btc_price).await;
                    }
                    mgr.channel_peer_ids()
                };
                if peers.is_some() {
                    // Registrations held since start are stored now rather than at the first tick after the load.
                    crate::stability_tick::share_channel_peers(&state.push_registrations, peers);
                    break;
                }
                tracing::warn!("startup reconcile failed; retrying in {:?}", retry);
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(Duration::from_secs(STARTUP_RECONCILE_MAX_RETRY_SECS));
            }
        });
    }
    event_loop::spawn(state.clone());
    stability_tick::spawn(state.clone());
    observability::spawn(state.clone());
    trade_response_retry::spawn(state.clone());
    revenue::spawn(state.clone());

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
            &format!("/{}", RELEASE_STABILITY_PAYMENT_PATH),
            post(handlers::stable_channels::release_stability_payment),
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
            &format!("/{}", LIST_CHANNEL_LEDGER_EVENTS_PATH),
            post(handlers::channel_ledger::list_channel_ledger_events),
        )
        .route(
            &format!("/{}", LDK_LOG_PATH),
            post(handlers::ldk_log::ldk_log),
        )
        .route(
            &format!("/{}", LIST_SETTLEMENT_PAYMENTS_PATH),
            post(handlers::stable_channels::list_settlement_payments),
        )
        .route(
            &format!("/{}", GET_REVENUE_PATH),
            post(handlers::revenue::get_revenue),
        )
        .route(
            &format!("/{}", REFUND_TRADE_FEE_PATH),
            post(handlers::revenue::refund_trade_fee),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        // Legacy push registration for deployed wallets: unsigned JSON, no HMAC.
        // Registered AFTER the auth layer so it is exempt from it (axum layers only
        // wrap routes added before them). Audit-flagged REGISTER_PUSH_LEGACY_*.
        .route(
            "/api/register-push",
            post(handlers::register_push::register_push_legacy),
        )
        // Restore guard for wallets: "does my node_id still have a channel?"
        // asked before a seed-only restore wipes LDK state and force-closes it.
        .route(
            "/api/channel-exists",
            post(handlers::channel_exists::channel_exists),
        )
        // A restored wallet's funding outpoints, to recover a force close; the node key signs the request.
        .route(
            "/api/funding-outpoints",
            post(handlers::funding_outpoints::funding_outpoints),
        )
        // Defense-in-depth body cap covering every route, including the endpoints outside the auth layer
        // that never reach the auth middleware's own capped read.
        .layer(axum::extract::DefaultBodyLimit::max(auth::MAX_AUTH_BODY_BYTES))
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

/// Size of the SC daemon's api key in bytes.
const API_KEY_LEN: usize = 32;

/// Read or auto-generate the api_key file. Returns the hex-encoded bytes used as the HMAC key.
fn ensure_local_api_key(cfg: &Config) -> Result<Vec<u8>> {
    let path = cfg.local_api_key_path();
    // symlink_metadata, so a dangling symlink counts as present and is never written through.
    let raw = match std::fs::symlink_metadata(&path) {
        Ok(_) => read_api_key(&path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut key = vec![0u8; API_KEY_LEN];
            use ring::rand::SecureRandom;
            let rng = ring::rand::SystemRandom::new();
            rng.fill(&mut key)
                .map_err(|_| anyhow::anyhow!("SecureRandom::fill failed"))?;

            if let Some(parent) = path.parent() {
                secret_file::create_private_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create parent dir for api_key: {}",
                        parent.display()
                    )
                })?;
            }
            secret_file::write_new(&path, &key, 0o600)
                .with_context(|| format!("Failed to write {}", path.display()))?;
            info!("generated new SC api_key at {}", path.display());
            key
        },
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to check api_key file {}", path.display()))
        },
    };

    Ok(bytes_to_lower_hex(&raw).into_bytes())
}

/// Read a stored api key, refusing one that is not exactly `API_KEY_LEN` bytes, and make it readable by its owner only.
fn read_api_key(path: &std::path::Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to read api_key file {}", path.display()))?;
    let mut raw = Vec::with_capacity(API_KEY_LEN + 1);
    // One byte past the expected size is enough to tell a longer file apart.
    (&file)
        .take(API_KEY_LEN as u64 + 1)
        .read_to_end(&mut raw)
        .with_context(|| format!("Failed to read api_key file {}", path.display()))?;
    if raw.len() != API_KEY_LEN {
        let size = if raw.len() > API_KEY_LEN {
            format!("more than {API_KEY_LEN} bytes")
        } else {
            format!("{} bytes", raw.len())
        };
        anyhow::bail!(
            "api_key file {} holds {size}, expected exactly {API_KEY_LEN}; refusing to start. \
             Stop the daemon, delete the file and start again to generate a new key, then give the new key to the operator GUI",
            path.display()
        );
    }
    match secret_file::make_owner_only(&file) {
        Ok(Some(old_mode)) => warn!(
            "api_key file {} was open to its group or other users (mode {:o}); set it to {:o}, owner only. \
             If anyone else may have read it, generate a new key: stop the daemon, delete the file and start again",
            path.display(),
            old_mode,
            old_mode & 0o700
        ),
        Ok(None) => {},
        Err(error) => warn!(
            "could not make api_key file {} owner only: {}",
            path.display(),
            error
        ),
    }
    Ok(raw)
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
            .map_err(|e| anyhow::anyhow!(e))?
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn config_with_storage(dir: &std::path::Path) -> Config {
        let mut cfg: Config = toml::from_str(
            r#"
            [node]
            rest_service_address = "127.0.0.1:3002"
            network = "regtest"
            [storage.disk]
            dir_path = ""
            [ldk_server]
            "#,
        )
        .unwrap();
        cfg.storage.disk.dir_path = dir.to_str().unwrap().to_owned();
        cfg
    }

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_generated_api_key_is_32_bytes_readable_by_its_owner_only_in_a_private_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with_storage(&dir.path().join("data"));

        let key = ensure_local_api_key(&cfg).unwrap();

        let path = cfg.local_api_key_path();
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw.len(), API_KEY_LEN);
        assert_eq!(key, bytes_to_lower_hex(&raw).into_bytes());
        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(mode_of(path.parent().unwrap()), 0o700);
        assert_eq!(mode_of(&dir.path().join("data")), 0o700);
        assert_eq!(ensure_local_api_key(&cfg).unwrap(), key, "the stored key is reused");
    }

    #[test]
    fn a_stored_api_key_open_to_its_group_or_others_is_made_owner_only_and_still_used() {
        for old_mode in [0o644, 0o664, 0o640] {
            let dir = tempfile::tempdir().unwrap();
            let cfg = config_with_storage(dir.path());
            let path = cfg.local_api_key_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, [0x42u8; API_KEY_LEN]).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(old_mode)).unwrap();

            let key = ensure_local_api_key(&cfg).unwrap();

            assert_eq!(key, bytes_to_lower_hex(&[0x42u8; API_KEY_LEN]).into_bytes());
            assert_eq!(mode_of(&path), 0o600, "from {old_mode:o}");
        }
    }

    #[test]
    fn a_stored_api_key_of_the_wrong_size_stops_startup() {
        for size in [0, 1, API_KEY_LEN - 1, API_KEY_LEN + 1, 4096] {
            let dir = tempfile::tempdir().unwrap();
            let cfg = config_with_storage(dir.path());
            let path = cfg.local_api_key_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, vec![0x42u8; size]).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

            let error = ensure_local_api_key(&cfg).unwrap_err().to_string();

            assert!(error.contains("expected exactly 32"), "size {size}: {error}");
            assert_eq!(std::fs::read(&path).unwrap().len(), size, "the file is left for the operator");
        }
    }

    #[test]
    fn a_dangling_api_key_symlink_stops_startup_without_writing_through_it() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with_storage(dir.path());
        let path = cfg.local_api_key_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = dir.path().join("elsewhere");
        std::os::unix::fs::symlink(&target, &path).unwrap();

        assert!(ensure_local_api_key(&cfg).is_err());

        assert!(!target.exists());
    }
}
