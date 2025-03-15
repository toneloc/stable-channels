use std::{net::SocketAddr, str::FromStr};

use ldk_node::UserChannelId;
use ldk_node::{config::ChannelConfig, lightning::offers::offer::Offer};
use stable_channels::{Bitcoin, StateManager};

use crate::{get_user_input, make_node};

use crate::config::{ComponentType, Config};

pub fn run() {
    let config = Config::get_or_create_for_component(ComponentType::Lsp);
    
    // Ensure directories exist
    if let Err(e) = config.ensure_directories_exist() {
        println!("Warning: Failed to create directories: {}", e);
    }

    let lsp_node = make_node(&config, None, true);
    let lsp = StateManager::new(lsp_node);
    let mut their_offer: Option<Offer> = None;

    loop {
        let (_input, command, args) = get_user_input("Enter command for lsp: ");

        match (command.as_deref(), args.as_slice()) {
            (Some("getaddress"), []) => {
                let funding_address = lsp.node().onchain_payment().new_address();
                match funding_address {
                    Ok(fund_addr) => println!("LSP Funding Address: {}", fund_addr),
                    Err(e) => println!("Error getting funding address: {}", e),
                }
            }
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
            (Some("listallchannels"), []) => {
                println!("channels:");
                for channel in lsp.node().list_channels().iter() {
                    let channel_id = channel.channel_id;
                    println!("{}", channel_id);
                }
                println!("channel details:");
                let channels = lsp.node().list_channels();
                println!("{:#?}", channels);
            }
            (Some("closechannel"), [channel_id_str]) => {
                // Decode hex string into bytes
                let channel_id_bytes = match hex::decode(channel_id_str) {
                    Ok(b) if b.len() == 32 => b,
                    _ => {
                        eprintln!("Invalid channel ID format. Ensure it's a 32-byte hex string.");
                        return;
                    }
                };
            
                let mut found = false;
                for channel in lsp.node().list_channels().iter() {
                    // Convert stored channel_id to a comparable format
                    let stored_channel_id = channel.channel_id.0.to_vec(); // Ensure it is a Vec<u8> for comparison
            
                    if stored_channel_id == channel_id_bytes {
                        let counterparty_node_id = channel.counterparty_node_id;
                        let _ = lsp.node().close_channel(&channel.user_channel_id, counterparty_node_id);
                        println!("Closing channel with ID: {:?}", channel_id_str);
                        found = true;
                        break;
                    }
                }
            
                if !found {
                    eprintln!("Channel ID {} not found.", channel_id_str);
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
