//! TOML config schema for stable-channels-lsp.

use std::path::PathBuf;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub node: NodeConfig,
    pub storage: StorageConfig,
    pub ldk_server: LdkServerSection,
    #[serde(default)]
    pub push: Option<PushConfig>,
    #[serde(default)]
    pub tls: TlsSection,
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

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TlsSection {
    /// Extra hostnames/IPs added to the self-signed cert's SAN (localhost + 127.0.0.1 are always included).
    #[serde(default)]
    pub hosts: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct PushConfig {
    pub apns_key_path: Option<String>,
    pub apns_key_id: Option<String>,
    pub apns_team_id: Option<String>,
    pub apns_topic: Option<String>,
    /// "sandbox" or "production". Defaults to "sandbox" if absent.
    pub apns_environment: Option<String>,
    pub fcm_service_account_path: Option<String>,
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

    /// Resolve LDK Server's configured log file path. None if no config_path or no [log] file.
    pub fn resolve_ldk_log_file(&self) -> Option<PathBuf> {
        let path_str = self.ldk_server.config_path.as_deref()?;
        let raw = std::fs::read_to_string(path_str).ok()?;
        let parsed: toml::Value = toml::from_str(&raw).ok()?;
        let log_section = parsed.get("log")?.as_table()?;
        let file = log_section.get("file")?.as_str()?;
        Some(PathBuf::from(file))
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

    #[test]
    fn parses_push_section() {
        let toml_text = r#"
            [node]
            rest_service_address = "127.0.0.1:3002"
            network              = "regtest"

            [storage.disk]
            dir_path = "/tmp/sc-data"

            [ldk_server]
            config_path = "/etc/ldk-server/config.toml"

            [push]
            apns_key_path            = "/etc/sc/AuthKey.p8"
            apns_key_id              = "ABC123KEY1"
            apns_team_id             = "TEAM123456"
            apns_topic               = "com.stablechannels.app"
            apns_environment         = "sandbox"
            fcm_service_account_path = "/etc/sc/firebase.json"
        "#;
        let cfg: Config = toml::from_str(toml_text).unwrap();
        let push = cfg.push.expect("push section parsed");
        assert_eq!(push.apns_key_path.as_deref(), Some("/etc/sc/AuthKey.p8"));
        assert_eq!(push.apns_key_id.as_deref(), Some("ABC123KEY1"));
        assert_eq!(push.apns_topic.as_deref(), Some("com.stablechannels.app"));
        assert_eq!(push.apns_environment.as_deref(), Some("sandbox"));
        assert_eq!(push.fcm_service_account_path.as_deref(), Some("/etc/sc/firebase.json"));
    }

    #[test]
    fn push_section_is_optional() {
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
        assert!(cfg.push.is_none());
    }
}
