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


#[cfg(feature = "user")]
mod config;

#[cfg(feature = "user")]
mod gui;

use std::io::{self, Write};

use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Network}, config::ChannelConfig, lightning::ln::msgs::SocketAddress, liquidity::LSPS2ServiceConfig, Builder 
};

use state::StateManager;

/// LDK set-up and initialization
fn make_node(alias: &str, port: u16, lsp_pubkey: Option<PublicKey>) -> ldk_node::Node {
    let mut builder = Builder::new();

    // If we pass in an LSP pubkey then set your liquidity source
    if let Some(lsp_pubkey) = lsp_pubkey {
        println!("{}", lsp_pubkey.to_string());
        let address: SocketAddress = "127.0.0.1:9377".parse().unwrap_or_else(|_| {
            eprintln!("Failed to parse default address, using fallback");
            "127.0.0.1:9737".parse().unwrap()
        });
        
        builder.set_liquidity_source_lsps2(
            lsp_pubkey,
            address,
            Some("00000000000000000000000000000000".to_owned()),
        );
    }

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

    // The old compare snippet used set_liquidity_provider_lsps2():
    builder.set_liquidity_provider_lsps2(service_config);

    builder.set_network(Network::Signet);

    // Configure chain source
    builder.set_chain_source_esplora("https://mutinynet.com/api/".to_string(), None);

    // Don't need gossip right now. Also interferes with Bolt12 implementation.
    builder.set_storage_dir_path(("./data/".to_owned() + alias).to_string());
    
    let _ = builder.set_listening_addresses(vec![format!("127.0.0.1:{}", port).parse().unwrap_or_else(|_| {
        eprintln!("Failed to parse listening address, using fallback");
        "127.0.0.1:9999".parse().unwrap()
    })]);
    
    let _ = builder.set_node_alias("some_alias".to_string());

    let node = match builder.build() {
        Ok(node) => node,
        Err(e) => {
            eprintln!("Failed to build node: {}", e);
            panic!("Node creation failed");
        }
    };
    
    if let Err(e) = node.start() {
        eprintln!("Failed to start node: {}", e);
        panic!("Node start failed");
    }
    
    let public_key: PublicKey = node.node_id();
    let listening_addresses: Vec<SocketAddress> = node.listening_addresses().unwrap_or_default();

    if let Some(first_address) = listening_addresses.first() {
        println!("");
        println!("Actor Role: {}", alias);
        println!("Public Key: {}", public_key);
        println!("Internet Address: {}", first_address);
        println!("");
    } else {
        println!("No listening addresses found.");
    } 

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
    #[allow(dead_code)]
    #[cfg(feature = "exchange")]
    exchange::run();

    #[allow(dead_code)]
    #[cfg(feature = "user")]
    gui::launch_app();

    #[allow(dead_code)]
    #[cfg(feature = "lsp")]
    lsp::run();

    // CLI user app - will only run if user feature is enabled AND egui app exits
    #[allow(dead_code)]
    #[cfg(all(feature = "user", not(any(feature = "gui"))))]
    {
        user::run();
    }

}