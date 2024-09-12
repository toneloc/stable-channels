
// Contents
// Section 1 - Dependencies, main data structure. helper functions
// Section 2 - LDK set-up
// Section 3 - Price feed config and logic
// Section 4 - Core stability logic 
// Section 5 - Program initialization and command-line-interface

// Section 1 - Dependencies and main data structure
extern crate ldk_node_hack;
use lightning_liquidity::events::Event;
use lightning_liquidity::lsps2::client::LSPS2ClientConfig;
use lightning_liquidity::lsps2::event::{LSPS2ClientEvent, LSPS2ServiceEvent};
use lightning_liquidity::lsps2::msgs::RawOpeningFeeParams;
use lightning_liquidity::lsps2::service::LSPS2ServiceConfig;
use lightning_liquidity::lsps2::utils::is_valid_opening_fee_params;
use lightning_liquidity::{LiquidityClientConfig, LiquidityServiceConfig};
use lightning_liquidity::LiquidityManager;

use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::lightning::ln::ChannelId;
use ldk_node::lightning::offers::offer::Offer;
use ldk_node::{lightning_invoice::Bolt11Invoice, Node, Builder};
use ldk_node::bitcoin::Network;

use std::ops::{Div, Sub};
use std::str::FromStr;
use std::{io::{self, Write}, sync::Arc, thread};
use ldk_node::{ChannelConfig, ChannelDetails};
use std::time::Duration;
use serde_json::Value;
use std::error::Error;
use reqwest::blocking::Client;
use retry::{retry, delay::Fixed};

// Main data structure
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
struct Bitcoin {
    sats: u64, // Stored in Satoshis for precision
}

impl Bitcoin {
    const SATS_IN_BTC: u64 = 100_000_000;

    fn from_sats(sats: u64) -> Self {
        Self { sats }
    }

    fn from_btc(btc: f64) -> Self {
        let sats = (btc * Self::SATS_IN_BTC as f64).round() as u64;
        Self::from_sats(sats)
    }

    fn to_btc(self) -> f64 {
        self.sats as f64 / Self::SATS_IN_BTC as f64
    }

    fn from_usd(usd: f64, btcusd_price: f64) -> Self {
        let btc_value = usd / btcusd_price;
        Self::from_btc(btc_value)
    }
}

impl Sub for Bitcoin {
    type Output = Bitcoin;

    fn sub(self, other: Bitcoin) -> Bitcoin {
        Bitcoin::from_sats(self.sats.saturating_sub(other.sats))
    }
}

impl std::fmt::Display for Bitcoin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let btc_value = self.to_btc();

        // Format the value to 8 decimal places with spaces
        let formatted_btc = format!("{:.8}", btc_value);
        let with_spaces = formatted_btc
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 4 || i == 7 { format!(" {}", c) } else { c.to_string() })
            .collect::<String>();

        write!(f, "{}btc", with_spaces)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
struct USD(f64);

impl USD {
    fn from_bitcoin(btc: Bitcoin, btcusd_price: f64) -> Self {
        Self(btc.to_btc() * btcusd_price)
    }

    fn from_f64(amount: f64) -> Self {
        Self(amount)
    }

    fn to_msats(self, btcusd_price: f64) -> u64 {
        let btc_value = self.0 / btcusd_price; 
        let sats = btc_value * Bitcoin::SATS_IN_BTC as f64;
        let millisats = sats * 1000.0;
        millisats.floor() as u64
    }


}

impl Sub for USD {
    type Output = USD;

    fn sub(self, other: USD) -> USD {
        USD(self.0 - other.0)
    }
}

impl Div<f64> for USD {
    type Output = USD;

    fn div(self, scalar: f64) -> USD {
        USD(self.0 / scalar)
    }
}

impl Div for USD {
    type Output = f64;

    fn div(self, other: USD) -> f64 {
        self.0 / other.0
    }
}

impl std::fmt::Display for USD {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "${:.2}", self.0)
    }
}

#[derive(Clone, Debug)]
struct StableChannel {
    channel_id: ChannelId, 
    is_stable_receiver: bool,
    counterparty: PublicKey, 
    expected_usd: USD,
    expected_btc: Bitcoin,
    stable_receiver_btc: Bitcoin,
    stable_provider_btc: Bitcoin,   
    stable_receiver_usd: USD,
    stable_provider_usd: USD,
    risk_level: i32,
    timestamp: i64,
    formatted_datetime: String,
    payment_made: bool,
    sc_dir: String,
    latest_price: f64,
    prices: String,
    counterparty_offer: Offer
}

// Section 2 - LDK set-up and helper functions
fn make_hack_node(alias: &str, port: u16) -> ldk_node_hack::Node {

    let mut builder = ldk_node_hack::Builder::new();

    let promise_secret = [0u8; 32];
    builder.set_liquidity_provider_lsps2(promise_secret);

    builder.set_network(Network::Signet);
    builder.set_esplora_server("https://mutinynet.ltbl.io/api".to_string());
    // builder.set_gossip_source_rgs("https://mutinynet.ltbl.io/snapshot".to_string());
    builder.set_storage_dir_path(("./data/".to_owned() + alias).to_string());

    builder.set_listening_addresses(vec![format!("127.0.0.1:{}", port).parse().unwrap()]);

    let node = builder.build().unwrap();

    node.start().unwrap();

    println!("{} public key: {}", alias, node.node_id());

    return node;
}

// Section 2 - LDK set-up and helper functions
fn make_node(alias: &str, port: u16, lsp_pubkey:Option<PublicKey>) -> ldk_node::Node {
    let mut builder = Builder::new();

    // If we pass in an LSP pubkey then set your liquidity source
    if let Some(lsp_pubkey) = lsp_pubkey {
        println!("j");
        println!("{}", lsp_pubkey.to_string());
        let address = "127.0.0.1:9377".parse().unwrap();
        builder.set_liquidity_source_lsps2(address, lsp_pubkey, Some("00000000000000000000000000000000".to_owned()));
    }

    builder.set_network(Network::Signet);
    builder.set_esplora_server("https://mutinynet.ltbl.io/api".to_string());
    // builder.set_gossip_source_rgs("https://mutinynet.ltbl.io/snapshot".to_string());
    builder.set_storage_dir_path(("./data/".to_owned() + alias).to_string());
    builder.set_listening_addresses(vec![format!("127.0.0.1:{}", port).parse().unwrap()]);

    let node = builder.build().unwrap();

    node.start().unwrap();

    println!("{} public key: {}", alias, node.node_id());

    return node;
}

fn make_node_test(alias: &str, port: u16, lsp_pubkey:Option<PublicKey>) -> ldk_node::Node {
    let mut builder = Builder::new();

    let promise_secret = [0u8; 32];
    builder.set_liquidity_provider_lsps2(promise_secret);

    builder.set_network(Network::Signet);
    builder.set_esplora_server("https://mutinynet.ltbl.io/api".to_string());
    // builder.set_gossip_source_rgs("https://mutinynet.ltbl.io/snapshot".to_string());
    builder.set_storage_dir_path(("./data/".to_owned() + alias).to_string());
    builder.set_listening_addresses(vec![format!("127.0.0.1:{}", port).parse().unwrap()]);

    let node = builder.build().unwrap();

    node.start().unwrap();

    println!("{} public key: {}", alias, node.node_id());

    return node;
}

// Section 3 - Price feed config and logic
struct PriceFeed {
    name: String,
    urlformat: String,
    jsonpath: Vec<String>,
}

impl PriceFeed {
    fn new(name: &str, urlformat: &str, jsonpath: Vec<&str>) -> PriceFeed {
        PriceFeed {
            name: name.to_string(),
            urlformat: urlformat.to_string(),
            jsonpath: jsonpath.iter().map(|&s| s.to_string()).collect(),
        }
    }
}

fn set_price_feeds() -> Vec<PriceFeed> {
    vec![
        PriceFeed::new(
            "bitstamp",
            "https://www.bitstamp.net/api/v2/ticker/btcusd/",
            vec!["last"],
        ),
        PriceFeed::new(
            "coingecko",
            "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd",
            vec!["bitcoin", "usd"],
        ),
        PriceFeed::new(
            "coindesk",
            "https://api.coindesk.com/v1/bpi/currentprice/USD.json",
            vec!["bpi", "USD", "rate_float"],
        ),
        PriceFeed::new(
            "coinbase",
            "https://api.coinbase.com/v2/prices/spot?currency=USD",
            vec!["data", "amount"],
        ),
        PriceFeed::new(
            "blockchain.info",
            "https://blockchain.info/ticker",
            vec!["USD", "last"],
        ),
    ]
}

fn fetch_prices(client: &Client, price_feeds: &[PriceFeed]) -> Result<Vec<(String, f64)>, Box<dyn Error>> {
    let mut prices = Vec::new();

    for price_feed in price_feeds {
        let url: String = price_feed.urlformat.replace("{currency_lc}", "usd").replace("{currency}", "USD");

        let response = retry(Fixed::from_millis(300).take(3), || {
            match client.get(&url).send() {
                Ok(resp) => {
                    if resp.status().is_success() {
                        Ok(resp)
                    } else {
                        Err(format!("Received status code: {}", resp.status()))
                    }
                },
                Err(e) => Err(e.to_string()),
            }
        }).map_err(|e| -> Box<dyn Error> { Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())) })?;

        let json: Value = response.json()?;
        let mut data = &json;

        for key in &price_feed.jsonpath {
            if let Some(inner_data) = data.get(key) {
                data = inner_data;
            } else {
                println!("Key '{}' not found in the response from {}", key, price_feed.name);
                continue;
            }
        }

        if let Some(price) = data.as_f64() {
            prices.push((price_feed.name.clone(), price));
        } else if let Some(price_str) = data.as_str() {
            if let Ok(price) = price_str.parse::<f64>() {
                prices.push((price_feed.name.clone(), price));
            } else {
                println!("Invalid price format for {}: {}", price_feed.name, price_str);
            }
        } else {
            println!("Price data not found or invalid format for {}", price_feed.name);
        }
    }

    if prices.len() < 5 {
        println!("Fewer than 5 fetched.");
    }

    if prices.is_empty() {
        return Err("No valid prices fetched.".into());
    }

    Ok(prices)
}

fn calculate_median_price(prices: Vec<(String, f64)>) -> Result<f64, Box<dyn std::error::Error>> {
    // Print all prices
    for (feed_name, price) in &prices {
        println!("{:<25} ${:>14.2}", feed_name, price);    }

    // Calculate the median price
    let mut price_values: Vec<f64> = prices.iter().map(|(_, price)| *price).collect();
    price_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_price = if price_values.len() % 2 == 0 {
        (price_values[price_values.len() / 2 - 1] + price_values[price_values.len() / 2]) / 2.0
    } else {
        price_values[price_values.len() / 2]
    };

    println!("Median BTC/USD price : ${:.2}", median_price);

    Ok(median_price)
}

// Section 4 - Core stability logic 
fn check_stability(node: &Node, mut sc: StableChannel) -> StableChannel {
    // Fetch and update prices
    sc.latest_price = fetch_prices(&Client::new(), &set_price_feeds())
        .and_then(|prices| calculate_median_price(prices))
        .unwrap_or(0.0);

    // Update channel balances
    if let Some(channel) = node.list_channels().iter().find(|c| c.channel_id == sc.channel_id) {
        sc = update_balances(sc, Some(channel.clone()));
    }

    // Calculate how far off 100% par we are 
    let dollars_from_par: USD = sc.stable_receiver_usd - sc.expected_usd;
    let percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();

    // Print balance information
    println!("{:<25} {:>15}", "User BTC:", sc.stable_receiver_btc);
    println!("{:<25} {:>15}", "Expected USD:", sc.expected_usd);
    println!("{:<25} {:>15}", "User USD:", sc.stable_receiver_usd);
    println!("{:<25} {:>15}", "Expected BTC:", sc.expected_btc);
    println!("{:<25} {:>15}", "LSP USD:", sc.stable_provider_usd);
    println!("{:<25} {:>5}", "Percent from par:", format!("{:.2}%", percent_from_par));

    enum Action {
        Wait,
        Pay,
        DoNothing,
        HighRisk,
    }

    // Determine action based on channel state and risk level
    let action = if percent_from_par < 0.1 {
        Action::DoNothing
    } else {
        let is_receiver_below_expected: bool = sc.stable_receiver_usd < sc.expected_usd;
        
        match (sc.is_stable_receiver, is_receiver_below_expected, sc.risk_level > 100) {
            (_, _, true) => Action::HighRisk, // High risk scenario
            (true, true, false) => Action::Wait,   // We are User and below peg, wait for payment
            (true, false, false) => Action::Pay,   // We are User and above peg, need to pay
            (false, true, false) => Action::Pay,   // We are LSP and below peg, need to pay
            (false, false, false) => Action::Wait, // We are LSP and above peg, wait for payment
        }
    };

    match action {
        // update state after each
        Action::DoNothing => println!("Difference from par less than 0.1%. Doing nothing."),
        Action::Wait => {
            println!("Waiting 10 seconds and checking on payment...");
            std::thread::sleep(std::time::Duration::from_secs(10));
            if let Some(channel) = node
                .list_channels()
                .iter()
                .find(|c| c.channel_id == sc.channel_id) {sc = update_balances(sc, Some(channel.clone()));
            }
        },
        Action::Pay => {
            println!("Paying the difference...");
            
            let mut amt = USD::to_msats(dollars_from_par, sc.latest_price);
            println!("{}", amt.to_string());
            
            // First, ensure we are connected
            let address = format!("127.0.0.1:9376").parse().unwrap();
            let result = node.connect(sc.counterparty, address, true);

            if let Err(e) = result {
                println!("Failed to connect with : {}", e);
            } else {
                println!("Successfully connected.");
            }

            // let result = node
            //     .spontaneous_payment()
            //     .send(amt, sc.counterparty);
            // match result {
            //     Ok(payment_id) => println!("Payment sent successfully with payment ID: {}", payment_id),
            //     Err(e) => println!("Failed to send payment: {}", e),
            // }

            let result = node.bolt12_payment().send_using_amount(&sc.counterparty_offer,Some("here ya go".to_string()),amt);
            
            match result {
                Ok(payment_id) => println!("Payment sent successfully with ID: {:?}", payment_id),
                Err(e) => eprintln!("Failed to send payment: {:?}", e),
            }
        },
        Action::HighRisk => {
            println!("Risk level high. Current risk level: {}", sc.risk_level);
        },
    }

    sc
}

fn update_balances(mut sc: StableChannel, channel_details: Option<ChannelDetails>) -> StableChannel {
    let (our_balance, their_balance) = match channel_details {
        Some(channel) => {
            let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
            let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
            let their_balance_sats = channel.channel_value_sats - our_balance_sats;
            (our_balance_sats, their_balance_sats)
        }
        None => (0, 0), // Handle the case where channel_details is None
    };

    // Update balances based on whether this is a User or provider
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

    sc // Return the modified StableChannel
}

// Section 5 - Program initialization and command-line-interface
fn main() {
    // Add more nodes if you need
    let exchange = make_node("exchange", 9735, None);
    
    // start this one next so we can plugin the pubkey as LSP provider for user
    let lsp = make_hack_node("lsp", 9737);
    let lsp_pubkey = lsp.node_id();

    let user = make_node("user", 9736, Some(lsp_pubkey));

    loop {
        let mut input = String::new();
        print!("Enter command: ");
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();

        let mut parts = input.split_whitespace();
        let node = parts.next();
        let command = parts.next();
        let args: Vec<&str> = parts.collect(); // Collect remaining arguments

        // lsp startstablechannel 4073cac457084b37f54465b8329384fdad1e0a5fc5f47c7a6ffd18e17c86e3ae true 33.0 0
        match (node, command, args.as_slice()) {
            (Some("lsp"), Some("startstablechannel"), [channel_id, is_stable_receiver, expected_dollar_amount, native_amount_sats]) => {
                let channel_id = channel_id.to_string();
                let is_stable_receiver = is_stable_receiver.parse::<bool>().unwrap_or(false);
                let expected_dollar_amount = expected_dollar_amount.parse::<f64>().unwrap_or(0.0);
                let native_amount_sats = native_amount_sats.parse::<f64>().unwrap_or(0.0);

                let counterparty = lsp.list_channels()
                    .iter()
                    .find(|channel| {
                        println!("channel_id: {}", channel.channel_id);
                        channel.channel_id.to_string() == channel_id
                    })
                    .map(|channel| channel.counterparty_node_id)
                    .expect("Failed to find channel with the specified sID");
            
                let channel_id_bytes: [u8; 32] = hex::decode(channel_id)
                    .expect("Invalid hex string")
                    .try_into()
                    .expect("Decoded channel ID has incorrect length");

                // fix
                let offer = user.bolt12_payment().receive_variable_amount("thanks").unwrap();

                let mut stable_channel = StableChannel {
                    channel_id: ChannelId::from_bytes(channel_id_bytes),
                    is_stable_receiver,  
                    counterparty,
                    expected_usd: USD::from_f64(expected_dollar_amount),
                    expected_btc: Bitcoin::from_btc(native_amount_sats),
                    stable_receiver_btc: Bitcoin::from_btc(0.0),
                    stable_provider_btc: Bitcoin::from_btc(0.0),  
                    stable_receiver_usd: USD::from_f64(0.0),
                    stable_provider_usd: USD::from_f64(0.0),
                    risk_level: 0, 
                    timestamp: 0,
                    formatted_datetime: "2021-06-01 12:00:00".to_string(), 
                    payment_made: false,
                    sc_dir: "/path/to/sc_dir".to_string(),
                    latest_price: 0.0, 
                    prices: "".to_string(),
                    counterparty_offer: offer
                };

                println!("Stable Channel created: {:?}", stable_channel.channel_id);

                loop {
                    println!();
                    println!("Checking stability for channel {}...", stable_channel.channel_id);
                    
                    stable_channel = check_stability(&exchange, stable_channel);

                    thread::sleep(Duration::from_secs(20));
                };
            },
            // Open to LSP
            (Some("exchange"), Some("openchannel"), []) => {
                let channel_config: Option<Arc<ChannelConfig>> = None;
                
                let announce_channel = true;

                // Extract the first listening address
                if let Some(listening_addresses) = lsp.listening_addresses() {
                    if let Some(lsp_addr) = listening_addresses.get(0) {
                        match exchange.connect_open_channel(lsp.node_id(), lsp_addr.clone(), 500000, Some(0), channel_config, announce_channel) {
                            Ok(result) => println!("Successfully opened channel between exchange and lsp."),
                            Err(e) => println!("Failed to open channel: {}", e),
                        }
                    } else {
                        println!("lsp has no listening addresses.");
                    }
                } else {
                    println!("Failed to get listening addresses for lsp.");
                }
            },
            (Some("exchange"), Some("getaddress"), []) => {
                let funding_address = exchange.onchain_payment().new_address();
                match funding_address {
                    Ok(fund_addr) => println!("Exchange Funding Address: {}", fund_addr),
                    Err(e) => println!("Error getting funding address: {}", e),
                }
            } 
            (Some("user"), Some("getaddress"), []) => {
                let funding_address = user.onchain_payment().new_address();
                match funding_address {
                    Ok(fund_addr) => println!("User Funding Address: {}", fund_addr),
                    Err(e) => println!("Error getting funding address: {}", e),
                }
            }
            (Some("lsp"), Some("getaddress"), []) => {
                let funding_address = lsp.onchain_payment().new_address();
                match funding_address {
                    Ok(fund_addr) => println!("LSP Funding Address: {}", fund_addr),
                    Err(e) => println!("Error getting funding address: {}", e),
                }
            }
            (Some("user"), Some("openchannel"), []) => {
                let channel_config: Option<Arc<ChannelConfig>> = None;    
                let announce_channel = true;

                // Extract the first listening address
                if let Some(listening_addresses) = lsp.listening_addresses() {
                    if let Some(lsp_addr) = listening_addresses.get(0) {
                        match user.connect_open_channel(lsp.node_id(), lsp_addr.clone(), 300000, Some(0), channel_config, announce_channel) {
                            Ok(_) => println!("Channel successfully opened between user and lsp."),
                            Err(e) => println!("Failed to open channel: {}", e),
                        }
                    } else {
                        println!("lsp has no listening addresses.");
                    }
                } else {
                    println!("Failed to get listening addresses for lsp.");
                }
            },
            (Some("exchange"), Some("balance"), []) => {
                let balances = exchange.list_balances();
                let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                let lightning_balance = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                println!("Exchange On-Chain Balance: {}", onchain_balance);
                println!("Exchange Lightning Balance: {}", lightning_balance);
            },
            (Some("user"), Some("balance"), []) => {
                let balances = user.list_balances();
                let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                let lightning_balance = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                println!("User On-Chain Balance: {}", onchain_balance);
                println!("Stabl Receiver Lightning Balance: {}", lightning_balance);
            },
            (Some("lsp"), Some("balance"), []) => { 
                let balances = lsp.list_balances();
                let onchain_balance = Bitcoin::from_sats(balances.total_onchain_balance_sats);
                let lightning_balance = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                println!("LSP On-Chain Balance: {}", onchain_balance);
                println!("LSP Lightning Balance: {}", lightning_balance);
            },
            (Some("user"), Some("connecttolsp"), []) => {
                if let Some(listening_addresses) = lsp.listening_addresses() {
                    if let Some(lsp_addr) = listening_addresses.get(0) {
                        match user.connect(lsp.node_id(), lsp_addr.clone(),true) {
                            Ok(_) => println!("Connected user and lsp."),
                            Err(e) => println!("Failed to connect: {}", e),
                        }
                    } else {
                        println!("lsp has no listening addresses.");
                    }
                } else {
                    println!("Failed to get listening addresses for lsp.");
                }
            },
            (Some("exchange"), Some("closeallchannels"), []) => {
                for channel in exchange.list_channels().iter() {
                    let user_channel_id = channel.user_channel_id;
                    let counterparty_node_id = channel.counterparty_node_id;
                    let _ = exchange.close_channel(&user_channel_id, counterparty_node_id);
                }
            },
            (Some("user"), Some("closeallchannels"), []) => {
                for channel in user.list_channels().iter() {
                    let user_channel_id = channel.user_channel_id;
                    let counterparty_node_id = channel.counterparty_node_id;
                    let _ = user.close_channel(&user_channel_id, counterparty_node_id);
                }
            },
            (Some("exchange"), Some("listallchannels"), []) => {
                println!("{}", "channels:");
                for channel in exchange.list_channels().iter() {
                    let channel_id = channel.channel_id;
                    println!("{}", channel_id);
                }
                println!("{}", "channel details:");
                let channels = exchange.list_channels();
                println!("{:#?}", channels);
               
            },
            (Some("user"), Some("listallchannels"), []) => {
                println!("{}", "channels:");
                for channel in user.list_channels().iter() {
                    let channel_id = channel.channel_id;
                    println!("{}", channel_id);
                }
                println!("{}", "channel details:");
                let channels = user.list_channels();
                println!("{:#?}", channels);
               
            },
            (Some("lsp"), Some("listallchannels"), []) => {
                println!("{}", "channels:");
                for channel in lsp.list_channels().iter() {
                    let channel_id = channel.channel_id;
                    println!("{}", channel_id);
                }
                println!("{}", "channel details:");
                let channels = lsp.list_channels();
                println!("{:#?}", channels);
               
            },
            (Some("exchange"), Some("getinvoice"), []) => {
                let bolt11 = exchange.bolt11_payment();
                let invoice = bolt11.receive(10000, "test invoice", 6000);
                match invoice {
                    Ok(inv) => {
                        println!("Exchange Invoice: {}", inv);
                    },
                    Err(e) => println!("Error creating invoice: {}", e)
                }
            },
            (Some("user"), Some("getinvoice"), []) => {
                let bolt11 = user.bolt11_payment();
                let invoice = bolt11.receive(1000000, "test invoice", 6000);
                match invoice {
                    Ok(inv) => {
                        println!("UserInvoice: {}", inv);
                    },
                    Err(e) => println!("Error creating invoice: {}", e)
                }
            },
            (Some("lsp"), Some("getinvoice"), []) => {
                let bolt11 = lsp.bolt11_payment();
                let invoice = bolt11.receive(22222, "test invoice", 6000);
                match invoice {
                    Ok(inv) => {
                        println!("LSP Invoice: {}", inv);
                    },
                    Err(e) => println!("Error creating invoice: {}", e)
                }
            },
            (Some("exchange"), Some("payjitinvoicewithamount"), [invoice_str]) => {
                let bolt11_invoice = invoice_str.parse::<Bolt11Invoice>();
                match bolt11_invoice {
                    Ok(invoice) => {
                        match exchange.bolt11_payment().send(&invoice) {
                            Ok(payment_id) => {
                                println!("Payment sent from Exchange with payment_id: {}", payment_id);
                            },
                            Err(e) => {
                                println!("Error sending payment from Exchange: {}", e);
                            }
                        }
                    },
                    Err(e) => {
                        println!("Error parsing invoice: {}", e);
                    }
                }
            },
            (Some("exchange"), Some("payjitinvoice"), [invoice_str]) => {
                let bolt11_invoice = invoice_str.parse::<Bolt11Invoice>();
                match bolt11_invoice {
                    Ok(invoice) => {
                        match exchange.bolt11_payment().send(&invoice) {
                            Ok(payment_id) => {
                                println!("Payment sent from Exchange with payment_id: {}", payment_id);
                            },
                            Err(e) => {
                                println!("Error sending payment from Exchange: {}", e);
                            }
                        }
                    },
                    Err(e) => {
                        println!("Error parsing invoice: {}", e);
                    }
                }
            },
            (Some("exchange"), Some("payinvoice"), [invoice_str]) => {
                let bolt11_invoice = invoice_str.parse::<Bolt11Invoice>();
                match bolt11_invoice {
                    Ok(invoice) => {
                        match exchange.bolt11_payment().send(&invoice) {
                            Ok(payment_id) => {
                                println!("Payment sent from Exchange with payment_id: {}", payment_id);
                            },
                            Err(e) => {
                                println!("Error sending payment from Exchange: {}", e);
                            }
                        }
                    },
                    Err(e) => {
                        println!("Error parsing invoice: {}", e);
                    }
                }
            },
            (Some("user"), Some("payinvoice"), [invoice_str]) => {
                let bolt11_invoice = invoice_str.parse::<Bolt11Invoice>();
                match bolt11_invoice {
                    Ok(invoice) => {
                        match user.bolt11_payment().send(&invoice) {
                            Ok(payment_id) => {
                                println!("Payment sent from Userwith payment_id: {}", payment_id);
                            },
                            Err(e) => {
                                println!("Error sending payment from Node 2: {}", e);
                            }
                        }
                    },
                    Err(e) => {
                        println!("Error parsing invoice: {}", e);
                    }
                }
            },
            (Some("lsp"), Some("payinvoice"), [invoice_str]) => {
                // lsp.bolt11_payment().
                let bolt11_invoice = invoice_str.parse::<Bolt11Invoice>();
                match bolt11_invoice {
                    Ok(invoice) => {
                        match lsp.bolt11_payment().send(&invoice) {
                            Ok(payment_id) => {
                                println!("Payment sent from LSP with payment_id: {}", payment_id);
                            },
                            Err(e) => {
                                println!("Error sending payment from LSP: {}", e);
                            }
                        }
                    },
                    Err(e) => {
                        println!("Error parsing invoice: {}", e);
                    }
                }
            },
            (Some("user"), Some("getjitinvoice"), []) => {
             
                match user.bolt11_payment().receive_via_jit_channel(
                    50000000, 
                    "Stable Channel", 
                    3600, Some(10000000)) {
                        Ok(invoice) => println!("Invoice: {:?}", invoice.to_string()),
                        Err(e) => println!("Error: {:?}", e),
                }
                
                // match user.bolt11_payment().receive_variable_amount_via_jit_channel("thanks", 3600, Some(1000)) {
                //     Ok(invoice) => println!("Invoice: {:?}", invoice.to_string()),
                //     Err(e) => println!("Error: {:?}", e),
                // };
                
            },
            (Some("exit"), _, _) => break,
            _ => println!("Unknown command or incorrect arguments: {}", input),
        }
    }
}
