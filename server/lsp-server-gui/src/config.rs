//! Configuration loading and saving - mostly used on native platforms only.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use hex::DisplayHex;
use serde::{Deserialize, Serialize};

/// GUI-specific config extracted from the SC daemon config file.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct GuiConfig {
	pub server_url: String,
	pub api_key: String,
	pub tls_cert_path: String,
	pub network: String,
	pub chain_source: ChainSourceConfig,
}

/// Map network string to the network-scoped directory name used by the SC daemon.
fn network_to_dir_name(network: &str) -> &str {
	match network {
		"bitcoin" | "mainnet" => "bitcoin",
		"testnet" => "testnet",
		"testnet4" => "testnet4",
		"signet" => "signet",
		"regtest" => "regtest",
		other => other,
	}
}

/// Load the API key from the generated file at {storage_dir}/{network}/api_key.
/// The server stores raw bytes; we return them hex-encoded.
fn load_api_key_from_file(storage_dir: &Path, network: &str) -> Option<String> {
	let network_dir = network_to_dir_name(network);
	let api_key_path = storage_dir.join(network_dir).join("api_key");
	std::fs::read(&api_key_path).ok().map(|bytes| bytes.to_lower_hex_string())
}

/// Chain source type for UI selection
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ChainSourceType {
	#[default]
	None,
	Bitcoind,
	Electrum,
	Esplora,
}

impl ChainSourceType {
	pub const ALL: [ChainSourceType; 3] =
		[ChainSourceType::Bitcoind, ChainSourceType::Electrum, ChainSourceType::Esplora];

	pub fn label(&self) -> &'static str {
		match self {
			ChainSourceType::None => "None",
			ChainSourceType::Bitcoind => "Bitcoin Core RPC",
			ChainSourceType::Electrum => "Electrum",
			ChainSourceType::Esplora => "Esplora",
		}
	}
}

/// Chain source configuration (Bitcoind RPC, Electrum, or Esplora)
#[derive(Debug, Clone, Default)]
pub enum ChainSourceConfig {
	#[default]
	None,
	Bitcoind {
		rpc_address: String,
		rpc_user: String,
		rpc_password: String,
	},
	Electrum {
		server_url: String,
	},
	Esplora {
		server_url: String,
	},
}

impl ChainSourceConfig {
	#[allow(dead_code)]
	pub fn source_type(&self) -> ChainSourceType {
		match self {
			ChainSourceConfig::None => ChainSourceType::None,
			ChainSourceConfig::Bitcoind { .. } => ChainSourceType::Bitcoind,
			ChainSourceConfig::Electrum { .. } => ChainSourceType::Electrum,
			ChainSourceConfig::Esplora { .. } => ChainSourceType::Esplora,
		}
	}
}

/// Partial deserialization of the SC daemon config file.
/// We only need the fields relevant to connecting as a client.
#[derive(Deserialize)]
struct TomlConfig {
	node: NodeConfig,
	storage: StorageConfig,
	bitcoind: Option<BitcoindConfig>,
	electrum: Option<ElectrumConfig>,
	esplora: Option<EsploraConfig>,
}

#[derive(Deserialize)]
struct NodeConfig {
	network: String,
	rest_service_address: String,
	// Note: api_key in config is ignored by the SC daemon.
	// The server generates its own key at {storage_dir}/{network}/api_key
}

#[derive(Deserialize)]
struct StorageConfig {
	disk: DiskConfig,
}

#[derive(Deserialize)]
struct DiskConfig {
	dir_path: String,
}

#[derive(Deserialize)]
struct BitcoindConfig {
	rpc_address: String,
	rpc_user: String,
	rpc_password: String,
}

#[derive(Deserialize)]
struct ElectrumConfig {
	server_url: String,
}

#[derive(Deserialize)]
struct EsploraConfig {
	server_url: String,
}

impl TryFrom<TomlConfig> for GuiConfig {
	type Error = String;

	fn try_from(toml: TomlConfig) -> Result<Self, Self::Error> {
		let storage_dir = PathBuf::from(&toml.storage.disk.dir_path);
		let tls_cert_path = storage_dir.join("tls.crt");

		// Load API key from the generated file (not from config - server ignores that field)
		let api_key = load_api_key_from_file(&storage_dir, &toml.node.network).unwrap_or_default();

		let chain_source = if let Some(btc) = toml.bitcoind {
			ChainSourceConfig::Bitcoind {
				rpc_address: btc.rpc_address,
				rpc_user: btc.rpc_user,
				rpc_password: btc.rpc_password,
			}
		} else if let Some(electrum) = toml.electrum {
			ChainSourceConfig::Electrum { server_url: electrum.server_url }
		} else if let Some(esplora) = toml.esplora {
			ChainSourceConfig::Esplora { server_url: esplora.server_url }
		} else {
			ChainSourceConfig::None
		};

		Ok(GuiConfig {
			server_url: toml.node.rest_service_address,
			api_key,
			tls_cert_path: tls_cert_path.to_string_lossy().to_string(),
			network: toml.node.network.clone(),
			chain_source,
		})
	}
}

/// Parse config from TOML string content.
pub fn parse_config_from_str(contents: &str) -> Result<GuiConfig, String> {
	let toml_config: TomlConfig =
		toml::from_str(contents).map_err(|e| format!("Failed to parse config: {}", e))?;

	GuiConfig::try_from(toml_config)
}

/// Try to load config from a file path.
pub fn load_config<P: AsRef<Path>>(path: P) -> Result<GuiConfig, String> {
	let contents = std::fs::read_to_string(path.as_ref())
		.map_err(|e| format!("Failed to read config file: {}", e))?;

	parse_config_from_str(&contents)
}

/// Search for the SC daemon's `sc-config.toml` in common locations and load it.
pub fn find_and_load_config() -> Option<GuiConfig> {
	let search_paths = [
		// Repo root (typical: `cargo run -p lsp-server-gui` from stable-channels/)
		PathBuf::from("sc-config.toml"),
		// SC daemon crate dir (if running from the workspace root)
		PathBuf::from("server/stable-channels-lsp/sc-config.toml"),
		// Sibling crate (if running from inside lsp-server-gui/)
		PathBuf::from("../stable-channels-lsp/sc-config.toml"),
		// Parent of repo root
		PathBuf::from("../sc-config.toml"),
	];

	for path in &search_paths {
		if path.exists() {
			if let Ok(config) = load_config(path) {
				return Some(config);
			}
		}
	}

	// Also check if there's an environment variable pointing to the config
	if let Ok(env_path) = std::env::var("SC_CONFIG") {
		let path = PathBuf::from(env_path);
		if path.exists() {
			if let Ok(config) = load_config(&path) {
				return Some(config);
			}
		}
	}

	None
}

/// Serializable chain source configs for saving to TOML
#[derive(Serialize)]
struct BitcoindConfigSave {
	rpc_address: String,
	rpc_user: String,
	rpc_password: String,
}

#[derive(Serialize)]
struct ElectrumConfigSave {
	server_url: String,
}

#[derive(Serialize)]
struct EsploraConfigSave {
	server_url: String,
}

/// Update the chain source in an existing config file.
/// Preserves all other config sections.
pub fn save_chain_source<P: AsRef<Path>>(
	path: P, chain_source: &ChainSourceConfig,
) -> Result<(), String> {
	// Read existing file
	let contents = std::fs::read_to_string(path.as_ref())
		.map_err(|e| format!("Failed to read config file: {}", e))?;

	// Parse as generic TOML value to preserve structure
	let mut doc: toml::Value =
		toml::from_str(&contents).map_err(|e| format!("Failed to parse config file: {}", e))?;

	let table = doc.as_table_mut().ok_or_else(|| "Config file root is not a table".to_string())?;

	// Remove existing chain source sections
	table.remove("bitcoind");
	table.remove("electrum");
	table.remove("esplora");

	// Add new chain source section
	match chain_source {
		ChainSourceConfig::None => {
			// No chain source - leave all removed
		},
		ChainSourceConfig::Bitcoind { rpc_address, rpc_user, rpc_password } => {
			let btc_config = BitcoindConfigSave {
				rpc_address: rpc_address.clone(),
				rpc_user: rpc_user.clone(),
				rpc_password: rpc_password.clone(),
			};
			let value = toml::Value::try_from(btc_config)
				.map_err(|e| format!("Failed to serialize bitcoind config: {}", e))?;
			table.insert("bitcoind".to_string(), value);
		},
		ChainSourceConfig::Electrum { server_url } => {
			let electrum_config = ElectrumConfigSave { server_url: server_url.clone() };
			let value = toml::Value::try_from(electrum_config)
				.map_err(|e| format!("Failed to serialize electrum config: {}", e))?;
			table.insert("electrum".to_string(), value);
		},
		ChainSourceConfig::Esplora { server_url } => {
			let esplora_config = EsploraConfigSave { server_url: server_url.clone() };
			let value = toml::Value::try_from(esplora_config)
				.map_err(|e| format!("Failed to serialize esplora config: {}", e))?;
			table.insert("esplora".to_string(), value);
		},
	}

	// Write back to file
	let output =
		toml::to_string_pretty(&doc).map_err(|e| format!("Failed to serialize config: {}", e))?;

	std::fs::write(path.as_ref(), output)
		.map_err(|e| format!("Failed to write config file: {}", e))?;

	Ok(())
}
