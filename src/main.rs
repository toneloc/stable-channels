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

#[cfg(feature = "user")]
mod user;

#[cfg(feature = "lsp")]
mod lsp;

#[cfg(feature = "exchange")]
mod exchange;

mod config;

use crate::config::Config;

use std::{io::{self, Write}, path::PathBuf};

use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Address, Network}, config::ChannelConfig, lightning::ln::msgs::SocketAddress, liquidity::LSPS2ServiceConfig, Builder, Node 
};

use state::StateManager;
use dirs_next::home_dir;

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
    #[cfg(feature = "user")]
    {
        println!("Starting in User mode");
        user::run();
    }
    
    #[cfg(feature = "lsp")]
    {
        println!("Starting in LSP mode");
        lsp::run();
    }

    #[cfg(feature = "exchange")]
    {
        println!("Starting in Exchange mode");
        exchange::run();
    }
    
    #[cfg(not(any(feature = "exchange", feature = "user", feature = "lsp")))]
    {
        println!("Error: No component selected.");
        println!("Please build with one of the following features:");
        println!("  --features exchange");
        println!("  --features user");
        println!("  --features lsp");
    }
}