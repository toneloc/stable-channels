use serde::Deserialize;
use std::error::Error;
use std::fs;

#[derive(Deserialize, Debug, Clone)]
pub struct LspConfig {
    pub pubkey: String,
    pub address: String,
    pub auth: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NodeConfig {
    pub network: String,
    pub chain_source_url: String,
    pub data_dir: String,
    pub alias: String,
    pub port: u16,
}

#[derive(Deserialize, Debug, Clone)]
pub struct StableChannelConfig {
    pub expected_usd: f64,
    pub sc_dir: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub lsp: LspConfig,
    pub node: NodeConfig,
    pub stable_channel_defaults: StableChannelConfig,
}

impl Config {
    pub fn from_file(path: &str) -> Result<Self, Box<dyn Error>> {
        let content = fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn default() -> Self {
        Config {
            lsp: LspConfig {
                pubkey: "02f66757a6204814d0996bf819a47024de6f18c3878e7797938d13a69a54d3791b".to_string(),
                address: "127.0.0.1:9737".to_string(),
                auth: "00000000000000000000000000000000".to_string(),
            },
            node: NodeConfig {
                network: "signet".to_string(),
                chain_source_url: "https://mutinynet.com/api/".to_string(),
                data_dir: "~/.stable-channels".to_string(),
                alias: "user".to_string(),
                port: 9736,
            },
            stable_channel_defaults: StableChannelConfig {
                expected_usd: 20.0,
                sc_dir: ".data".to_string(),
            },
        }
    }
}