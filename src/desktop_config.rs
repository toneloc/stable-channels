use std::path::PathBuf;

use crate::constants::{
    DEFAULT_CHAIN_URL, DEFAULT_LSP_ADDRESS, DEFAULT_LSP_PUBKEY, DEFAULT_NETWORK, DEFAULT_USER_PORT,
    FALLBACK_CHAIN_URL,
};

const E2E_FLAG: &str = "SC_E2E";
const NETWORK_ENV: &str = "SC_MAC_NETWORK";
const CHAIN_URL_ENV: &str = "SC_MAC_CHAIN_URL";
const FALLBACK_CHAIN_URL_ENV: &str = "SC_MAC_FALLBACK_CHAIN_URL";
const LSP_PUBKEY_ENV: &str = "SC_MAC_LSP_PUBKEY";
const LSP_ADDRESS_ENV: &str = "SC_MAC_LSP_ADDRESS";
const USER_PORT_ENV: &str = "SC_MAC_USER_PORT";
const USER_DATA_DIR_ENV: &str = "SC_MAC_USER_DATA_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopRuntimeConfig {
    pub network: String,
    pub primary_chain_url: String,
    pub fallback_chain_url: String,
    pub lsp_pubkey: String,
    pub lsp_address: String,
    pub user_port: u16,
    pub user_data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopRuntimeConfigError {
    MissingEnv(&'static str),
    InvalidUserPort(String),
    UnsafeE2EValue(&'static str, String),
}

impl std::fmt::Display for DesktopRuntimeConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEnv(key) => write!(f, "{key} must be set when SC_E2E=1"),
            Self::InvalidUserPort(value) => write!(f, "SC_MAC_USER_PORT is invalid: {value}"),
            Self::UnsafeE2EValue(key, value) => {
                write!(f, "{key} has an unsafe E2E value: {value}")
            }
        }
    }
}

impl std::error::Error for DesktopRuntimeConfigError {}

impl Default for DesktopRuntimeConfig {
    fn default() -> Self {
        Self {
            network: DEFAULT_NETWORK.to_string(),
            primary_chain_url: DEFAULT_CHAIN_URL.to_string(),
            fallback_chain_url: FALLBACK_CHAIN_URL.to_string(),
            lsp_pubkey: DEFAULT_LSP_PUBKEY.to_string(),
            lsp_address: DEFAULT_LSP_ADDRESS.to_string(),
            user_port: DEFAULT_USER_PORT,
            user_data_dir: None,
        }
    }
}

pub fn mac_e2e_overrides_enabled() -> bool {
    cfg!(debug_assertions) && std::env::var(E2E_FLAG).is_ok_and(|value| value == "1")
}

pub fn load_desktop_runtime_config() -> Result<DesktopRuntimeConfig, DesktopRuntimeConfigError> {
    if !mac_e2e_overrides_enabled() {
        return Ok(DesktopRuntimeConfig::default());
    }

    let user_port_raw = required_env(USER_PORT_ENV)?;
    let user_port = user_port_raw
        .parse::<u16>()
        .map_err(|_| DesktopRuntimeConfigError::InvalidUserPort(user_port_raw.clone()))?;

    let config = DesktopRuntimeConfig {
        network: required_env(NETWORK_ENV)?,
        primary_chain_url: required_env(CHAIN_URL_ENV)?,
        fallback_chain_url: required_env(FALLBACK_CHAIN_URL_ENV)?,
        lsp_pubkey: required_env(LSP_PUBKEY_ENV)?,
        lsp_address: required_env(LSP_ADDRESS_ENV)?,
        user_port,
        user_data_dir: optional_env(USER_DATA_DIR_ENV).map(PathBuf::from),
    };

    validate_e2e_config(&config)?;
    Ok(config)
}

fn required_env(key: &'static str) -> Result<String, DesktopRuntimeConfigError> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(DesktopRuntimeConfigError::MissingEnv(key))
}

fn optional_env(key: &'static str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn validate_e2e_config(config: &DesktopRuntimeConfig) -> Result<(), DesktopRuntimeConfigError> {
    if config.network != "regtest" {
        return Err(DesktopRuntimeConfigError::UnsafeE2EValue(
            NETWORK_ENV,
            config.network.clone(),
        ));
    }

    require_loopback(CHAIN_URL_ENV, &config.primary_chain_url)?;
    require_loopback(FALLBACK_CHAIN_URL_ENV, &config.fallback_chain_url)?;
    require_loopback(LSP_ADDRESS_ENV, &config.lsp_address)?;

    if config.lsp_pubkey == DEFAULT_LSP_PUBKEY {
        return Err(DesktopRuntimeConfigError::UnsafeE2EValue(
            LSP_PUBKEY_ENV,
            config.lsp_pubkey.clone(),
        ));
    }

    Ok(())
}

fn require_loopback(key: &'static str, value: &str) -> Result<(), DesktopRuntimeConfigError> {
    let lower = value.to_lowercase();
    let is_loopback = lower.contains("localhost")
        || lower.contains("127.0.0.1")
        || lower.contains("[::1]")
        || lower.starts_with("[::1]");

    if is_loopback {
        Ok(())
    } else {
        Err(DesktopRuntimeConfigError::UnsafeE2EValue(
            key,
            value.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    const TEST_LSP_PUBKEY: &str =
        "02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn clear_env() {
        for key in [
            E2E_FLAG,
            NETWORK_ENV,
            CHAIN_URL_ENV,
            FALLBACK_CHAIN_URL_ENV,
            LSP_PUBKEY_ENV,
            LSP_ADDRESS_ENV,
            USER_PORT_ENV,
            USER_DATA_DIR_ENV,
        ] {
            std::env::remove_var(key);
        }
    }

    fn set_valid_e2e_env() {
        std::env::set_var(E2E_FLAG, "1");
        std::env::set_var(NETWORK_ENV, "regtest");
        std::env::set_var(CHAIN_URL_ENV, "http://127.0.0.1:30000");
        std::env::set_var(FALLBACK_CHAIN_URL_ENV, "http://localhost:30000");
        std::env::set_var(LSP_PUBKEY_ENV, TEST_LSP_PUBKEY);
        std::env::set_var(LSP_ADDRESS_ENV, "127.0.0.1:9735");
        std::env::set_var(USER_PORT_ENV, "19736");
        std::env::set_var(USER_DATA_DIR_ENV, "/tmp/sc-mac-e2e");
    }

    #[test]
    fn defaults_ignore_mac_overrides_without_e2e_flag() {
        let _guard = env_lock().lock().unwrap();
        clear_env();
        std::env::set_var(NETWORK_ENV, "regtest");
        std::env::set_var(CHAIN_URL_ENV, "http://127.0.0.1:30000");

        let config = load_desktop_runtime_config().unwrap();

        assert_eq!(config.network, DEFAULT_NETWORK);
        assert_eq!(config.primary_chain_url, DEFAULT_CHAIN_URL);
        assert_eq!(config.lsp_address, DEFAULT_LSP_ADDRESS);
        clear_env();
    }

    #[test]
    fn loads_mac_regtest_overrides_when_e2e_enabled() {
        let _guard = env_lock().lock().unwrap();
        clear_env();
        set_valid_e2e_env();

        let config = load_desktop_runtime_config().unwrap();

        assert_eq!(config.network, "regtest");
        assert_eq!(config.primary_chain_url, "http://127.0.0.1:30000");
        assert_eq!(config.fallback_chain_url, "http://localhost:30000");
        assert_eq!(config.lsp_pubkey, TEST_LSP_PUBKEY);
        assert_eq!(config.lsp_address, "127.0.0.1:9735");
        assert_eq!(config.user_port, 19736);
        assert_eq!(config.user_data_dir, Some(PathBuf::from("/tmp/sc-mac-e2e")));
        clear_env();
    }

    #[test]
    fn rejects_production_chain_url_when_e2e_enabled() {
        let _guard = env_lock().lock().unwrap();
        clear_env();
        set_valid_e2e_env();
        std::env::set_var(CHAIN_URL_ENV, DEFAULT_CHAIN_URL);

        let err = load_desktop_runtime_config().unwrap_err();

        assert!(matches!(
            err,
            DesktopRuntimeConfigError::UnsafeE2EValue(CHAIN_URL_ENV, _)
        ));
        clear_env();
    }

    #[test]
    fn rejects_default_lsp_pubkey_when_e2e_enabled() {
        let _guard = env_lock().lock().unwrap();
        clear_env();
        set_valid_e2e_env();
        std::env::set_var(LSP_PUBKEY_ENV, DEFAULT_LSP_PUBKEY);

        let err = load_desktop_runtime_config().unwrap_err();

        assert!(matches!(
            err,
            DesktopRuntimeConfigError::UnsafeE2EValue(LSP_PUBKEY_ENV, _)
        ));
        clear_env();
    }
}
