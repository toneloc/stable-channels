use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use ldk_node::bitcoin::{Address, FeeRate, Network};
use ldk_node::lightning_invoice::Bolt11Invoice;
use ldk_node::{config::ChannelConfig, lightning::ln::msgs::SocketAddress};

use crate::types::StableChannel;
use crate::{get_user_input, types::Bitcoin};

use ldk_node::Node;
use ldk_node::{Builder};

// Configuration constants
const EXCHANGE_DATA_DIR: &str = "data/exchange";
const EXCHANGE_NODE_ALIAS: &str = "exchange";
const EXCHANGE_PORT: u16 = 9735;
const DEFAULT_NETWORK: &str = "signet";
const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mutinynet.com/api/";

struct ExchangeState {
    node: Node,
    stable_channel: StableChannel,
    last_check: SystemTime,
    initialized: bool,
}

#[cfg(feature = "exchange")]
fn make_exchange_node() -> Node {
    println!("Initializing exchange node...");

    let mut builder = Builder::new();
    
    // Configure the network based on config
    let network = match DEFAULT_NETWORK.to_lowercase().as_str() {
        "signet" => Network::Signet,
        "testnet" => Network::Testnet,
        "bitcoin" => Network::Bitcoin,
        _ => {
            println!("Warning: Unknown network in config, defaulting to Signet");
            Network::Signet
        }
    };
    
    println!("Setting network to: {:?}", network);
    builder.set_network(network);
    
    // Set up Esplora chain source
    println!("Setting Esplora API URL: {}", DEFAULT_CHAIN_SOURCE_URL);
    builder.set_chain_source_esplora(DEFAULT_CHAIN_SOURCE_URL.to_string(), None);
    
    // Set up data directory
    println!("Setting storage directory: {}", EXCHANGE_DATA_DIR);
    
    // Ensure the data directory exists
    if !std::path::Path::new(EXCHANGE_DATA_DIR).exists() {
        println!("Creating data directory: {}", EXCHANGE_DATA_DIR);
        std::fs::create_dir_all(EXCHANGE_DATA_DIR).unwrap_or_else(|e| {
            println!("WARNING: Failed to create data directory: {}. Error: {}", EXCHANGE_DATA_DIR, e);
        });
    }
    
    builder.set_storage_dir_path(EXCHANGE_DATA_DIR.to_string());
    
    // Set up listening address for the exchange node
    let listen_addr = format!("127.0.0.1:{}", EXCHANGE_PORT).parse().unwrap();
    println!("Setting listening address: {}", listen_addr);
    builder.set_listening_addresses(vec![listen_addr]).unwrap();
    
    // Set node alias
    builder.set_node_alias(EXCHANGE_NODE_ALIAS.to_string());
    
    // Build the node
    let node = match builder.build() {
        Ok(node) => {
            println!("Exchange node built successfully");
            node
        },
        Err(e) => {
            panic!("Failed to build exchange node: {:?}", e);
        }
    };
    
    // Start the node
    if let Err(e) = node.start() {
        panic!("Failed to start exchange node: {:?}", e);
    }
    
    println!("Exchange node started with ID: {}", node.node_id());
    println!("To connect to this node, use:");
    println!("  openchannel {} 127.0.0.1:{} [SATS_AMOUNT]", node.node_id(), EXCHANGE_PORT);
    
    node
}

#[cfg(feature = "exchange")]
pub fn run() {
    // Ensure exchange directory exists
    if !std::path::Path::new(EXCHANGE_DATA_DIR).exists() {
        std::fs::create_dir_all(EXCHANGE_DATA_DIR).unwrap_or_else(|e| {
            println!("Warning: Failed to create directories: {}", e);
        });
    }

    let exchange = make_exchange_node();
    
    let exchange_state = ExchangeState {
        node: exchange,
        stable_channel: StableChannel::default(),
        last_check: SystemTime::now(),
        initialized: false,
    };

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

                match exchange_state.node.open_announced_channel(
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
                let funding_address = exchange_state.node.onchain_payment().new_address();
                match funding_address {
                    Ok(fund_addr) => println!("Exchange Funding Address: {}", fund_addr),
                    Err(e) => println!("Error getting funding address: {}", e),
                }
            }
            (Some("balance"), []) => {
                let balances = exchange_state.node.list_balances();
                let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                let lightning_balance = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                println!("Exchange On-Chain Balance: {}", onchain_balance);
                println!("Exchange Lightning Balance: {}", lightning_balance);
            }
            (Some("listallchannels"), []) => {
                println!("Channels:");
                
                let channels = exchange_state.node.list_channels();
                
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
                    Ok(invoice) => match exchange_state.node.bolt11_payment().send(&invoice, None) {
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
                            match exchange_state.node.onchain_payment().send_to_address(&addr_checked, amount_sats, fee_rate) {
                                Ok(txid) => println!("Transaction broadcasted successfully: {}", txid),
                                Err(e) => eprintln!("Error broadcasting transaction: {}", e),
                            }
                        }
                        Err(_) => eprintln!("Invalid address for this network."),
                    },
                    Err(_) => eprintln!("Invalid Bitcoin address."),
                }
            }
            (Some("getinvoice"), [sats]) => {
                if let Ok(sats_value) = sats.parse::<u64>() {
                    let msats = sats_value * 1000;
                    let bolt11 = exchange_state.node.bolt11_payment();
                    
                    // Create a proper invoice description
                    let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
                        ldk_node::lightning_invoice::Description::new("Exchange Invoice".to_string()).unwrap_or_else(|_| {
                            println!("Failed to create description, using fallback");
                            ldk_node::lightning_invoice::Description::new("Fallback Invoice".to_string()).unwrap()
                        })
                    );
                    
                    match bolt11.receive(msats, &description, 6000) {
                        Ok(inv) => println!("Exchange Invoice: {}", inv),
                        Err(e) => println!("Error creating invoice: {}", e),
                    }
                } else {
                    println!("Invalid sats value provided");
                }
            }
            (Some("closeallchannels"), []) => {
                for channel in exchange_state.node.list_channels().iter() {
                    let user_channel_id = channel.user_channel_id;
                    let counterparty_node_id = channel.counterparty_node_id;
                    let _ = exchange_state.node.close_channel(&user_channel_id, counterparty_node_id);
                }
                println!("Closing all channels.")
            }
            (Some("exit"), _) => break,
            _ => println!("Unknown command or incorrect arguments: {}", input),
        }
    }
}