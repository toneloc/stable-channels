//! TOML config schema for stable-channels-lsp.

use std::path::PathBuf;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub node: NodeConfig,
    pub storage: StorageConfig,
    pub ldk_server: LdkServerSection,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NodeConfig {
    pub rest_service_address: String,
    pub network: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StorageConfig {
    pub disk: DiskConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DiskConfig {
    pub dir_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LdkServerSection {
    /// Path to LDK Server's own config.toml. We use its config helpers to resolve cert/api_key.
    pub config_path: Option<String>,
    /// Explicit override for LDK Server's gRPC address (takes priority over config_path).
    pub grpc_address: Option<String>,
    /// Explicit override for LDK Server's TLS cert file.
    pub cert_path: Option<String>,
    /// Explicit override for LDK Server's API key file.
    pub api_key_path: Option<String>,
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config '{}': {}", path.display(), e))?;
        toml::from_str(&contents)
            .map_err(|e| format!("Failed to parse config '{}': {}", path.display(), e))
    }

    /// SC daemon's TLS cert path (auto-generated if missing).
    pub fn local_tls_cert_path(&self) -> PathBuf {
        PathBuf::from(&self.storage.disk.dir_path).join("tls.crt")
    }

    /// SC daemon's TLS key path (auto-generated if missing).
    pub fn local_tls_key_path(&self) -> PathBuf {
        PathBuf::from(&self.storage.disk.dir_path).join("tls.key")
    }

    /// SC daemon's own API key file (network-scoped).
    pub fn local_api_key_path(&self) -> PathBuf {
        PathBuf::from(&self.storage.disk.dir_path)
            .join(&self.node.network)
            .join("api_key")
    }

    /// SC daemon's audit log file.
    pub fn audit_log_path(&self) -> PathBuf {
        PathBuf::from(&self.storage.disk.dir_path).join("audit_log.txt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_example() {
        let toml_text = r#"
            [node]
            rest_service_address = "127.0.0.1:3002"
            network              = "regtest"

            [storage.disk]
            dir_path = "/tmp/sc-data"

            [ldk_server]
            config_path = "/etc/ldk-server/config.toml"
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(cfg.node.rest_service_address, "127.0.0.1:3002");
        assert_eq!(cfg.node.network, "regtest");
        assert_eq!(cfg.storage.disk.dir_path, "/tmp/sc-data");
        assert_eq!(cfg.ldk_server.config_path.as_deref(), Some("/etc/ldk-server/config.toml"));
        assert_eq!(
            cfg.local_tls_cert_path(),
            PathBuf::from("/tmp/sc-data/tls.crt"),
        );
        assert_eq!(
            cfg.local_api_key_path(),
            PathBuf::from("/tmp/sc-data/regtest/api_key"),
        );
    }

    #[test]
    fn parses_with_explicit_overrides() {
        let toml_text = r#"
            [node]
            rest_service_address = "0.0.0.0:3002"
            network              = "bitcoin"

            [storage.disk]
            dir_path = "/var/lib/sc"

            [ldk_server]
            grpc_address    = "10.0.0.1:3536"
            cert_path       = "/etc/ldk/tls.crt"
            api_key_path    = "/etc/ldk/bitcoin/api_key"
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        assert!(cfg.ldk_server.config_path.is_none());
        assert_eq!(cfg.ldk_server.grpc_address.as_deref(), Some("10.0.0.1:3536"));
    }
}
