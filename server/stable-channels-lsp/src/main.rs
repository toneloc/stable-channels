mod access;
mod auth;
mod backfill;
mod channel_audit;
mod channel_close;
mod config;
mod emails;
mod event_loop;
mod fixed_window;
mod geoip;
mod handlers;
mod ldk_client;
pub mod messages;
mod observability;
mod price_task;
mod payment_filter;
mod push;
mod queued_splice;
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
use axum::routing::{post, MethodRouter};
use axum::Router;
use clap::Parser;
use ldk_server_client::client::LdkServerClient;
use ldk_server_client::config as ldk_config;
use ldk_server_client::ldk_server_grpc::api::GetPermissionsRequest;
use sc_protos::access::{
    CREATE_TOKEN_PATH, LIST_TOKENS_PATH, LOGIN_PATH, LOGOUT_PATH, REVOKE_TOKEN_PATH, WHO_AM_I_PATH,
};
use sc_protos::revenue::{GET_REVENUE_PATH, REFUND_TRADE_FEE_PATH};
use sc_protos::stable::{
    AUDIT_LOG_PATH, CANCEL_QUEUED_SPLICE_PATH, EDIT_STABLE_CHANNEL_PATH, GET_PRICE_PATH,
    LDK_LOG_PATH, LIST_CHANNEL_LEDGER_EVENTS_PATH, LIST_QUEUED_SPLICES_PATH,
    LIST_SETTLEMENT_PAYMENTS_PATH, LIST_STABLE_CHANNELS_PATH, QUEUE_SPLICE_IN_PATH,
    REGISTER_PUSH_PATH, RELEASE_STABILITY_PAYMENT_PATH,
};
use ldk_server_client::ldk_server_grpc::endpoints::{
    BOLT11_RECEIVE_PATH, BOLT11_SEND_PATH, BOLT12_RECEIVE_PATH, BOLT12_SEND_PATH,
    CLOSE_CHANNEL_PATH, CONNECT_PEER_PATH, DISCONNECT_PEER_PATH, EXPORT_PATHFINDING_SCORES_PATH,
    FORCE_CLOSE_CHANNEL_PATH, GET_BALANCES_PATH, GET_NODE_INFO_PATH, GET_PAYMENT_DETAILS_PATH,
    GRAPH_GET_CHANNEL_PATH, GRAPH_GET_NODE_PATH, GRAPH_LIST_CHANNELS_PATH, GRAPH_LIST_NODES_PATH,
    LIST_CHANNELS_PATH, LIST_FORWARDED_PAYMENTS_PATH, LIST_PAYMENTS_PATH, LIST_PEERS_PATH, ONCHAIN_RECEIVE_PATH,
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

/// Longest wait between startup attempts to load the channel list, or to learn the daemon's own token.
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

    if let Some(path) = old_api_key_file(&cfg) {
        info!(
            "{} is no longer used: the GUI logs in with an LDK Server token; the file can be deleted",
            path.display()
        );
    }

    set_audit_log_path(
        cfg.audit_log_path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("audit_log_path is not valid UTF-8"))?,
    );

    let (ldk_server, ldk_endpoint) = build_ldk_server_client(&cfg)?;
    info!("LDK Server gRPC endpoint: {}", ldk_endpoint.base_url);

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

    let ldk_server = Arc::new(ldk_client::LdkClient::new(ldk_server));
    let tokens = access::tokens::LdkTokens::new(Arc::new(ldk_endpoint), Arc::clone(&ldk_server));
    let state = AppState {
        ldk_server,
        tokens: Arc::new(tokens),
        access: Arc::default(),
        gui_token_id: Arc::default(),
        data_dir: data_dir.clone(),
        network: cfg.node.network.clone(),
        db: db_arc,
        push: Arc::new(tokio::sync::Mutex::new(push_service)),
        push_registrations: Arc::new(crate::push::registrations::Registrations::new(&data_dir)),
        emails: crate::emails::Emails::open_or_off(&data_dir),
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
    // GUI logins open once LDK Server says who the daemon's token is; the REST server starts now regardless.
    tokio::spawn(open_gui_logins(state.clone()));
    event_loop::spawn(state.clone());
    stability_tick::spawn(state.clone());
    queued_splice::spawn(state.clone());
    observability::spawn(state.clone());
    trade_response_retry::spawn(state.clone());
    revenue::spawn(state.clone());

    let router = build_router(state);

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

/// Every route behind the session layer, by name, with its handler; each name needs an entry in the permission table.
const ROUTES: &[(&str, fn() -> MethodRouter<AppState>)] = &[
    (GET_NODE_INFO_PATH, || post(handlers::proxy::get_node_info)),
    (GET_BALANCES_PATH, || post(handlers::proxy::get_balances)),
    (LIST_CHANNELS_PATH, || post(handlers::proxy::list_channels)),
    // peers
    (LIST_PEERS_PATH, || post(handlers::peers::list_peers)),
    (CONNECT_PEER_PATH, || post(handlers::peers::connect_peer)),
    (DISCONNECT_PEER_PATH, || post(handlers::peers::disconnect_peer)),
    // payments
    (LIST_PAYMENTS_PATH, || post(handlers::payments::list_payments)),
    (GET_PAYMENT_DETAILS_PATH, || post(handlers::payments::get_payment_details)),
    (LIST_FORWARDED_PAYMENTS_PATH, || post(handlers::payments::list_forwarded_payments)),
    // lightning
    (BOLT11_RECEIVE_PATH, || post(handlers::lightning::bolt11_receive)),
    (BOLT11_SEND_PATH, || post(handlers::lightning::bolt11_send)),
    (BOLT12_RECEIVE_PATH, || post(handlers::lightning::bolt12_receive)),
    (BOLT12_SEND_PATH, || post(handlers::lightning::bolt12_send)),
    (SPONTANEOUS_SEND_PATH, || post(handlers::lightning::spontaneous_send)),
    // onchain
    (ONCHAIN_RECEIVE_PATH, || post(handlers::onchain::onchain_receive)),
    (ONCHAIN_SEND_PATH, || post(handlers::onchain::onchain_send)),
    // channels
    (OPEN_CHANNEL_PATH, || post(handlers::channels::open_channel)),
    (CLOSE_CHANNEL_PATH, || post(handlers::channels::close_channel)),
    (FORCE_CLOSE_CHANNEL_PATH, || post(handlers::channels::force_close_channel)),
    (SPLICE_IN_PATH, || post(handlers::channels::splice_in)),
    (SPLICE_OUT_PATH, || post(handlers::channels::splice_out)),
    (UPDATE_CHANNEL_CONFIG_PATH, || post(handlers::channels::update_channel_config)),
    // graph
    (GRAPH_LIST_CHANNELS_PATH, || post(handlers::graph::graph_list_channels)),
    (GRAPH_GET_CHANNEL_PATH, || post(handlers::graph::graph_get_channel)),
    (GRAPH_LIST_NODES_PATH, || post(handlers::graph::graph_list_nodes)),
    (GRAPH_GET_NODE_PATH, || post(handlers::graph::graph_get_node)),
    // tools
    (SIGN_MESSAGE_PATH, || post(handlers::tools::sign_message)),
    (VERIFY_SIGNATURE_PATH, || post(handlers::tools::verify_signature)),
    (EXPORT_PATHFINDING_SCORES_PATH, || post(handlers::tools::export_pathfinding_scores)),
    // SC-specific
    (GET_PRICE_PATH, || post(handlers::price::get_price)),
    (LIST_STABLE_CHANNELS_PATH, || post(handlers::stable_channels::list_stable_channels)),
    (EDIT_STABLE_CHANNEL_PATH, || post(handlers::stable_channels::edit_stable_channel)),
    (RELEASE_STABILITY_PAYMENT_PATH, || post(handlers::stable_channels::release_stability_payment)),
    (REGISTER_PUSH_PATH, || post(handlers::register_push::register_push)),
    (AUDIT_LOG_PATH, || post(handlers::audit_log::audit_log)),
    (LIST_CHANNEL_LEDGER_EVENTS_PATH, || post(handlers::channel_ledger::list_channel_ledger_events)),
    (LDK_LOG_PATH, || post(handlers::ldk_log::ldk_log)),
    (LIST_SETTLEMENT_PAYMENTS_PATH, || post(handlers::stable_channels::list_settlement_payments)),
    (GET_REVENUE_PATH, || post(handlers::revenue::get_revenue)),
    (REFUND_TRADE_FEE_PATH, || post(handlers::revenue::refund_trade_fee)),
    (QUEUE_SPLICE_IN_PATH, || post(handlers::queued_splices::queue_splice_in)),
    (LIST_QUEUED_SPLICES_PATH, || post(handlers::queued_splices::list_queued_splices)),
    (CANCEL_QUEUED_SPLICE_PATH, || post(handlers::queued_splices::cancel_queued_splice)),
    // sessions
    (LOGOUT_PATH, || post(access::handlers::logout)),
    (WHO_AM_I_PATH, || post(access::handlers::who_am_i)),
    // tokens
    (LIST_TOKENS_PATH, || post(access::manage::list_tokens)),
    (CREATE_TOKEN_PATH, || post(access::manage::create_token)),
    (REVOKE_TOKEN_PATH, || post(access::manage::revoke_token)),
];

/// Every route the daemon serves; Login and the wallet routes, added after the session layer, are exempt from it.
fn build_router(state: AppState) -> Router {
    let mut router = Router::new();
    for (route, handler) in ROUTES {
        router = router.route(&format!("/{route}"), handler());
    }
    router
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            access::middleware::session_layer,
        ))
        // The token in Login's body is its proof, so it sits outside the session layer and caps its body itself.
        .route(&format!("/{LOGIN_PATH}"), post(access::handlers::login))
        // Legacy push registration for deployed wallets: unsigned JSON, no HMAC.
        // Registered AFTER the session layer so it is exempt from it (axum layers only
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
        // Optional wallet email: proven by a node-key signature, not HMAC, so wallets with no channel can share one.
        .route(
            "/api/register-email",
            post(handlers::register_email::register_email),
        )
        // Defense-in-depth body cap covering every route, including the endpoints outside the session layer
        // that never reach its own capped read.
        .layer(axum::extract::DefaultBodyLimit::max(auth::MAX_AUTH_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The api_key file the daemon used before token logins, if it is still on disk; it is no longer read.
fn old_api_key_file(cfg: &Config) -> Option<PathBuf> {
    let path = cfg.local_api_key_path();
    std::fs::symlink_metadata(&path).is_ok().then_some(path)
}

/// Asks LDK Server about the daemon's own token until it answers, then opens GUI logins.
async fn open_gui_logins(state: AppState) {
    let mut retry = Duration::from_secs(1);
    let own = loop {
        match state.ldk_server.get_permissions(GetPermissionsRequest {}).await {
            Ok(response) => match response.macaroon {
                Some(info) => break access::tokens::TokenFacts::from(info),
                None => warn!(
                    "LDK Server's answer about the daemon's token had no details; GUI logins wait, retrying in {:?}",
                    retry
                ),
            },
            Err(error) => warn!(
                "could not ask LDK Server about the daemon's token; GUI logins wait, retrying in {:?}: {}",
                retry, error.message
            ),
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(Duration::from_secs(STARTUP_RECONCILE_MAX_RETRY_SECS));
    };
    access::start(&state, own).await;
}

/// Build the gRPC client and return it together with where it connects (base URL and cert).
fn build_ldk_server_client(cfg: &Config) -> Result<(LdkServerClient, access::tokens::LdkEndpoint)> {
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

    if let Some(p) = &cfg.ldk_server.api_key_path {
        warn!(
            "[ldk_server] api_key_path = {} is ignored: LDK Server now uses macaroons (set macaroon_path, or config_path to use its admin.macaroon)",
            p
        );
    }
    let macaroon = resolve_ldk_macaroon(cfg.ldk_server.macaroon_path.as_deref(), ldk_cfg_ref)?;

    let client = LdkServerClient::new(base_url.clone(), macaroon, &cert_pem)
        .map_err(|e| anyhow::anyhow!("LdkServerClient::new failed: {}", e))?;
    Ok((client, access::tokens::LdkEndpoint { base_url, cert_pem }))
}

/// LDK Server's macaroon: the `macaroon_path` file if set, else `admin.macaroon` in LDK Server's storage dir.
fn resolve_ldk_macaroon(macaroon_path: Option<&str>, ldk_cfg: Option<&ldk_config::Config>) -> Result<String> {
    if let Some(p) = macaroon_path {
        return read_macaroon_file(std::path::Path::new(p));
    }
    let default_path = ldk_config::resolve_macaroon_path(ldk_cfg).map_err(|e| anyhow::anyhow!(e))?;
    ldk_config::resolve_macaroon(None, ldk_cfg).map_err(|e| anyhow::anyhow!(e))?.ok_or_else(|| match default_path {
        Some(path) => anyhow::anyhow!(
            "LDK Server macaroon not found at {} (start LDK Server once to create it, or set [ldk_server] macaroon_path)",
            path.display()
        ),
        None => anyhow::anyhow!("Could not locate LDK Server's macaroon; set [ldk_server] macaroon_path or config_path"),
    })
}

fn read_macaroon_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    // Hex doubles the size; the slack covers a trailing newline.
    let limit = ldk_server_client::macaroon::MAX_MACAROON_BYTES * 2 + 16;
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open LDK Server macaroon file {}", path.display()))?;
    let mut raw = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut raw)
        .with_context(|| format!("Failed to read LDK Server macaroon file {}", path.display()))?;
    if raw.len() > limit {
        anyhow::bail!("LDK Server macaroon file {} is larger than {} bytes", path.display(), limit);
    }
    let token = std::str::from_utf8(&raw)
        .map(str::trim)
        .map_err(|_| anyhow::anyhow!("LDK Server macaroon file {} is not a hex macaroon (an old api_key file?)", path.display()))?;
    ldk_server_client::macaroon::parse_reusable_macaroon(token)
        .map_err(|e| anyhow::anyhow!("Invalid LDK Server macaroon in {}: {}", path.display(), e))?;
    Ok(token.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ldk_server_client::ldk_server_grpc::error::ErrorCode;
    use ldk_server_client::ldk_server_grpc::permissions::{ALL_PERMISSIONS, READONLY_PERMISSIONS};
    use prost::Message;
    use sc_protos::access::{required, session_header, LoginRequest, Requirement, WhoAmIResponse, SESSION_UNKNOWN};
    use tower::ServiceExt;

    use crate::access::testing::{
        fresh_nonce, login, offline_state, offline_state_before_logins, open_as, post, refusal, sign, signed_post,
        test_macaroon, FakeLdk,
    };

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

    #[tokio::test]
    async fn register_email_needs_no_admin_auth_but_keeps_the_body_cap() {
        let dir = tempfile::tempdir().unwrap();
        let router = build_router(offline_state(dir.path(), &FakeLdk::new()));
        let post = |path: &str, body: Vec<u8>| {
            Request::post(path)
                .body(axum::body::Body::from(body))
                .unwrap()
        };

        let admin = router
            .clone()
            .oneshot(post("/GetNodeInfo", vec![]))
            .await
            .unwrap();
        assert_eq!(
            admin.status(),
            StatusCode::UNAUTHORIZED,
            "admin routes need a session"
        );
        // With emails.db off the handler itself answers 503, so this status proves the request got past auth to it.
        let reached = router
            .clone()
            .oneshot(post("/api/register-email", b"{}".to_vec()))
            .await
            .unwrap();
        assert_eq!(reached.status(), StatusCode::SERVICE_UNAVAILABLE);
        let oversized = router
            .oneshot(post(
                "/api/register-email",
                vec![b'x'; auth::MAX_AUTH_BODY_BYTES + 1],
            ))
            .await
            .unwrap();
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    fn route_names() -> impl Iterator<Item = &'static str> {
        ROUTES.iter().map(|(route, _)| *route)
    }

    #[tokio::test]
    async fn admin_routes_need_a_session() {
        use bitcoin_hashes::hmac::{Hmac, HmacEngine};
        use bitcoin_hashes::{sha256, Hash, HashEngine};
        let dir = tempfile::tempdir().unwrap();
        let router = build_router(offline_state(dir.path(), &FakeLdk::new()));
        // What the old daemon accepted: HMAC over ts and body, keyed with the hex of its 32-byte api_key.
        let ts = access::unix_now();
        let mut engine: HmacEngine<sha256::Hash> = HmacEngine::new("42".repeat(32).as_bytes());
        engine.input(&ts.to_be_bytes());
        let old_header = format!("HMAC {}:{:x}", ts, Hmac::<sha256::Hash>::from_engine(engine));
        let unknown_session = session_header("00112233445566778899aabbccddeeff", ts, &fresh_nonce(), &"0".repeat(64));

        for auth in [None, Some(old_header.as_str()), Some(unknown_session.as_str())] {
            let (status, bytes) = post(&router, "/GetNodeInfo", auth, vec![]).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{auth:?}");
            assert_eq!(refusal(&bytes), (ErrorCode::AuthError, SESSION_UNKNOWN.to_string()), "{auth:?}");
        }
    }

    #[test]
    fn every_route_behind_the_session_layer_is_in_the_permission_table() {
        let routes: Vec<&str> = route_names().collect();
        assert_eq!(routes.iter().collect::<HashSet<_>>().len(), routes.len(), "a route is listed twice");
        for route in routes {
            match required(route) {
                Some(Requirement::Permission(permission)) => {
                    assert!(ALL_PERMISSIONS.contains(&permission), "{route} needs {permission}, which LDK doesn't know")
                }
                Some(Requirement::AnySession) => {}
                other => panic!("{route} is behind the session layer but the table says {other:?}"),
            }
        }
        assert!(route_names().any(|r| r == "Logout") && route_names().any(|r| r == "WhoAmI"));
        assert!(!route_names().any(|r| r == "Login"), "Login proves itself and sits outside the layer");
    }

    #[test]
    fn the_ldk_backed_rows_match_ldk_server() {
        // Copied from method_authorization in ldk-server/src/macaroons/authorization.rs at aa2d60f5.
        let ldk_rows: &[(&str, &str)] = &[
            ("GetNodeInfo", "node:read"),
            ("GetBalances", "node:read"),
            ("ExportPathfindingScores", "node:read"),
            ("OnchainReceive", "onchain:receive"),
            ("OnchainSend", "onchain:send"),
            ("Bolt11Receive", "invoices:create"),
            ("Bolt12Receive", "invoices:create"),
            ("Bolt11Send", "payments:send"),
            ("Bolt12Send", "payments:send"),
            ("SpontaneousSend", "payments:send"),
            ("SpliceOut", "payments:send"),
            ("GetPaymentDetails", "payments:read"),
            ("ListPayments", "payments:read"),
            ("ListForwardedPayments", "payments:read"),
            ("ListChannels", "channels:read"),
            ("OpenChannel", "channels:manage"),
            ("UpdateChannelConfig", "channels:manage"),
            ("CloseChannel", "channels:manage"),
            ("SpliceIn", "channels:manage"),
            ("ForceCloseChannel", "channels:force_close"),
            ("ListPeers", "peers:read"),
            ("ConnectPeer", "peers:manage"),
            ("DisconnectPeer", "peers:manage"),
            ("SignMessage", "messages:sign"),
            ("VerifySignature", "messages:verify"),
            ("GraphListChannels", "graph:read"),
            ("GraphGetChannel", "graph:read"),
            ("GraphListNodes", "graph:read"),
            ("GraphGetNode", "graph:read"),
        ];
        for (route, permission) in ldk_rows {
            assert_eq!(required(route), Some(Requirement::Permission(permission)), "{route}");
            assert!(route_names().any(|r| r == *route), "{route} is proxied but not served");
        }
    }

    #[tokio::test]
    async fn login_then_whoami_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let token = fake.add(1, "night-shift", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));

        let session = login(&router, &token).await;
        assert_eq!(session.token_name, "night-shift");
        let (status, bytes) = post(&router, "/WhoAmI", Some(&sign(&session, "WhoAmI", b"")), vec![]).await;
        assert_eq!(status, StatusCode::OK, "{:?}", refusal(&bytes));
        let me = WhoAmIResponse::decode(&bytes[..]).unwrap();
        assert_eq!(me.token_id, session.token_id);
        assert_eq!((me.token_name.as_str(), me.permissions), ("night-shift", vec!["admin".to_string()]));

        let (status, _) = post(&router, "/Logout", Some(&sign(&session, "Logout", b"")), vec![]).await;
        assert_eq!(status, StatusCode::OK);
        let (status, bytes) = post(&router, "/WhoAmI", Some(&sign(&session, "WhoAmI", b"")), vec![]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(refusal(&bytes), (ErrorCode::AuthError, SESSION_UNKNOWN.to_string()), "logged out");
    }

    #[tokio::test]
    async fn a_read_only_session_is_refused_before_the_handler_runs() {
        use sc_protos::stable::EditStableChannelRequest;
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let token = fake.add(2, "auditor", &READONLY_PERMISSIONS);
        let state = offline_state(dir.path(), &fake);
        let user_channel_id = "5e1d0c4b3a29f8e7d6c5b4a392817060";
        state.db.save_channel("chan-7f3a", user_channel_id, 125.0, 0, 50_000, Some("before")).unwrap();
        let rows = |state: &AppState| format!("{:?}", state.db.load_all_channels().unwrap());
        let (rows_before, events_before) = (rows(&state), state.db.max_ledger_event_id().unwrap());
        let router = build_router(state.clone());
        let session = login(&router, &token).await;

        let edit = EditStableChannelRequest {
            channel_id: "chan-7f3a".into(),
            expected_usd: Some(1.0),
            note: Some("after".into()),
        };
        let body = edit.encode_to_vec();
        let auth = sign(&session, "EditStableChannel", &body);
        let (status, bytes) = post(&router, "/EditStableChannel", Some(&auth), body).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(refusal(&bytes), (ErrorCode::AuthError, "this token lacks channels:manage".to_string()));
        assert_eq!(rows(&state), rows_before);
        assert_eq!(state.db.max_ledger_event_id().unwrap(), events_before);
    }

    #[tokio::test]
    async fn a_signature_covers_the_route_it_was_made_for() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let token = fake.add(3, "operator", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));
        let session = login(&router, &token).await;

        let (status, bytes) = post(&router, "/WhoAmI", Some(&sign(&session, "Logout", b"")), vec![]).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(refusal(&bytes), (ErrorCode::AuthError, "request signature does not match".to_string()));
    }

    #[tokio::test]
    async fn until_ldk_answers_about_the_daemon_token_logins_wait_and_wallet_routes_still_serve() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let token = fake.add(4, "operator", &["admin"]);
        let state = offline_state_before_logins(dir.path(), &fake);
        let router = build_router(state.clone());

        let body = LoginRequest { token: token.clone() }.encode_to_vec();
        let (status, bytes) = post(&router, "/Login", None, body).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(refusal(&bytes), (ErrorCode::InternalServerError, "Can't reach LDK Server".to_string()));
        let (status, bytes) = post(&router, "/GetNodeInfo", None, vec![]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(refusal(&bytes), (ErrorCode::AuthError, SESSION_UNKNOWN.to_string()));
        let (status, _) = post(&router, "/api/register-email", None, b"{}".to_vec()).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "the wallet route's own handler answered");

        open_as(&state, &fake, &["admin"]);
        let session = login(&router, &token).await;
        assert_eq!(session.permissions, ["admin"]);
    }

    #[tokio::test]
    async fn a_daemon_token_that_is_not_admin_caps_every_session() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let token = fake.add(5, "operator", &["admin"]);
        let state = offline_state_before_logins(dir.path(), &fake);
        open_as(&state, &fake, &["node:read", "channels:read", "macaroons:manage"]);
        let router = build_router(state);

        let session = login(&router, &token).await;

        assert_eq!(session.permissions, ["channels:read", "macaroons:manage", "node:read"]);
        let (status, bytes) = post(&router, "/OnchainSend", Some(&sign(&session, "OnchainSend", b"")), vec![]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(refusal(&bytes), (ErrorCode::AuthError, "this token lacks onchain:send".to_string()));
    }

    #[tokio::test]
    async fn a_daemon_token_without_macaroons_manage_lets_no_one_log_in() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let token = fake.add(5, "operator", &["admin"]);
        let state = offline_state_before_logins(dir.path(), &fake);
        open_as(&state, &fake, &["node:read", "channels:read"]);
        let router = build_router(state);

        let (status, bytes) = post(&router, "/Login", None, LoginRequest { token }.encode_to_vec()).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "LDK Server won't list tokens to this daemon token");
        let expected = (ErrorCode::AuthError, "LDK Server refused this token (wrong, revoked or expired)".to_string());
        assert_eq!(refusal(&bytes), expected);
    }

    #[tokio::test]
    async fn proxy_calls_go_out_with_the_callers_token() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let token = fake.add(6, "operator", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));

        let (status, bytes) = signed_post(&router, &token, "GetNodeInfo", vec![]).await;

        assert_ne!(status, StatusCode::OK);
        let (_, message) = refusal(&bytes);
        // The caller's client points at localhost:2 and the daemon's at localhost:1.
        assert!(message.contains("localhost:2"), "{message}");
    }

    #[tokio::test]
    async fn splice_in_still_uses_the_daemon_token() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let token = fake.add(7, "operator", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));
        let body = ldk_server_client::ldk_server_grpc::api::SpliceInRequest { user_channel_id: "9".into(), ..Default::default() }
            .encode_to_vec();

        let (status, bytes) = signed_post(&router, &token, "SpliceIn", body).await;

        assert_ne!(status, StatusCode::OK);
        let (_, message) = refusal(&bytes);
        assert!(message.contains("localhost:1"), "{message}");
    }

    #[tokio::test]
    async fn stable_channel_locations_need_peers_read() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        let channels_only = fake.add(8, "channels", &["channels:read"]);
        let readonly = fake.add(9, "readonly", &READONLY_PERMISSIONS);
        let mut state = offline_state(dir.path(), &fake);
        state.geoip_record_ips = true;
        let node = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".to_string();
        state.db.record_peer_sighting(&node, "1.2.3.4", None, 10).unwrap();
        state.stable_manager.lock().await.stable_channels.push(stable_channels::types::StableChannel {
            counterparty: node.parse().unwrap(),
            ..Default::default()
        });
        let router = build_router(state);

        let locations = |bytes: &[u8]| {
            let resp = sc_protos::stable::ListStableChannelsResponse::decode(bytes).unwrap();
            assert_eq!(resp.channels.len(), 1);
            resp.channels[0].recent_locations.len()
        };
        let (status, bytes) = signed_post(&router, &channels_only, "ListStableChannels", vec![]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(locations(&bytes), 0);
        let (status, bytes) = signed_post(&router, &readonly, "ListStableChannels", vec![]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(locations(&bytes), 1);
    }

    #[tokio::test]
    async fn login_refusals_never_echo_the_token_and_the_body_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeLdk::new();
        fake.add(6, "operator", &["admin"]);
        let router = build_router(offline_state(dir.path(), &fake));
        let unlisted = test_macaroon(b"0f1e2d3c4b5a69788796a5b4c3d2e1f0");

        for token in ["not a token".to_string(), unlisted.clone()] {
            let body = LoginRequest { token: token.clone() }.encode_to_vec();
            let (status, bytes) = post(&router, "/Login", None, body).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let (code, message) = refusal(&bytes);
            assert_eq!(code, ErrorCode::AuthError);
            assert_eq!(message, "LDK Server refused this token (wrong, revoked or expired)");
            assert!(!message.contains(&token));
        }

        let declared = Request::post("/Login")
            .header(axum::http::header::CONTENT_LENGTH, (auth::MAX_AUTH_BODY_BYTES + 1).to_string())
            .body(Body::from(LoginRequest { token: unlisted }.encode_to_vec()))
            .unwrap();
        let response = router.clone().oneshot(declared).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let (status, _) = post(&router, "/Login", None, vec![b'x'; auth::MAX_AUTH_BODY_BYTES + 1]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "a body over the cap is not read in full");
    }

    #[test]
    fn an_old_api_key_file_is_noted_and_left_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with_storage(dir.path());
        assert_eq!(old_api_key_file(&cfg), None);

        let path = cfg.local_api_key_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [0x42u8; 32]).unwrap();

        assert_eq!(old_api_key_file(&cfg), Some(path.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), [0x42u8; 32]);
    }

    fn ldk_config_with_storage(dir: &std::path::Path) -> ldk_config::Config {
        let path = dir.join("ldk-server.toml");
        let text = format!(
            "[node]\nnetwork = \"regtest\"\ngrpc_service_address = \"127.0.0.1:3536\"\n[storage.disk]\ndir_path = \"{}\"\n",
            dir.display()
        );
        std::fs::write(&path, text).unwrap();
        ldk_config::load_config(&path).unwrap()
    }

    fn write_admin_macaroon(storage: &std::path::Path, contents: &str) -> PathBuf {
        let path = storage.join("regtest").join("macaroons").join("admin.macaroon");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[tokio::test]
    async fn a_config_still_naming_the_old_api_key_connects_with_ldk_servers_admin_macaroon() {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        let ldk_dir = dir.path().join("ldk");
        std::fs::create_dir_all(&ldk_dir).unwrap();
        ldk_config_with_storage(&ldk_dir);
        tls::get_or_generate_tls_config(None, ldk_dir.to_str().unwrap()).unwrap();
        write_admin_macaroon(&ldk_dir, &test_macaroon(b"admin"));
        let old_key = ldk_dir.join("regtest").join("api_key");
        std::fs::write(&old_key, [0x42u8; 32]).unwrap();
        let mut cfg = config_with_storage(&dir.path().join("sc"));
        cfg.ldk_server.config_path = Some(ldk_dir.join("ldk-server.toml").to_str().unwrap().to_owned());
        cfg.ldk_server.api_key_path = Some(old_key.to_str().unwrap().to_owned());

        let (_, endpoint) = build_ldk_server_client(&cfg).unwrap();

        assert_eq!(endpoint.base_url, "127.0.0.1:3536");
    }

    #[test]
    fn the_ldk_server_macaroon_comes_from_macaroon_path_with_its_newline_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let token = test_macaroon(b"sc-daemon");
        let path = dir.path().join("sc-daemon.macaroon");
        std::fs::write(&path, format!("{token}\n")).unwrap();

        let resolved = resolve_ldk_macaroon(Some(path.to_str().unwrap()), None).unwrap();

        assert_eq!(resolved, token);
    }

    #[test]
    fn without_macaroon_path_the_admin_macaroon_in_ldk_servers_storage_dir_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let ldk_cfg = ldk_config_with_storage(dir.path());
        let admin = test_macaroon(b"admin");
        write_admin_macaroon(dir.path(), &format!("{admin}\n"));

        assert_eq!(resolve_ldk_macaroon(None, Some(&ldk_cfg)).unwrap(), admin);

        let scoped = test_macaroon(b"scoped");
        let scoped_path = dir.path().join("scoped.macaroon");
        std::fs::write(&scoped_path, &scoped).unwrap();
        let resolved = resolve_ldk_macaroon(Some(scoped_path.to_str().unwrap()), Some(&ldk_cfg)).unwrap();
        assert_eq!(resolved, scoped, "macaroon_path wins over LDK Server's admin.macaroon");
    }

    #[test]
    fn a_missing_admin_macaroon_stops_startup_and_names_where_it_looked() {
        let dir = tempfile::tempdir().unwrap();
        let ldk_cfg = ldk_config_with_storage(dir.path());
        let expected = dir.path().join("regtest").join("macaroons").join("admin.macaroon");

        let error = resolve_ldk_macaroon(None, Some(&ldk_cfg)).unwrap_err().to_string();

        assert!(error.contains(&expected.display().to_string()), "{error}");
    }

    #[test]
    fn an_old_api_key_file_is_not_accepted_as_a_macaroon() {
        let dir = tempfile::tempdir().unwrap();
        let ldk_cfg = ldk_config_with_storage(dir.path());
        let old_key = dir.path().join("regtest").join("api_key");
        std::fs::create_dir_all(old_key.parent().unwrap()).unwrap();
        for raw in [vec![0xdeu8; 32], vec![0x42u8; 32]] {
            std::fs::write(&old_key, &raw).unwrap();

            let error = resolve_ldk_macaroon(Some(old_key.to_str().unwrap()), Some(&ldk_cfg))
                .unwrap_err()
                .to_string();

            assert!(error.contains(&old_key.display().to_string()), "{error}");
        }
        let admin_path = write_admin_macaroon(dir.path(), "4242");
        let error = resolve_ldk_macaroon(None, Some(&ldk_cfg)).unwrap_err().to_string();
        assert!(error.contains(&admin_path.display().to_string()), "{error}");
    }

    #[test]
    fn an_oversized_macaroon_file_is_refused_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.macaroon");
        std::fs::write(&path, "a".repeat(ldk_server_client::macaroon::MAX_MACAROON_BYTES * 2 + 17)).unwrap();

        let error = resolve_ldk_macaroon(Some(path.to_str().unwrap()), None).unwrap_err().to_string();

        assert!(error.contains("larger than"), "{error}");
    }
}
