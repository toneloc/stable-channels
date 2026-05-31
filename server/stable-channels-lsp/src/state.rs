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
    /// LDK Server's log file path, resolved at daemon startup. None if not configured.
    pub ldk_log_file: Option<PathBuf>,
}
