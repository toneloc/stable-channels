use std::{net::SocketAddr, str::FromStr};

use ldk_node::{config::ChannelConfig, lightning::offers::offer::Offer};
use stable_channels::{Bitcoin, StateManager};

use crate::{get_user_input, make_node};

pub fn run() {
    let lsp_node = make_node("lsp", 9737, None);
    let lsp = StateManager::new(lsp_node);
    let mut their_offer: Option<Offer> = None;

    loop {
        let (_input, command, args) = get_user_input("Enter command for lsp: ");

        match (command.as_deref(), args.as_slice()) {
            (Some("settheiroffer"), [their_offer_str]) => {
                match Offer::from_str(their_offer_str) {
                    Ok(offer) => {
                        their_offer = Some(offer);
                        println!("Offer set.");
                    }
                    Err(_) => println!("Error parsing offer"),
                }
            }
            (Some("getouroffer"), []) => {
                match lsp.node().bolt12_payment().receive_variable_amount("thanks", None) {
                    Ok(our_offer) => println!("{}", our_offer),
                    Err(e) => println!("Error creating offer: {}", e),
                }
            }
            (Some("openchannel"), args) if args.len() == 3 => {
                let node_id_str = &args[0];
                let listening_address_str = &args[1];
                let sats_str = &args[2];

                let user_node_id = match node_id_str.parse() {
                    Ok(id) => id,
                    Err(e) => {
                        println!("Failed to parse node ID: {}", e);
                        continue;
                    }
                };

                let lsp_net_address: SocketAddr = match listening_address_str.parse() {
                    Ok(addr) => addr,
                    Err(e) => {
                        println!("Failed to parse address: {}", e);
                        continue;
                    }
                };

                let sats: u64 = match sats_str.parse() {
                    Ok(s) => s,
                    Err(e) => {
                        println!("Failed to parse sats amount: {}", e);
                        continue;
                    }
                };

                let channel_config: Option<ChannelConfig> = None;

                match lsp.node().open_announced_channel(
                    user_node_id,
                    lsp_net_address.into(),
                    sats,
                    Some(sats / 2),
                    channel_config,
                ) {
                    Ok(_) => println!("Channel successfully opened to {}", node_id_str),
                    Err(e) => println!("Failed to open channel: {}", e),
                }
            }
            (Some("balance"), []) => {
                let balances = lsp.node().list_balances();
                let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                let lightning_balance = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                println!("LSP On-Chain Balance: {}", onchain_balance);
                println!("LSP Lightning Balance: {}", lightning_balance);
            }
            (Some("exit"), _) => break,
            _ => println!("Unknown command or incorrect arguments"),
        }
    }
}
