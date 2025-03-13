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
mod config;

#[cfg(feature = "user")]
mod gui;

use std::{
    io::{self, Write},
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Network}, 
    config::ChannelConfig, 
    lightning::{
        ln::msgs::SocketAddress,
        offers::offer::Offer,
    }, 
    lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescription, Description}, 
    payment::SendingParameters, 
    Builder, 
    ChannelDetails, 
    Node
};

use state::{StateManager, StabilityAction};
use ldk_node::lightning::ln::types::ChannelId;
use types::{Bitcoin, StableChannel, USD};

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

    return node;
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
    #[cfg(feature = "exchange")]
    {
        let exchange_node = make_node("exchange", 9735, None);
        let exchange = StateManager::new(exchange_node);

        loop {
            let (input, command, args) = get_user_input("Enter command for exchange: ");

            match (command.as_deref(), args.as_slice()) {
                (Some("openchannel"), args) => {
                    if args.len() != 3 {
                        println!("Error: 'openchannel' command requires three parameters: <node_id>, <listening_address>, and <sats>");
                        continue;
                    }

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
                    let lightning_balance =
                        Bitcoin::from_sats(balances.total_lightning_balance_sats);
                    println!("Exchange On-Chain Balance: {}", onchain_balance);
                    println!("Exchange Lightning Balance: {}", lightning_balance);
                }
                (Some("closeallchannels"), []) => {
                    for channel in exchange.node().list_channels().iter() {
                        let user_channel_id = channel.user_channel_id;
                        let counterparty_node_id = channel.counterparty_node_id;
                        let _ = exchange.node().close_channel(&user_channel_id, counterparty_node_id);
                    }
                    println!("Closing all channels.")
                }
                (Some("listallchannels"), []) => {
                    println!("channels:");
                    for channel in exchange.node().list_channels().iter() {
                        let channel_id = channel.channel_id;
                        println!("{}", channel_id);
                    }
                    println!("channel details:");
                    let channels = exchange.node().list_channels();
                    println!("{:#?}", channels);
                }
                (Some("getinvoice"), [sats]) => {
                    if let Ok(sats_value) = sats.parse::<u64>() {
                        let msats = sats_value * 1000;
                        let bolt11 = exchange.node().bolt11_payment();
                        let description = Bolt11InvoiceDescription::Direct(
                            Description::new("Invoice".to_string()).unwrap_or_else(|_| {
                                println!("Failed to create description, using fallback");
                                Description::new("Fallback Invoice".to_string()).unwrap()
                            })
                        );
                        
                        let invoice = bolt11.receive(msats, &description, 6000);
                        match invoice {
                            Ok(inv) => println!("Exchange Invoice: {}", inv),
                            Err(e) => println!("Error creating invoice: {}", e),
                        }
                    } else {
                        println!("Invalid sats value provided");
                    }
                }
                (Some("payjitinvoice"), [invoice_str]) | (Some("payinvoice"), [invoice_str]) => {
                    let bolt11_invoice = match invoice_str.parse::<Bolt11Invoice>() {
                        Ok(invoice) => invoice,
                        Err(e) => {
                            println!("Error parsing invoice: {}", e);
                            continue;
                        }
                    };
                    
                    match exchange.node().bolt11_payment().send(&bolt11_invoice, None) {
                        Ok(payment_id) => {
                            println!("Payment sent from Exchange with payment_id: {}", payment_id)
                        }
                        Err() => println!("Error sending payment from Exchange:"),
                    }
                }
                (Some("exit"), _) => break,
                _ => println!("Unknown command or incorrect arguments: {}", input),
            }
        }
    }

    #[cfg(feature = "user")]
    {
        // Launch egui application when in user mode
        gui::launch_app();
    }

    #[cfg(all(feature = "user", not(any(feature = "lsp", feature = "exchange"))))]
    {
        // This block is intentionally left empty as the GUI handles everything
    }

    // Original CLI user app - will only run if user feature is enabled AND egui app exits
    #[cfg(all(feature = "user", not(any(feature = "gui"))))]
    {
        let user_node = make_node("user", 9736, None);
        let user = StateManager::new(user_node);
        let mut their_offer: Option<Offer> = None;

        loop {
            let (_input, command, args) = get_user_input("Enter command for user: ");

            match (command.as_deref(), args.as_slice()) {
                (Some("settheiroffer"), [their_offer_str]) => {
                    match Offer::from_str(their_offer_str) {
                        Ok(offer) => {
                            their_offer = Some(offer);
                            println!("Offer set.")
                        },
                        Err(e) => println!("Error parsing offer"),
                    }
                }
                (Some("getouroffer"), []) => {
                    match user.node().bolt12_payment().receive_variable_amount("thanks", None) {
                        Ok(our_offer) => println!("{}", our_offer),
                        Err(e) => println!("Error creating offer"),
                    }
                }
                (Some("checkstability"), []) => {
                    let action = user.check_stability();
                    match action {
                        StabilityAction::Pay(amount) => {
                            println!("Action: Pay {} msats", amount);
                            match user.execute_payment(amount) {
                                Ok(payment_id) => println!("Payment sent with ID: {}", payment_id),
                                Err(e) => println!("Failed to send payment: {}", e),
                            }
                        },
                        StabilityAction::Wait => println!("Action: Wait for counterparty payment"),
                        StabilityAction::DoNothing => println!("Action: Do nothing, channel is stable"),
                        StabilityAction::HighRisk(risk) => println!("Action: High risk level ({})", risk),
                        StabilityAction::NotInitialized => {
                            println!("Channel not properly initialized or may have been closed. Exiting stability loop.");
                            break; // Exit the loop if the channel is not initialized
                        }
                    }
                }
                // Sample start command below:
                // startstablechannel CHANNEL_ID IS_STABLE_RECEIVER EXPECTED_DOLLAR_AMOUNT EXPECTED_BTC_AMOUNT
                // startstablechannel 44c105c0f12c47ef4f573928448fb1c662fd61289b0baf93537f03075aa99010 true 305.0 0
                (Some("startstablechannel"), [channel_id, is_stable_receiver, expected_dollar_amount, native_amount_sats]) =>
                {
                    let channel_id = channel_id.to_string();
                    let is_stable_receiver = match is_stable_receiver.parse::<bool>() {
                        Ok(val) => val,
                        Err(_) => {
                            println!("Error: is_stable_receiver must be 'true' or 'false'");
                            continue;
                        }
                    };
                    
                    let expected_dollar_amount = match expected_dollar_amount.parse::<f64>() {
                        Ok(val) => val,
                        Err(_) => {
                            println!("Error: expected_dollar_amount must be a valid number");
                            continue;
                        }
                    };
                    
                    let native_amount_sats = match native_amount_sats.parse::<f64>() {
                        Ok(val) => val,
                        Err(_) => {
                            println!("Error: native_amount_sats must be a valid number");
                            continue;
                        }
                    };

                    // Initialize the stable channel using our state manager
                    match user.initialize_stable_channel(
                        &channel_id, 
                        is_stable_receiver, 
                        expected_dollar_amount, 
                        native_amount_sats
                    ) {
                        Ok(()) => {
                            println!("Stable Channel initialized: {}", channel_id);
                            
                            // Start the stability checking loop
                            use crate::price_feeds::{calculate_median_price, fetch_prices, set_price_feeds};
                            use crate::types::{Bitcoin, StableChannel, USD};
                            use ldk_node::{
                                bitcoin::secp256k1::PublicKey,
                                lightning::ln::types::ChannelId,
                                ChannelDetails, Node,
                            };
                            use std::sync::{Arc, Mutex};
                            use std::time::{SystemTime, UNIX_EPOCH, Duration};
                            use ureq::Agent;
                            
                            /// Represents the action to take after a stability check
                            #[derive(Debug, Clone)]
                            pub enum StabilityAction {
                                /// No action needed, channel is stable enough
                                DoNothing,
                                /// Wait for payment from counterparty
                                Wait,
                                /// Make a payment to maintain stability
                                Pay(u64), // amount in msats
                                /// High risk situation detected
                                HighRisk(u32), // risk level
                                /// Channel not properly initialized or not found
                                NotInitialized,
                            }
                            
                            /// Main state management for Stable Channels
                            pub struct StateManager {
                                /// LDK Node instance
                                node: Arc<Node>,
                                /// Current stable channel state
                                stable_channel: Arc<Mutex<StableChannel>>,
                                /// HTTP agent for price fetching
                                agent: Agent,
                                /// Last timestamp of stability check
                                last_check: Arc<Mutex<SystemTime>>,
                                /// Whether the channel has been properly initialized
                                initialized: Arc<Mutex<bool>>,
                            }
                            
                            impl StateManager {
                                /// Create a new state manager with the given node
                                pub fn new(node: Node) -> Self {
                                    Self {
                                        node: Arc::new(node),
                                        stable_channel: Arc::new(Mutex::new(StableChannel::default())),
                                        agent: Agent::new(),
                                        last_check: Arc::new(Mutex::new(SystemTime::now())),
                                        initialized: Arc::new(Mutex::new(false)),
                                    }
                                }
                            
                                /// Get a reference to the node
                                pub fn node(&self) -> &Node {
                                    &self.node
                                }
                            
                                /// Check if the state manager has been properly initialized with a valid channel
                                pub fn is_initialized(&self) -> bool {
                                    *self.initialized.lock().unwrap()
                                }
                            
                                /// Get the current stable channel state
                                pub fn get_stable_channel(&self) -> StableChannel {
                                    self.stable_channel.lock().unwrap().clone()
                                }
                            
                                /// Initialize a stable channel with the given parameters
                                pub fn initialize_stable_channel(
                                    &self,
                                    channel_id_str: &str,
                                    is_stable_receiver: bool,
                                    expected_dollar_amount: f64,
                                    native_amount_sats: f64,
                                ) -> Result<(), Box<dyn std::error::Error>> {
                                    let mut sc = self.stable_channel.lock().unwrap();
                                    
                                    // Check if the channel_id is provided as hex string or full channel id
                                    let channel_id = if channel_id_str.len() == 64 { // It's a hex string
                                        let channel_id_bytes: [u8; 32] = hex::decode(channel_id_str)?
                                            .try_into()
                                            .map_err(|_| "Decoded channel ID has incorrect length")?;
                                        ChannelId::from_bytes(channel_id_bytes)
                                    } else { // It's already a formatted channel id
                                        ChannelId::from_str(channel_id_str)?
                                    };
                            
                                    // Find the counterparty node ID from the channel list
                                    let mut counterparty = None;
                                    for channel in self.node.list_channels() {
                                        if channel.channel_id.to_string() == channel_id.to_string() {
                                            counterparty = Some(channel.counterparty_node_id);
                                            break;
                                        }
                                    }
                            
                                    let counterparty = counterparty.ok_or("Failed to find channel with the specified ID")?;
                            
                                    // Update the stable channel state
                                    sc.channel_id = channel_id;
                                    sc.is_stable_receiver = is_stable_receiver;
                                    sc.counterparty = counterparty;
                                    sc.expected_usd = USD::from_f64(expected_dollar_amount);
                                    sc.expected_btc = Bitcoin::from_btc(native_amount_sats);
                                    
                                    // Get initial price
                                    let latest_price = self.get_latest_price();
                                    sc.latest_price = latest_price;
                            
                                    // Update balances
                                    drop(sc); // Release lock before calling update_balances
                                    self.update_balances();
                            
                                    // Mark as initialized only if the channel exists and balances were updated
                                    let mut initialized = self.initialized.lock().unwrap();
                                    *initialized = true;
                            
                                    Ok(())
                                }
                            
                                /// Fetch the latest BTC/USD price
                                pub fn get_latest_price(&self) -> f64 {
                                    match fetch_prices(&self.agent, &set_price_feeds())
                                        .and_then(|prices| calculate_median_price(prices)) {
                                        Ok(price) => price,
                                        Err(e) => {
                                            eprintln!("Error fetching price: {:?}", e);
                                            60000.0 // Default fallback price
                                        }
                                    }
                                }
                                
                                /// Check if the given channel exists in the node's channel list
                                fn channel_exists(&self, channel_id: &ChannelId) -> bool {
                                    let channels = self.node.list_channels();
                                    channels.iter().any(|c| c.channel_id == *channel_id)
                                }
                                
                                /// Update stable channel balances based on current channel state
                                pub fn update_balances(&self) -> bool {
                                    let mut sc = self.stable_channel.lock().unwrap();
                                    
                                    // Find the matching channel
                                    let mut matching_channel_found = false;
                                    
                                    // Get current price if we don't have it
                                    if sc.latest_price == 0.0 {
                                        sc.latest_price = self.get_latest_price();
                                    }
                                    
                                    // First check if we're using the default channel ID (all zeros)
                                    let is_default = sc.channel_id == ChannelId::from_bytes([0; 32]);
                                    
                                    // If it's a default channel ID, try to find any available channel
                                    if is_default {
                                        if let Some(channel) = self.node.list_channels().first() {
                                            sc.channel_id = channel.channel_id;
                                            println!("Set active channel ID to: {}", sc.channel_id);
                                            matching_channel_found = true;
                                            
                                            // Update the channel balances
                                            let (our_balance, their_balance) = {
                                                let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
                                                let our_balance_sats =
                                                    (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
                                                let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                                                (our_balance_sats, their_balance_sats)
                                            };
                                            
                                            // Update balances based on whether we're the stable receiver or provider
                                            if sc.is_stable_receiver {
                                                sc.stable_receiver_btc = Bitcoin::from_sats(our_balance);
                                                sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                                                sc.stable_provider_btc = Bitcoin::from_sats(their_balance);
                                                sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                                            } else {
                                                sc.stable_provider_btc = Bitcoin::from_sats(our_balance);
                                                sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                                                sc.stable_receiver_btc = Bitcoin::from_sats(their_balance);
                                                sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                                            }
                                        }
                                    } else {
                                        // Otherwise, look for a channel matching our stored ID
                                        for channel in self.node.list_channels() {
                                            if channel.channel_id == sc.channel_id {
                                                matching_channel_found = true;
                                                
                                                // Update the channel balances
                                                let (our_balance, their_balance) = {
                                                    let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
                                                    let our_balance_sats =
                                                        (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
                                                    let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                                                    (our_balance_sats, their_balance_sats)
                                                };
                                                
                                                // Update balances based on whether we're the stable receiver or provider
                                                if sc.is_stable_receiver {
                                                    sc.stable_receiver_btc = Bitcoin::from_sats(our_balance);
                                                    sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                                                    sc.stable_provider_btc = Bitcoin::from_sats(their_balance);
                                                    sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                                                } else {
                                                    sc.stable_provider_btc = Bitcoin::from_sats(our_balance);
                                                    sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                                                    sc.stable_receiver_btc = Bitcoin::from_sats(their_balance);
                                                    sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                                                }
                                                
                                                break;
                                            }
                                        }
                                    }
                                    
                                    if !matching_channel_found {
                                        println!("No matching channel found for ID: {}", sc.channel_id);
                                        // If we can't find the channel, update initialization status
                                        if !is_default {
                                            println!("Channel may have been closed or not properly initialized.");
                                            if *self.initialized.lock().unwrap() {
                                                *self.initialized.lock().unwrap() = false;
                                            }
                                        }
                                    }
                                    
                                    // Update last check time
                                    *self.last_check.lock().unwrap() = SystemTime::now();
                                    
                                    matching_channel_found
                                }
                                
                                /// Check if the stable channel is in balance and determine what action to take
                                pub fn check_stability(&self) -> StabilityAction {
                                    // First, check if we're initialized with a valid channel
                                    let is_initialized = *self.initialized.lock().unwrap();
                                    if !is_initialized {
                                        let default_channel_id = ChannelId::from_bytes([0; 32]);
                                        
                                        // Get the current stable channel
                                        let sc = self.stable_channel.lock().unwrap();
                                        
                                        // If we have a non-default channel ID but it's not found in the channels list
                                        if sc.channel_id != default_channel_id && !self.channel_exists(&sc.channel_id) {
                                            println!("Stable channel with ID {} not found or not properly initialized", sc.channel_id);
                                            return StabilityAction::NotInitialized;
                                        }
                                        
                                        // If we have an default channel ID and there are no channels
                                        if sc.channel_id == default_channel_id && self.node.list_channels().is_empty() {
                                            println!("No channels available. Please create a channel first.");
                                            return StabilityAction::NotInitialized;
                                        }
                                        
                                        // If we have a default channel ID but channels exist, we might be able to use one
                                        if sc.channel_id == default_channel_id && !self.node.list_channels().is_empty() {
                                            // Try to update balances and automatically select an available channel
                                            let balances_updated = self.update_balances();
                                            if balances_updated {
                                                // If we successfully found a channel, mark as initialized
                                                *self.initialized.lock().unwrap() = true;
                                            } else {
                                                println!("Failed to initialize with available channels.");
                                                return StabilityAction::NotInitialized;
                                            }
                                        }
                                    }
                                    
                                    // Update the price and balances
                                    let price = self.get_latest_price();
                                    
                                    {
                                        let mut sc = self.stable_channel.lock().unwrap();
                                        sc.latest_price = price;
                                    }
                                    
                                    // Try to update balances, return NotInitialized if failed
                                    if !self.update_balances() {
                                        return StabilityAction::NotInitialized;
                                    }
                                    
                                    let sc = self.stable_channel.lock().unwrap();
                                    
                                    // Print the current state
                                    println!("{:<25} ${:>15.2}", "BTC/USD Price:", sc.latest_price);
                                    println!("{:<25} {:>15}", "Expected USD:", sc.expected_usd);
                                    println!("{:<25} {:>15}", "User USD:", sc.stable_receiver_usd);
                                    
                                    // Check for division by zero - if expected_usd is 0, we can't calculate difference
                                    if sc.expected_usd.0 == 0.0 {
                                        println!("Expected USD amount is zero. Cannot calculate stability difference.");
                                        return StabilityAction::NotInitialized;
                                    }
                                    
                                    // Calculate difference from expected value
                                    let dollars_from_par: USD = sc.stable_receiver_usd - sc.expected_usd;
                                    let percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();
                                    
                                    println!("{:<25} {:>5}", "Percent from par:", format!("{:.2}%", percent_from_par));
                                    println!("{:<25} {:>15}", "User BTC:", sc.stable_receiver_btc);
                                    println!("{:<25} {:>15}", "LSP USD:", sc.stable_provider_usd);
                                    println!("{:<25} {:>15}", "LSP BTC:", sc.stable_provider_btc);
                                    
                                    // Determine action based on conditions
                                    if percent_from_par < 0.1 {
                                        println!("\nDifference from par less than 0.1%. Doing nothing.");
                                        return StabilityAction::DoNothing;
                                    } 
                                    
                                    let is_receiver_below_expected = sc.stable_receiver_usd < sc.expected_usd;
                            
                                    match (sc.is_stable_receiver, is_receiver_below_expected, sc.risk_level > 100) {
                                        (_, _, true) => {
                                            println!("Risk level high. Current risk level: {}", sc.risk_level);
                                            StabilityAction::HighRisk(sc.risk_level as u32)
                                        },
                                        (true, true, false) => {
                                            println!("\nWaiting for payment from counterparty...");
                                            StabilityAction::Wait
                                        },
                                        (true, false, false) => {
                                            println!("\nPaying the difference...");
                                            let amt = USD::to_msats(dollars_from_par, sc.latest_price);
                                            StabilityAction::Pay(amt)
                                        },
                                        (false, true, false) => {
                                            println!("\nPaying the difference...");
                                            let amt = USD::to_msats(dollars_from_par, sc.latest_price);
                                            StabilityAction::Pay(amt)
                                        },
                                        (false, false, false) => {
                                            println!("\nWaiting for payment from counterparty...");
                                            StabilityAction::Wait
                                        },
                                    }
                                }
                                
                                /// Execute a payment to maintain stability
                                pub fn execute_payment(&self, amount_msats: u64) -> Result<String, Box<dyn std::error::Error>> {
                                    // First check if we're initialized
                                    if !*self.initialized.lock().unwrap() {
                                        return Err("Stable channel not initialized".into());
                                    }
                                    
                                    let sc = self.stable_channel.lock().unwrap();
                                    
                                    // Verify the counterparty exists
                                    if !self.node.list_channels().iter().any(|c| c.counterparty_node_id == sc.counterparty) {
                                        return Err("Counterparty not found in available channels".into());
                                    }
                                    
                                    let result = self.node
                                        .spontaneous_payment()
                                        .send(amount_msats, sc.counterparty, None)?;
                                        
                                    Ok(result.to_string())
                                }
                                
                                /// Get the time elapsed since the last stability check
                                pub fn time_since_last_check(&self) -> Duration {
                                    SystemTime::now()
                                        .duration_since(*self.last_check.lock().unwrap())
                                        .unwrap_or(Duration::from_secs(0))
                                }
                            }
                            
                            /// Extension trait to add ChannelId::from_str
                            trait ChannelIdExt {
                                fn from_str(s: &str) -> Result<ChannelId, Box<dyn std::error::Error>>;
                            }
                            
                            impl ChannelIdExt for ChannelId {
                                fn from_str(s: &str) -> Result<ChannelId, Box<dyn std::error::Error>> {
                                    // Simplified parsing - may need to be expanded based on the actual string format
                                    let clean_str = s.trim();
                                    
                                    if clean_str.len() >= 64 {
                                        // It's likely a hex string
                                        let hex_part = if clean_str.len() > 64 {
                                            // Extract just the 64 hex chars if there's extra formatting
                                            let start = clean_str.find(|c: char| c.is_ascii_hexdigit())
                                                .ok_or("No hex digits found in channel ID string")?;
                                            &clean_str[start..(start+64)]
                                        } else {
                                            clean_str
                                        };
                                        
                                        let bytes = hex::decode(hex_part)?;
                                        if bytes.len() != 32 {
                                            return Err(format!("Expected 32 bytes, got {}", bytes.len()).into());
                                        }
                                        
                                        let mut arr = [0u8; 32];
                                        arr.copy_from_slice(&bytes);
                                        Ok(ChannelId::from_bytes(arr))
                                    } else {
                                        Err("Channel ID string is too short".into())
                                    }
                                }
                            }
                        },
                        Err(e) => println!("Failed to initialize stable channel: {}", e),
                    }
                }
                (Some("getaddress"), []) => {
                    let funding_address = user.node().onchain_payment().new_address();
                    match funding_address {
                        Ok(fund_addr) => println!("User Funding Address: {}", fund_addr),
                        Err(e) => println!("Error getting funding address: {}", e),
                    }
                }
                (Some("openchannel"), args) => {
                    if args.len() != 3 {
                        println!("Error: 'openchannel' command requires three parameters: <node_id>, <listening_address>, and <sats>");
                        continue;
                    }

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
                    
                    let push_msat = (sats / 2) * 1000;
                    let channel_config: Option<ChannelConfig> = None;

                    match user.node().open_announced_channel(
                        lsp_node_id,
                        lsp_net_address,
                        sats,
                        Some(push_msat),
                        channel_config,
                    ) {
                        Ok(_) => println!("Channel successfully opened to {}", node_id_str),
                        Err(e) => println!("Failed to open channel: {}", e),
                    }
                }
                (Some("balance"), []) => {
                    let balances = user.node().list_balances();
                    let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                    let lightning_balance =
                        Bitcoin::from_sats(balances.total_lightning_balance_sats);
                    
                    // Get stable channel info if initialized
                    let sc = user.get_stable_channel();
                    if sc.latest_price > 0.0 {
                        println!("User On-Chain Balance: {}", onchain_balance);
                        println!("User Lightning Balance: {}", lightning_balance);
                        println!("Current BTC/USD Price: ${:.2}", sc.latest_price);
                        
                        // Print stable channel balances
                        if sc.is_stable_receiver {
                            // User is the receiver
                            println!("User Receiver Balance: {} (${:.2})",
                                sc.stable_receiver_btc,
                                sc.stable_receiver_usd.0);
                            println!("LSP Provider Balance: {} (${:.2})",
                                sc.stable_provider_btc,
                                sc.stable_provider_usd.0);
                        } else {
                            // User is the provider
                            println!("User Provider Balance: {} (${:.2})",
                                sc.stable_provider_btc,
                                sc.stable_provider_usd.0);
                            println!("LSP Receiver Balance: {} (${:.2})",
                                sc.stable_receiver_btc,
                                sc.stable_receiver_usd.0);
                        }
                    } else {
                        println!("User On-Chain Balance: {}", onchain_balance);
                        println!("User Lightning Balance: {}", lightning_balance);
                    }
                }
                (Some("closeallchannels"), []) => {
                    for channel in user.node().list_channels().iter() {
                        let user_channel_id = channel.user_channel_id;
                        let counterparty_node_id = channel.counterparty_node_id;
                        let _ = user.node().close_channel(&user_channel_id, counterparty_node_id);
                    }
                    println!("Closing all channels.")
                }
                (Some("listallchannels"), []) => {
                    let channels = user.node().list_channels();
                    if channels.is_empty() {
                        println!("No channels found.");
                    } else {
                        println!("User Channels:");
                        for channel in channels.iter() {
                            println!("--------------------------------------------");
                            println!("Channel ID: {}", channel.channel_id);
                            println!(
                                "Channel Value: {}",
                                Bitcoin::from_sats(channel.channel_value_sats)
                            );
                            println!("Channel Ready?: {}", channel.is_channel_ready);
                        }
                        println!("--------------------------------------------");
                    }
                }
                (Some("getinvoice"), [sats]) => {
                    if let Ok(sats_value) = sats.parse::<u64>() {
                        let msats = sats_value * 1000;
                        let bolt11 = user.node().bolt11_payment();
                        let description = Bolt11InvoiceDescription::Direct(
                            Description::new("Invoice".to_string()).unwrap_or_else(|_| {
                                println!("Failed to create description, using fallback");
                                Description::new("Fallback Invoice".to_string()).unwrap()
                            })
                        );
                        
                        match bolt11.receive(msats, &description, 6000) {
                            Ok(inv) => println!("User Invoice: {}", inv),
                            Err(e) => println!("Error creating invoice: {}", e),
                        }
                    } else {
                        println!("Invalid sats value provided");
                    }
                }
                (Some("payinvoice"), [invoice_str]) => {
                    let bolt11_invoice = match invoice_str.parse::<Bolt11Invoice>() {
                        Ok(invoice) => invoice,
                        Err(e) => {
                            println!("Error parsing invoice: {}", e);
                            continue;
                        }
                    };
                    
                    match user.node().bolt11_payment().send(&bolt11_invoice, None) {
                        Ok(payment_id) => {
                            println!("Payment sent from User with payment_id: {}", payment_id)
                        }
                        Err(e) => println!("Error sending payment from User: {}", e),
                    }
                }
                (Some("getjitinvoice"), []) => {
                    let description = Bolt11InvoiceDescription::Direct(
                        Description::new("Stable Channel JIT payment".to_string()).unwrap_or_else(|_| {
                            println!("Failed to create description, using fallback");
                            Description::new("Fallback JIT Invoice".to_string()).unwrap()
                        })
                    );
                    
                    match user.node().bolt11_payment().receive_via_jit_channel(
                        50000000,
                        &description,
                        3600,
                        Some(10000000),
                    ) {
                        Ok(invoice) => println!("Invoice: {}", invoice.to_string()),
                        Err(e) => println!("Error: {}", e),
                    }
                }
                (Some("exit"), _) => break,
                _ => println!("Unknown command or incorrect arguments"),
            }
        }
    }

    #[cfg(feature = "lsp")]
    {
        let lsp_node = make_node("lsp", 9737, None);
        let lsp = StateManager::new(lsp_node);
        let mut their_offer: Option<Offer> = None;

        loop {
            let (input, command, args) = get_user_input("Enter command for lsp: ");

            match (command.as_deref(), args.as_slice()) {
                (Some("settheiroffer"), [their_offer_str]) => {
                    match Offer::from_str(their_offer_str) {
                        Ok(offer) => {
                            their_offer = Some(offer);
                            println!("Offer set.");
                        },
                        Err(e) => println!("Error parsing offer"),
                    }
                }
                (Some("getouroffer"), []) => {
                    match lsp.node().bolt12_payment().receive_variable_amount("thanks", None) {
                        Ok(our_offer) => println!("{}", our_offer),
                        Err(e) => println!("Error creating offer: {}", e),
                    }
                }
                (Some("getaddress"), []) => {
                    let funding_address = lsp.node().onchain_payment().new_address();
                    match funding_address {
                        Ok(fund_addr) => println!("LSP Funding Address: {}", fund_addr),
                        Err(e) => println!("Error getting funding address: {}", e),
                    }
                }
                (Some("openchannel"), args) => {
                    if args.len() != 3 {
                        println!("Error: 'openchannel' command requires three parameters: <node_id>, <listening_address>, and <sats>");
                        continue;
                    }

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

                    match lsp.node().open_announced_channel(
                        user_node_id,
                        lsp_net_address,
                        sats,
                        Some(sats / 2),
                        channel_config,
                    ) {
                        Ok(_) => println!("Channel successfully opened to {}", node_id_str),
                        Err(e) => println!("Failed to open channel: {}", e),
                    }
                }
                // Sample start command below:
                // startstablechannel CHANNEL_ID IS_STABLE_RECEIVER EXPECTED_DOLLAR_AMOUNT EXPECTED_BTC_AMOUNT
                // startstablechannel 569b7829b98de19a86ec7d73079a0b3c5e03686aa923e86669f6ab8397674759 false 172.0 0
                (Some("startstablechannel"), [channel_id, is_stable_receiver, expected_dollar_amount, native_amount_sats]) =>
                {
                    let channel_id = channel_id.to_string();
                    let is_stable_receiver = match is_stable_receiver.parse::<bool>() {
                        Ok(val) => val,
                        Err(_) => {
                            println!("Error: is_stable_receiver must be 'true' or 'false'");
                            continue;
                        }
                    };
                    
                    let expected_dollar_amount = match expected_dollar_amount.parse::<f64>() {
                        Ok(val) => val,
                        Err(_) => {
                            println!("Error: expected_dollar_amount must be a valid number");
                            continue;
                        }
                    };
                    
                    let native_amount_sats = match native_amount_sats.parse::<f64>() {
                        Ok(val) => val,
                        Err(_) => {
                            println!("Error: native_amount_sats must be a valid number");
                            continue;
                        }
                    };

                    // Initialize the stable channel using our state manager
                    match lsp.initialize_stable_channel(
                        &channel_id, 
                        is_stable_receiver, 
                        expected_dollar_amount, 
                        native_amount_sats
                    ) {
                        Ok(()) => {
                            println!("Stable Channel initialized: {}", channel_id);
                            
                            // Start the stability checking loop
                            loop {
                                let now = SystemTime::now();
                                let now_duration = now.duration_since(UNIX_EPOCH).unwrap_or_else(|_| {
                                    println!("Error getting system time, using default");
                                    Duration::from_secs(0)
                                });
                                
                                let now_secs = now_duration.as_secs();
                                let next_60_sec = ((now_secs / 60) + 1) * 60;
                                let next_60_sec_duration = Duration::from_secs(next_60_sec);
                                let sleep_duration = next_60_sec_duration
                                    .checked_sub(now_duration)
                                    .unwrap_or_else(|| Duration::from_secs(0));

                                // Sleep until the next 60-second mark
                                std::thread::sleep(sleep_duration);

                                println!();
                                println!("\nChecking stability for channel {}...\n", channel_id);
                                
                                // Perform stability check and take appropriate action
                                let action = lsp.check_stability();
                                match action {
                                    StabilityAction::Pay(amount) => {
                                        match lsp.execute_payment(amount) {
                                            Ok(payment_id) => println!("Payment sent with ID: {}", payment_id),
                                            Err(e) => println!("Failed to send payment: {}", e),
                                        }
                                    },
                                    StabilityAction::Wait => {
                                        println!("Waiting for counterparty payment...");
                                        // Wait 10 seconds then check again
                                        std::thread::sleep(std::time::Duration::from_secs(10));
                                        lsp.update_balances();
                                    },
                                    StabilityAction::DoNothing => {
                                        println!("Channel stable, no action needed.");
                                    },
                                    StabilityAction::HighRisk(risk) => {
                                        println!("Risk level high: {}", risk);
                                    },
                                    StabilityAction::NotInitialized => {
                                        println!("Channel not properly initialized or may have been closed. Exiting stability loop.");
                                        // Exit the loop if the channel is not initialized
                                    }
                                }
                            }
                        },
                        Err(e) => println!("Failed to initialize stable channel: {}", e),
                    }
                }
                (Some("balance"), []) => {
                    let balances = lsp.node().list_balances();
                    let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                    let lightning_balance =
                        Bitcoin::from_sats(balances.total_lightning_balance_sats);
                    
                    // Get stable channel info if initialized
                    let sc = lsp.get_stable_channel();
                    if sc.latest_price > 0.0 {
                        println!("LSP On-Chain Balance: {}", onchain_balance);
                        println!("LSP Lightning Balance: {}", lightning_balance);
                        println!("Current BTC/USD Price: ${:.2}", sc.latest_price);
                        
                        // Print stable channel balances
                        if !sc.is_stable_receiver {
                            // LSP is the provider
                            println!("LSP Provider Balance: {} (${:.2})",
                                sc.stable_provider_btc,
                                sc.stable_provider_usd.0);
                            println!("User Receiver Balance: {} (${:.2})",
                                sc.stable_receiver_btc,
                                sc.stable_receiver_usd.0);
                        } else {
                            // LSP is the receiver
                            println!("LSP Receiver Balance: {} (${:.2})",
                                sc.stable_receiver_btc,
                                sc.stable_receiver_usd.0);
                            println!("User Provider Balance: {} (${:.2})",
                                sc.stable_provider_btc,
                                sc.stable_provider_usd.0);
                        }
                    } else {
                        println!("LSP On-Chain Balance: {}", onchain_balance);
                        println!("LSP Lightning Balance: {}", lightning_balance);
                    }
                }
                (Some("checkstability"), []) => {
                    let action = lsp.check_stability();
                    match action {
                        StabilityAction::Pay(amount) => {
                            println!("Action: Pay {} msats", amount);
                            match lsp.execute_payment(amount) {
                                Ok(payment_id) => println!("Payment sent with ID: {}", payment_id),
                                Err(e) => println!("Failed to send payment: {}", e),
                            }
                        },
                        StabilityAction::Wait => println!("Action: Wait for counterparty payment"),
                        StabilityAction::DoNothing => println!("Action: Do nothing, channel is stable"),
                        StabilityAction::HighRisk(risk) => println!("Action: High risk level ({})", risk),
                        StabilityAction::NotInitialized => {
                            println!("Channel not properly initialized or may have been closed. Exiting stability loop.");
                            break; // Exit the loop if the channel is not initialized
                        }
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
                (Some("getinvoice"), [sats]) => {
                    if let Ok(sats_value) = sats.parse::<u64>() {
                        let msats = sats_value * 1000;
                        let bolt11 = lsp.node().bolt11_payment();
                        let description = Bolt11InvoiceDescription::Direct(
                            Description::new("test invoice".to_string()).unwrap_or_else(|_| {
                                println!("Failed to create description, using fallback");
                                Description::new("Fallback Invoice".to_string()).unwrap()
                            })
                        );
                        
                        let invoice = bolt11.receive(msats, &description, 6000);
                        match invoice {
                            Ok(inv) => println!("LSP Invoice: {}", inv),
                            Err(e) => println!("Error creating invoice: {}", e),
                        }
                    } else {
                        println!("Invalid sats value provided");
                    }
                }
                (Some("payinvoice"), [invoice_str]) => {
                    let bolt11_invoice = match invoice_str.parse::<Bolt11Invoice>() {
                        Ok(invoice) => invoice,
                        Err(e) => {
                            println!("Error parsing invoice: {}", e);
                            continue;
                        }
                    };
                    
                    match lsp.node().bolt11_payment().send(&bolt11_invoice, None) {
                        Ok(payment_id) => {
                            println!("Payment sent from LSP with payment_id: {}", payment_id)
                        }
                        Err(e) => println!("Error sending payment from LSP: {}", e),
                    }
                }
                (Some("exit"), _) => break,
                _ => println!("Unknown command or incorrect arguments: {}", input),
            }
        }
    }
}