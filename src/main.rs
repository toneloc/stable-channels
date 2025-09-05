/// Stable Channels in LDK 
/// Contents
/// Main data structure and helper types are in `types.rs`.
/// The price feed config and logic is in price_feeds.rs.
/// User-facing (stability) code in user.rs
/// Server code in server.rs
/// This present file includes LDK set-up, program initialization,
/// a command-line interface, and the core stability logic.
/// We have three different services: exchange, user, and lsp

use std::env;

pub mod price_feeds;
pub mod types;
pub mod audit;
pub mod stable;
pub mod user;

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "user".to_string());

    match mode.as_str() {
        "user" => user::run(),
        // "lsp" | "exchange" => server::run_with_mode(&mode),
        _ => {
            eprintln!("Unknown mode: '{}'. Use: `user`, `lsp`, or `exchange`", mode);
            std::process::exit(1);
        }
    }
}