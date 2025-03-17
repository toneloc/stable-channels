/// Stable Channels in LDK 
/// Contents
/// Main data structure and helper types are in `types.rs`.
/// The price feed config and logic is in price_feeds.rs.
/// The state management is in state.rs.
/// This present file includes LDK set-up, program initialization,
/// a command-line interface, and the core stability logic.
/// We have three different services: exchange, user, and lsp

mod types;
mod price_feeds;
mod state;
mod lsp;
mod exchange;
mod user;
mod config;

use crate::config::Config;

use std::{io::{self, Write}, path::PathBuf};

use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Address, Network}, config::ChannelConfig, lightning::ln::msgs::SocketAddress, liquidity::LSPS2ServiceConfig, Builder, Node 
};

use state::StateManager;
use dirs_next::home_dir;

/// LDK set-up and initialization
fn make_node(config: &Config, lsp_pubkey: Option<PublicKey>, is_service: bool) -> Node {
    println!("Config used for make_node: {:?}", config);

    let mut builder = Builder::new();
    
    if let Some(lsp_pubkey) = lsp_pubkey {
        let address = config.lsp.address.parse().unwrap();
        println!("Setting LSP with address: {} and pubkey: {:?}", address, lsp_pubkey);
        builder.set_liquidity_source_lsps2(lsp_pubkey, address, Some(config.lsp.auth.clone()));
    }

    let network = match config.node.network.to_lowercase().as_str() {
        "signet" => Network::Signet,
        "testnet" => Network::Testnet,
        "bitcoin" => Network::Bitcoin,
        _ => Network::Signet,
    };
    println!("Network set to: {:?}", network);

    // If the node is offering a service, build this.
    if is_service {
        let service_config = LSPS2ServiceConfig {
            require_token: None,
            advertise_service: true,
            channel_opening_fee_ppm: 10_000,
            channel_over_provisioning_ppm: 100_000,
            min_channel_opening_fee_msat: 0,
            min_channel_lifetime: 100,
            max_client_to_self_delay: 1024,
            min_payment_size_msat: 0,
            max_payment_size_msat: 1_000_000_000,
        };
    
        builder.set_liquidity_provider_lsps2(service_config);
    }
    
    builder.set_network(network);
    builder.set_chain_source_esplora(config.node.chain_source_url.clone(), None);

    // FIX: Use the node.data_dir directly without appending anything
    let data_dir = &config.node.data_dir;
    println!("Storage directory: {:?}", data_dir);
    
    if !std::path::Path::new(data_dir).exists() {
        println!("Creating data directory: {:?}", data_dir);
        std::fs::create_dir_all(data_dir).unwrap_or_else(|e| {
            println!("WARNING: Failed to create data directory: {}. Error: {}", data_dir, e);
        });
    } else {
        println!("Data directory exists: {:?}", data_dir);
    }

    builder.set_storage_dir_path(data_dir.clone());

    builder
        .set_listening_addresses(vec![format!("127.0.0.1:{}", config.node.port)
        .parse()
        .unwrap()])
        .unwrap();

    builder.set_node_alias(config.node.alias.clone());

    let node = match builder.build() {
        Ok(node) => {
            println!("Node built successfully.");
            node
        }
        Err(e) => {
            panic!("Node build failed: {:?}", e);
        }
    };

    if let Err(e) = node.start() {
        panic!("Node start failed: {:?}", e);
    }
    
    println!("Node started with ID: {:?}", node.node_id());
    
    // Print connection info
    config.print_connection_info(&node);
    
    node
}
#[allow(dead_code)]
fn get_user_input(prompt: &str) -> (String, Option<String>, Vec<String>) {
    let mut input = String::new();
    print!("{}", prompt);
    io::stdout().flush().unwrap();
    io::stdin().read_line(&mut input).unwrap_or_default();

    let input = input.trim().to_string();

    let mut parts = input.split_whitespace();
    let command = parts.next().map(|s| s.to_string());
    let args: Vec<String> = parts.map(|s| s.to_string()).collect();

    (input, command, args)
}

/// Program initialization and command-line-interface
fn main() {
    println!("Features enabled:");
    
    #[cfg(feature = "exchange")]
    println!("- exchange");
    
    #[cfg(feature = "user")]
    println!("- user");
    
    #[cfg(feature = "gui")]
    println!("- gui");
    
    #[cfg(feature = "lsp")]
    println!("- lsp");

    #[allow(dead_code)]
    #[cfg(feature = "exchange")]
    exchange::run();

    #[allow(dead_code)]
    #[cfg(feature = "user")]
    {
        println!("Starting user module...");
        #[cfg(feature = "gui")]
        println!("GUI feature is enabled, should launch GUI app");
        
        #[cfg(not(feature = "gui"))]
        println!("GUI feature is NOT enabled, should launch CLI");
        
        user::run();
    }

    #[allow(dead_code)]
    #[cfg(feature = "lsp")]
    lsp::run();

}