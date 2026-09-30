//! Shared application state injected into every axum handler.

use std::path::PathBuf;
use std::sync::Arc;

use ldk_server_client::client::LdkServerClient;
use stable_channels::db::Database;
use tokio::sync::Mutex;

use crate::push::PushService;
use crate::stable_manager::StableChannelManager;

#[derive(Clone)]
pub struct AppState {
    /// gRPC client for LDK Server (used by proxy handlers).
    pub ldk_server: Arc<LdkServerClient>,
    /// HMAC api_key for this daemon's REST surface (used by auth middleware).
    pub api_key: Arc<Vec<u8>>,
    /// SC daemon's data directory (audit log, sqlite, etc.).
    pub data_dir: PathBuf,
    /// Network name, e.g. "regtest" / "bitcoin".
    pub network: String,
    /// sqlite handle to the SC daemon's stable channels database.
    pub db: Arc<Database>,
    /// Push notification service. Wrapped in a tokio mutex because `notify` mutates the cooldown map.
    pub push: Arc<Mutex<PushService>>,
    /// In-memory + sqlite-backed stable channel manager.
    pub stable_manager: Arc<Mutex<StableChannelManager>>,
    /// Offline IP-to-country lookup for stable counterparties' connection history.
    pub geoip: Arc<dyn crate::geoip::CountryLookup>,
    /// Revenue snapshot rebuilt off the request path.
    pub revenue: Arc<crate::revenue::RevenueStore>,
    /// LDK Server's log file path, resolved at daemon startup. None if not configured.
    pub ldk_log_file: Option<PathBuf>,
    /// Compatibility field: durable stream-gap boundary loaded at startup, NOT newest ledger
    /// activity. The event loop reloads the authoritative checkpoint (fixtures may leave None).
    #[allow(dead_code)]
    pub last_event_before_start_ms: Option<i64>,
}
