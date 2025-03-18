use std::{net::SocketAddr, str::FromStr};

use ldk_node::bitcoin::{Address, FeeRate, Network};
use ldk_node::lightning_invoice::Bolt11Invoice;
use ldk_node::{config::ChannelConfig, lightning::offers::offer::Offer};
use stable_channels::{Bitcoin, StateManager};

use crate::{get_user_input};

use crate::config::{ComponentType, Config};

use ldk_node::Node;

use ldk_node::{Builder};


#[cfg(feature = "lsp")]
fn make_lsp_node(config: &Config) -> Node {
    use ldk_node::{liquidity::LSPS2ServiceConfig, Node};

    println!("Initializing LSP node with config: {:?}", config);

    let mut builder = Builder::new();
    
    // Configure the network based on config
    let network = match config.node.network.to_lowercase().as_str() {
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
    
    // Configure LSPS2 service
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
    
    // Set up Esplora chain source
    println!("Setting Esplora API URL: {}", config.node.chain_source_url);
    builder.set_chain_source_esplora(config.node.chain_source_url.clone(), None);
    
    // Set up data directory
    let data_dir = &config.node.data_dir;
    println!("Setting storage directory: {}", data_dir);
    
    // Ensure the data directory exists
    if !std::path::Path::new(data_dir).exists() {
        println!("Creating data directory: {}", data_dir);
        std::fs::create_dir_all(data_dir).unwrap_or_else(|e| {
            println!("WARNING: Failed to create data directory: {}. Error: {}", data_dir, e);
        });
    }
    
    builder.set_storage_dir_path(data_dir.clone());
    
    // Set up listening address for the LSP node
    let listen_addr = format!("127.0.0.1:{}", config.node.port).parse().unwrap();
    println!("Setting listening address: {}", listen_addr);
    builder.set_listening_addresses(vec![listen_addr]).unwrap();
    
    // Set node alias
    builder.set_node_alias(config.node.alias.clone());
    
    // Build the node
    let node = match builder.build() {
        Ok(node) => {
            println!("LSP node built successfully");
            node
        },
        Err(e) => {
            panic!("Failed to build LSP node: {:?}", e);
        }
    };

    // Start the node
    if let Err(e) = node.start() {
        panic!("Failed to start LSP node: {:?}", e);
    }
    
    println!("LSP node started with ID: {}", node.node_id());
    
    // Print connection info
    config.print_connection_info(&node);
    
    // Final delay to ensure stability
    std::thread::sleep(std::time::Duration::from_millis(200));
    
    node
}


#[cfg(feature = "lsp")]
pub fn run() {
    let config = Config::get_or_create_for_component(ComponentType::Lsp);
    
    // Ensure directories exist
    if let Err(e) = config.ensure_directories_exist() {
        println!("Warning: Failed to create directories: {}", e);
    }

    let lsp_node = make_lsp_node(&config);
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
            (Some("getinvoice"), [sats]) => {
                if let Ok(sats_value) = sats.parse::<u64>() {
                    let msats = sats_value * 1000;
                    let bolt11 = lsp.node().bolt11_payment();
                    
                    // Create a proper invoice description
                    let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
                        ldk_node::lightning_invoice::Description::new("LSP Invoice".to_string()).unwrap_or_else(|_| {
                            println!("Failed to create description, using fallback");
                            ldk_node::lightning_invoice::Description::new("Fallback Invoice".to_string()).unwrap()
                        })
                    );
                    
                    match bolt11.receive(msats, &description, 6000) {
                        Ok(inv) => println!("LSP Invoice: {}", inv),
                        Err(e) => println!("Error creating invoice: {}", e),
                    }
                } else {
                    println!("Invalid sats value provided");
                }
            }
            (Some("closeallchannels"), []) => {
                for channel in lsp.node().list_channels().iter() {
                    let user_channel_id = channel.user_channel_id;
                    let counterparty_node_id = channel.counterparty_node_id;
                    let _ = lsp.node().close_channel(&user_channel_id, counterparty_node_id);
                }
                print!("Closing all channels.")
            }
            (Some("closechannel"), [channel_id]) => {
                if let Some(channel) = lsp.node().list_channels().iter().find(|c| format!("{:?}", c.user_channel_id) == *channel_id) {
                    let _ = lsp.node().close_channel(&channel.user_channel_id, channel.counterparty_node_id);
                    println!("Closing channel with ID: {:?}", channel.user_channel_id);
                } else {
                    println!("Channel with ID {:?} not found.", channel_id);
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
                            match lsp.node().onchain_payment().send_to_address(&addr_checked, amount_sats, fee_rate) {
                                Ok(txid) => println!("Transaction broadcasted successfully: {}", txid),
                                Err(e) => eprintln!("Error broadcasting transaction: {}", e),
                            }
                        }
                        Err(_) => eprintln!("Invalid address for this network."),
                    },
                    Err(_) => eprintln!("Invalid Bitcoin address."),
                }
            }
            (Some("payjitinvoice"), [invoice_str]) | (Some("payinvoice"), [invoice_str]) => {
                let bolt11_invoice = invoice_str.parse::<Bolt11Invoice>();
                match bolt11_invoice {
                    Ok(invoice) => match lsp.node().bolt11_payment().send(&invoice, None) {
                        Ok(payment_id) => {
                            println!("Payment sent from Exchange with payment_id: {}", payment_id)
                        }
                        Err(e) => println!("Error sending payment from Exchange: {}", e),
                    },
                    Err(e) => println!("Error parsing invoice: {}", e),
                }
            }
            (Some("exit"), _) => break,
            _ => println!("Unknown command or incorrect arguments"),
        }
    }
}


