use std::str::FromStr;

use ldk_node::bitcoin::{Address, FeeRate, Network};
use ldk_node::lightning_invoice::Bolt11Invoice;
use ldk_node::{config::ChannelConfig, lightning::ln::msgs::SocketAddress};
use stable_channels::StateManager;

use crate::{get_user_input, make_node, types::Bitcoin};

use crate::config::{ComponentType, Config};


pub fn run() {
    let config = Config::get_or_create_for_component(ComponentType::Exchange);
    
    // Ensure directories exist
    if let Err(e) = config.ensure_directories_exist() {
        println!("Warning: Failed to create directories: {}", e);
    }

    let exchange_node = make_node(&config, None, false);
    let exchange = StateManager::new(exchange_node);
    
    loop {
        let (input, command, args) = get_user_input("Enter command for exchange: ");

        match (command.as_deref(), args.as_slice()) {
            (Some("openchannel"), args) if args.len() == 3 => {
                let node_id_str = &args[0];
                let listening_address_str = &args[1];
                let sats_str = &args[2];

                let lsp_node_id = match node_id_str.parse() {
                    Ok(id) => id,
                    Err(e) => {
                        println!("Failed to parse node ID: {}", e);
                        continue;
                    }
                };

                let lsp_net_address: SocketAddress = match listening_address_str.parse() {
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

                match exchange.node().open_announced_channel(
                    lsp_node_id,
                    lsp_net_address,
                    sats,
                    Some(sats / 2),
                    channel_config,
                ) {
                    Ok(_) => println!("Channel successfully opened to {}", node_id_str),
                    Err(e) => println!("Failed to open channel: {}", e),
                }
            }
            (Some("getaddress"), []) => {
                let funding_address = exchange.node().onchain_payment().new_address();
                match funding_address {
                    Ok(fund_addr) => println!("Exchange Funding Address: {}", fund_addr),
                    Err(e) => println!("Error getting funding address: {}", e),
                }
            }
            (Some("balance"), []) => {
                let balances = exchange.node().list_balances();
                let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                let lightning_balance = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                println!("Exchange On-Chain Balance: {}", onchain_balance);
                println!("Exchange Lightning Balance: {}", lightning_balance);
            }
            (Some("listallchannels"), []) => {
                println!("Channels:");
                
                let channels = exchange.node().list_channels();
                
                for channel in &channels {
                    println!("-----------------------------------");
                    println!("Channel ID: {}", channel.channel_id);
                    println!("Counterparty: {}", channel.counterparty_node_id);
                    println!("Amount (Sats): {}", channel.channel_value_sats);
                    println!("Ours (Msats): {}", channel.outbound_capacity_msat);
                    println!("Theirs (Msats): {}", channel.inbound_capacity_msat);
                    println!("Ready: {}", channel.is_channel_ready);
                    println!("-----------------------------------");
                }
            }
            (Some("payjitinvoice"), [invoice_str]) | (Some("payinvoice"), [invoice_str]) => {
                let bolt11_invoice = invoice_str.parse::<Bolt11Invoice>();
                match bolt11_invoice {
                    Ok(invoice) => match exchange.node().bolt11_payment().send(&invoice, None) {
                        Ok(payment_id) => {
                            println!("Payment sent from Exchange with payment_id: {}", payment_id)
                        }
                        Err(e) => println!("Error sending payment from Exchange: {}", e),
                    },
                    Err(e) => println!("Error parsing invoice: {}", e),
                }
            }
            (Some("onchainsend"), [address_str, amount_str, fee_rate_str]) => {
                let amount_sats = match amount_str.parse::<u64>() {
                    Ok(a) => a,
                    Err(_) => {
                        eprintln!("Invalid amount format. Please enter a valid integer.");
                        continue;
                    }
                };
            
                // Parse the fee rate if provided, otherwise use `None`
                let fee_rate = if fee_rate_str == "default" {
                    None
                } else {
                    match fee_rate_str.parse::<u32>() {
                        Ok(rate) => Some(FeeRate::from_sat_per_kwu(rate.into())), // Adjust as needed
                        Err(_) => {
                            eprintln!("Invalid fee rate format. Please enter a valid number or 'default'.");
                            continue;
                        }
                    }
                };
            
                match Address::from_str(address_str) {
                    Ok(addr) => match addr.require_network(Network::Signet) {
                        Ok(addr_checked) => {
                            match exchange.node().onchain_payment().send_to_address(&addr_checked, amount_sats, fee_rate) {
                                Ok(txid) => println!("Transaction broadcasted successfully: {}", txid),
                                Err(e) => eprintln!("Error broadcasting transaction: {}", e),
                            }
                        }
                        Err(_) => eprintln!("Invalid address for this network."),
                    },
                    Err(_) => eprintln!("Invalid Bitcoin address."),
                }
            }
            (Some("exit"), _) => break,
            _ => println!("Unknown command or incorrect arguments: {}", input),
        }
    }
}
