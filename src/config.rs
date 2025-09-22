use std::env;
use std::path::PathBuf;
use dirs::data_dir;
use crate::constants::*;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub network: String,
    pub user_node_alias: String,
    pub user_port: u16,
    pub lsp_node_alias: String,
    pub lsp_port: u16,
    pub chain_source_url: String,
    pub expected_usd: f64,
    pub lsp_pubkey: String,
    pub gateway_pubkey: String,
    pub lsp_address: String,
    pub gateway_address: String,
    pub bitcoin_rpc_user: Option<String>,
    pub bitcoin_rpc_password: Option<String>,
}

impl AppConfig {
    pub fn load() -> Result<Self, String> {
        // Try to load .env file (ignore if not found)
        let _ = dotenvy::dotenv();
        
        Ok(AppConfig {
            network: env_var_or_default("STABLE_CHANNELS_NETWORK", DEFAULT_NETWORK),
            user_node_alias: env_var_or_default("STABLE_CHANNELS_USER_NODE_ALIAS", DEFAULT_USER_ALIAS),
            user_port: env_var_or_default_parse("STABLE_CHANNELS_USER_PORT", DEFAULT_USER_PORT),
            lsp_node_alias: env_var_or_default("STABLE_CHANNELS_LSP_NODE_ALIAS", DEFAULT_LSP_ALIAS),
            lsp_port: env_var_or_default_parse("STABLE_CHANNELS_LSP_PORT", DEFAULT_LSP_PORT),
            chain_source_url: env_var_or_default("STABLE_CHANNELS_CHAIN_SOURCE_URL", DEFAULT_CHAIN_URL),
            expected_usd: env_var_or_default_parse("STABLE_CHANNELS_EXPECTED_USD", DEFAULT_EXPECTED_USD),
            lsp_pubkey: env::var("STABLE_CHANNELS_LSP_PUBKEY").unwrap_or_else(|_| DEFAULT_LSP_PUBKEY.to_string()),
            gateway_pubkey: env::var("STABLE_CHANNELS_GATEWAY_PUBKEY").unwrap_or_else(|_| DEFAULT_GATEWAY_PUBKEY.to_string()),
            lsp_address: env::var("STABLE_CHANNELS_LSP_ADDRESS").unwrap_or_else(|_| DEFAULT_LSP_ADDRESS.to_string()),
            gateway_address: env::var("STABLE_CHANNELS_GATEWAY_ADDRESS").unwrap_or_else(|_| DEFAULT_GATEWAY_ADDRESS.to_string()),
            bitcoin_rpc_user: env::var("STABLE_CHANNELS_BITCOIN_RPC_USER").ok(),
            bitcoin_rpc_password: env::var("STABLE_CHANNELS_BITCOIN_RPC_PASSWORD").ok(),
        })
    }
    
    pub fn validate(&self) -> Result<(), Vec<String>> {
        // No validation needed since we now have smart defaults
        // All required fields will have values from constants if not set via environment
        Ok(())
    }
    
    pub fn get_user_data_dir(&self) -> PathBuf {
        data_dir()
            .expect("Could not determine user data dir")
            .join("StableChannels")
            .join(&self.user_node_alias)
    }
    
    pub fn get_lsp_data_dir(&self) -> PathBuf {
        data_dir()
            .expect("Could not determine LSP data dir")
            .join("StableChannels")
            .join(&self.lsp_node_alias)
    }
    
    pub fn get_audit_log_path(&self, mode: &str) -> String {
        let base_dir = match mode {
            "user" => self.get_user_data_dir(),
            "lsp" => self.get_lsp_data_dir(),
            _ => panic!("Invalid mode for audit log path"),
        };
        base_dir.join("audit_log.txt").to_string_lossy().into_owned()
    }
}

// Default constants
const DEFAULT_NETWORK: &str = "bitcoin";
const DEFAULT_USER_ALIAS: &str = "user";
const DEFAULT_USER_PORT: u16 = 9736;
const DEFAULT_LSP_ALIAS: &str = "lsp";
const DEFAULT_LSP_PORT: u16 = 9737;
const DEFAULT_CHAIN_URL: &str = "https://blockstream.info/api";
const DEFAULT_EXPECTED_USD: f64 = 100.0;

// Helper functions
fn env_var_or_default(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_var_or_default_parse<T>(key: &str, default: T) -> T 
where 
    T: std::str::FromStr + Copy,
    <T as std::str::FromStr>::Err: std::fmt::Debug,
{
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}