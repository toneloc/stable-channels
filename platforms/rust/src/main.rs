// Contents
// Section 1 - Dependencies and main data structure
// Section 2 - Price feed config and logic
// Section 3 - Core logic 
// Section 4 - Program initialization

// Section 1 - Dependencies and main data structure
use std::fmt;
use std::time::Duration;
use reqwest::blocking::ClientBuilder;
use reqwest::StatusCode;
use serde_json::Value;
use std::error::Error;
use std::collections::HashMap;
use reqwest::blocking::Client;
use retry::{retry, delay::Fixed};

// Main data structure
struct StableChannel {
    channel_id: String,
    expected_dollar_amount: f64,
    native_amount_msat: u64,
    is_stable_receiver: bool,
    counterparty: String,
    our_balance: f64,
    their_balance: f64,
    risk_score: i32,
    stable_receiver_dollar_amount: f64,
    stable_provider_dollar_amount: f64,
    timestamp: i64,
    formatted_datetime: String,
    payment_made: bool,
    sc_dir: String,
}

// Implement methods for StableChannel
impl StableChannel {
    fn new(
        channel_id: String,
        expected_dollar_amount: f64,
        native_amount_msat: u64,
        is_stable_receiver: bool,
        counterparty: String,
        our_balance: f64,
        their_balance: f64,
        risk_score: i32,
        stable_receiver_dollar_amount: f64,
        stable_provider_dollar_amount: f64,
        timestamp: i64,
        formatted_datetime: String,
        payment_made: bool,
        sc_dir: String,
    ) -> StableChannel {
        StableChannel {
            channel_id,
            expected_dollar_amount,
            native_amount_msat,
            is_stable_receiver,
            counterparty,
            our_balance,
            their_balance,
            risk_score,
            stable_receiver_dollar_amount,
            stable_provider_dollar_amount,
            timestamp,
            formatted_datetime,
            payment_made,
            sc_dir,
        }
    }
}

impl fmt::Display for StableChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "StableChannel(\n\
            channel_id={},\n\
            expected_dollar_amount={},\n\
            native_amount_msat={},\n\
            is_stable_receiver={},\n\
            counterparty={},\n\
            our_balance={},\n\
            their_balance={},\n\
            risk_score={},\n\
            stable_receiver_dollar_amount={},\n\
            stable_provider_dollar_amount={},\n\
            timestamp={},\n\
            formatted_datetime={},\n\
            payment_made={},\n\
            sc_dir={}\n\
            )",
            self.channel_id,
            self.expected_dollar_amount,
            self.native_amount_msat,
            self.is_stable_receiver,
            self.counterparty,
            self.our_balance,
            self.their_balance,
            self.risk_score,
            self.stable_receiver_dollar_amount,
            self.stable_provider_dollar_amount,
            self.timestamp,
            self.formatted_datetime,
            self.payment_made,
            self.sc_dir
        )
    }
}

// // Section 2 - Price feed config and logic
struct PriceFeed {
    name: String,
    urlformat: String,
    replymembers: Vec<String>,
}

impl PriceFeed {
    fn new(name: &str, urlformat: &str, replymembers: Vec<&str>) -> PriceFeed {
        PriceFeed {
            name: name.to_string(),
            urlformat: urlformat.to_string(),
            replymembers: replymembers.iter().map(|&s| s.to_string()).collect(),
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

    for feed in price_feeds {
        let url: String = feed.urlformat.replace("{currency_lc}", "usd").replace("{currency}", "USD");

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

        for key in &feed.replymembers {
            if let Some(inner_data) = data.get(key) {
                data = inner_data;
            } else {
                println!("Key '{}' not found in the response from {}", key, feed.name);
                continue;
            }
        }

        if let Some(price) = data.as_f64() {
            prices.push((feed.name.clone(), price));
        } else if let Some(price_str) = data.as_str() {
            if let Ok(price) = price_str.parse::<f64>() {
                prices.push((feed.name.clone(), price));
            } else {
                println!("Invalid price format for {}: {}", feed.name, price_str);
            }
        } else {
            println!("Price data not found or invalid format for {}", feed.name);
        }
    }

    if prices.is_empty() {
        return Err("No valid prices fetched.".into());
    }

    Ok(prices)
}

fn print_prices_and_median(prices: Vec<(String, f64)>) -> Result<(), Box<dyn Error>> {
    // Print all prices
    for (feed_name, price) in &prices {
        println!("{}: ${:.2}", feed_name, price);
    }

    // Calculate the median price
    let mut price_values: Vec<f64> = prices.iter().map(|(_, price)| *price).collect();
    price_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_price = if price_values.len() % 2 == 0 {
        (price_values[price_values.len() / 2 - 1] + price_values[price_values.len() / 2]) / 2.0
    } else {
        price_values[price_values.len() / 2]
    };

    println!("The median BTC/USD price is: ${:.2}", median_price);

    Ok(())
}

// Section 3 - Core logic 

// Section 4 - Program initialization
 
fn main() -> Result<(), Box<dyn Error>> {
    let client = Client::new();
    
    let price_feeds = set_price_feeds();

    let prices = fetch_prices(&client, &price_feeds)?;

    print_prices_and_median(prices)?;

    // Initializing a new StableChannel 
    let channel_id = "example_channel_id".to_string();
    let stable_dollar_amount: f64 = 100.0;
    let native_amount: f64 = 5000.0;
    let is_stable_receiver: bool = true;
    let native_amount_msat = (native_amount * 1000.0) as u64; 
    let counterparty = "example_counterparty".to_string();
    let our_balance = 2500.0;
    let their_balance = 2500.0;
    let risk_score = 5;
    let stable_receiver_dollar_amount = 50.0;
    let stable_provider_dollar_amount = 50.0;
    let timestamp = 1609459200;  
    let formatted_datetime = "2021-01-01 00:00:00".to_string();
    let payment_made = false;
    let sc_dir = "/path/to/sc_dir".to_string();

    let channel = StableChannel::new(
        channel_id,
        stable_dollar_amount,
        native_amount_msat,
        is_stable_receiver,
        counterparty,
        our_balance,
        their_balance,
        risk_score,
        stable_receiver_dollar_amount,
        stable_provider_dollar_amount,
        timestamp,
        formatted_datetime,
        payment_made,
        sc_dir,
    );

    println!("{}", channel);

    Ok(())
}


