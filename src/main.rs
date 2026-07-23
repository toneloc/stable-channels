// On Windows release builds, ship as a GUI app: no console window pops up
// alongside the eframe window. Debug builds keep the console so panics and
// `eprintln!` from `cargo run` remain visible during dev.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

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

pub mod audit;
pub mod constants;
pub mod price_feeds;
pub mod stable;
pub mod types;
pub mod user;

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "user".to_string());

    match mode.as_str() {
        "user" => user::run(),
        #[cfg(debug_assertions)]
        "mac-smoke" => run_mac_smoke(),
        #[cfg(debug_assertions)]
        "mac-flows" => {
            if let Err(err) = user::run_mac_flows() {
                eprintln!("Mac flows failed: {err}");
                std::process::exit(1);
            }
        }
        #[cfg(debug_assertions)]
        "mac-demo" => user::run(),
        // "lsp" | "exchange" => server::run_with_mode(&mode),
        _ => {
            eprintln!(
                "Unknown mode: '{}'. Use: `user`, `mac-smoke`, `mac-flows`, `mac-demo`, `lsp`, or `exchange`",
                mode
            );
            std::process::exit(1);
        }
    }
}

#[cfg(debug_assertions)]
fn run_mac_smoke() {
    let config = match stable_channels::desktop_config::load_desktop_runtime_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Mac smoke config failed: {err}");
            std::process::exit(1);
        }
    };

    if config.network != "regtest" {
        eprintln!("Mac smoke expected regtest, got {}", config.network);
        std::process::exit(1);
    }

    let feeds = stable_channels::constants::get_default_price_feeds();
    let feed_base = std::env::var("SC_PRICE_FEED_BASE").unwrap_or_default();
    if feed_base.is_empty()
        || !feeds
            .iter()
            .all(|feed| feed.url_format.starts_with(&feed_base))
    {
        eprintln!("Mac smoke price feeds are not using SC_PRICE_FEED_BASE");
        std::process::exit(1);
    }

    println!("Mac smoke config OK");
    println!("network={}", config.network);
    println!("chain={}", config.primary_chain_url);
    println!("fallback_chain={}", config.fallback_chain_url);
    println!("lsp_address={}", config.lsp_address);
    println!("user_port={}", config.user_port);
}
