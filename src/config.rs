use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use ldk_node::Node;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct LspConfig {
    pub pubkey: String,
    pub address: String,
    pub auth: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct NodeConfig {
    pub network: String,
    pub chain_source_url: String,
    pub data_dir: String,
    pub alias: String,
    pub port: u16,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct StableChannelConfig {
    pub expected_usd: f64,
    pub sc_dir: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub lsp: LspConfig,
    pub node: NodeConfig,
    pub stable_channel_defaults: StableChannelConfig,
}

// Define ComponentType enum to replace string literals
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentType {
    User,
    Lsp,
    Exchange,
}

impl ComponentType {
    pub fn as_str(&self) -> &str {
        match self {
            ComponentType::User => "user",
            ComponentType::Lsp => "lsp",
            ComponentType::Exchange => "exchange"        }
    }
    
    pub fn default_port(&self) -> u16 {
        match self {
            ComponentType::User => 9736,
            ComponentType::Lsp => 9737,
            ComponentType::Exchange => 9738
        }
    }
    
    pub fn config_dir(&self) -> PathBuf {
        let mut path = PathBuf::from("data");
        path.push(self.as_str());
        path
    }
    
    pub fn config_path(&self) -> PathBuf {
        let mut path = self.config_dir();
        path.push("config.toml");
        path
    }
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self, Box<dyn Error>> {
        let content = fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn default() -> Self {
        Config {
            lsp: LspConfig {
                pubkey: "022814b30dc90b3c53312c250021165644fdf1650aa7ba4be5d6cd51302b2f31bb".to_string(),
                address: "127.0.0.1:9737".to_string(),
                auth: "00000000000000000000000000000000".to_string(),
            },
            node: NodeConfig {
                network: "signet".to_string(),
                chain_source_url: "https://mutinynet.com/api/".to_string(),
                data_dir: "data".to_string(),
                alias: "user".to_string(),
                port: 9736,
            },
            stable_channel_defaults: StableChannelConfig {
                expected_usd: 20.0,
                sc_dir: "data".to_string(),
            },
        }
    }
    
    // Get or create component-specific configuration
    pub fn get_or_create_for_component(component: ComponentType) -> Self {
        // First ensure the component directory exists
        let config_dir = component.config_dir();
        if !config_dir.exists() {
            println!("Creating component directory: {:?}", config_dir);
            fs::create_dir_all(&config_dir).unwrap_or_else(|e| {
                println!("Warning: Failed to create directory: {}", e);
            });
        }
        
        // Check if component-specific config exists
        let config_path = component.config_path();
        
        if !config_path.exists() {
            // If not, first check for global config to use as base
            let global_config = if Path::new("config.toml").exists() {
                match Config::from_file(Path::new("config.toml")) {
                    Ok(config) => config,
                    Err(_) => Config::default(),
                }
            } else {
                Config::default()
            };
            
            // Customize config for this component
            let component_config = global_config.for_component(component);
            
            // Save to component-specific config file
            if let Err(e) = component_config.save_to_file(&config_path) {
                println!("Warning: Failed to save component config: {}", e);
            }
            
            component_config
        } else {
            // Load component-specific config
            match Config::from_file(&config_path) {
                Ok(mut config) => {
                    // Ensure the config has component-specific settings
                    config.node.alias = component.as_str().to_string();
                    config.node.port = component.default_port();
                    config.node.data_dir = config_dir.to_string_lossy().to_string();
                    config.stable_channel_defaults.sc_dir = config_dir.to_string_lossy().to_string();
                    config
                },
                Err(e) => {
                    println!("Error loading component config: {}. Using defaults.", e);
                    Config::default().for_component(component)
                }
            }
        }
    }
    
    // Save config to file
    pub fn save_to_file(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        let config_str = toml::to_string_pretty(&self)?;
        fs::write(path, config_str)?;
        Ok(())
    }
    
    // Get component-specific configuration
    pub fn for_component(&self, component_type: ComponentType) -> Self {
        let mut config = self.clone();
        let component_name = component_type.as_str();
        let config_dir = component_type.config_dir();
        
        // Set unique data directory and port for each component
        config.node.data_dir = config_dir.to_string_lossy().to_string();
        config.node.alias = component_name.to_string();
        config.node.port = component_type.default_port();
        
        config.stable_channel_defaults.sc_dir = config_dir.to_string_lossy().to_string();
        
        config
    }
    
    // Ensure component data directories exist
    pub fn ensure_directories_exist(&self) -> Result<(), Box<dyn Error>> {
        // Create node data directory
        let data_dir = Path::new(&self.node.data_dir);
        if !data_dir.exists() {
            println!("Creating data directory: {:?}", data_dir);
            fs::create_dir_all(data_dir)?;
        }
        
        // Create stable channel data directory
        let sc_dir = Path::new(&self.stable_channel_defaults.sc_dir);
        if !sc_dir.exists() {
            println!("Creating stable channel data directory: {:?}", sc_dir);
            fs::create_dir_all(sc_dir)?;
        }
        
        Ok(())
    }
    
    // Print helpful connection info for this component
    pub fn print_connection_info(&self, node: &Node) {
        let node_id = node.node_id();
        
        println!("========== {} Node Connection Info ==========", self.node.alias.to_uppercase());
        println!("Node ID: {}", node_id);
        println!("Address: 127.0.0.1:{}", self.node.port);
        println!("");
        println!("To connect to this node, use the command:");
        println!("  openchannel {} 127.0.0.1:{} [SATS_AMOUNT]", node_id, self.node.port);
        println!("==============================================");
    }
}