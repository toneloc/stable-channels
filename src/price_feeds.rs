use ureq::Agent;
use serde_json::Value;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use retry::{retry, delay::Fixed};
use crate::audit::audit_event;
use serde_json::json;

lazy_static::lazy_static! {
    static ref PRICE_CACHE: Arc<Mutex<PriceCache>> = Arc::new(Mutex::new(PriceCache {
        price: 0.0,
        last_update: Instant::now() - Duration::from_secs(10),
        updating: false,
    }));
}

pub struct PriceCache {
    price: f64,
    last_update: Instant,
    updating: bool,
}

pub struct PriceFeed {
    pub name: String,
    pub urlformat: String,
    pub jsonpath: Vec<String>,
}

impl PriceFeed {
    pub fn new(name: &str, urlformat: &str, jsonpath: Vec<&str>) -> PriceFeed {
        PriceFeed {
            name: name.to_string(),
            urlformat: urlformat.to_string(),
            jsonpath: jsonpath.iter().map(|&s| s.to_string()).collect(),
        }
    }
}

// Get cached price or fetch a new one if needed
pub fn get_cached_price() -> f64 {
    let should_update = {
        let cache = PRICE_CACHE.lock().unwrap();
        cache.last_update.elapsed() > Duration::from_secs(5) && !cache.updating
    };

    if should_update {
        let mut cache = PRICE_CACHE.lock().unwrap();
        cache.updating = true;
        drop(cache);

        let agent = Agent::new();
        if let Ok(new_price) = get_latest_price(&agent) {
            let mut cache = PRICE_CACHE.lock().unwrap();
            cache.price = new_price;
            cache.last_update = Instant::now();
            cache.updating = false;
            audit_event("PRICE_FETCH", json!({ "btc_price": new_price }));
            return new_price;
        } else {
            let mut cache = PRICE_CACHE.lock().unwrap();
            cache.updating = false;
            return cache.price;
        }
    }

    let cache = PRICE_CACHE.lock().unwrap();
    cache.price
}

pub fn set_price_feeds() -> Vec<PriceFeed> {
    vec![
        PriceFeed::new(
            "Bitstamp",
            "https://www.bitstamp.net/api/v2/ticker/btcusd/",
            vec!["last"],
        ),
        PriceFeed::new(
            "CoinGecko",
            "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd",
            vec!["bitcoin", "usd"],
        ),
        // Kraken returns { "result": { "XXBTZUSD": { "c": ["<last>", "<vol>"], ... } } }
        PriceFeed::new(
            "Kraken",
            "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD",
            vec!["result", "XXBTZUSD", "c"], // we'll take c[0] below
        ),
        PriceFeed::new(
            "Coinbase",
            "https://api.coinbase.com/v2/prices/spot?currency=USD",
            vec!["data", "amount"],
        ),
        PriceFeed::new(
            "Blockchain.com",
            "https://blockchain.info/ticker",
            vec!["USD", "last"],
        ),
    ]
}

pub fn fetch_prices(
    agent: &Agent,
    price_feeds: &[PriceFeed],
) -> Result<Vec<(String, f64)>, Box<dyn Error>> {
    let mut prices = Vec::new();

    'feeds: for price_feed in price_feeds {
        let url: String = price_feed
            .urlformat
            .replace("{currency_lc}", "usd")
            .replace("{currency}", "USD");

        let response = retry(Fixed::from_millis(300).take(3), || {
            match agent.get(&url).call() {
                Ok(resp) => {
                    if resp.status() >= 200 && resp.status() < 300 {
                        Ok(resp)
                    } else {
                        Err(format!("Received status code: {}", resp.status()))
                    }
                }
                Err(e) => Err(e.to_string()),
            }
        })
        .map_err(|e| -> Box<dyn Error> {
            Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        })?;

        let json: Value = response.into_json()?;
        let mut data = &json;

        for key in &price_feed.jsonpath {
            if let Some(inner_data) = data.get(key) {
                data = inner_data;
            } else {
                eprintln!("Key '{}' not found in the response from {}", key, price_feed.name);
                continue 'feeds;
            }
        }

        // If the value is an array (e.g., Kraken "c": ["<last>", "<vol>"]), take the first item.
        if let Some(arr) = data.as_array() {
            if let Some(first) = arr.get(0) {
                data = first;
            }
        }

        if let Some(price) = data.as_f64() {
            prices.push((price_feed.name.clone(), price));
        } else if let Some(price_str) = data.as_str() {
            if let Ok(price) = price_str.parse::<f64>() {
                prices.push((price_feed.name.clone(), price));
            } else {
                eprintln!("Invalid price format for {}: {}", price_feed.name, price_str);
            }
        } else {
            eprintln!("Price data not found or invalid format for {}", price_feed.name);
        }
    }

    if prices.is_empty() {
        return Err("No valid prices fetched.".into());
    }

    Ok(prices)
}

pub fn get_latest_price(agent: &Agent) -> Result<f64, Box<dyn Error>> {
    let price_feeds = set_price_feeds();
    let prices = fetch_prices(agent, &price_feeds)?;

    for (feed_name, price) in &prices {
        println!("{:<25} ${:>1.2}", feed_name, price);
    }

    let mut price_values: Vec<f64> = prices.iter().map(|(_, price)| *price).collect();
    price_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_price = if price_values.len() % 2 == 0 {
        (price_values[price_values.len() / 2 - 1] + price_values[price_values.len() / 2]) / 2.0
    } else {
        price_values[price_values.len() / 2]
    };

    println!("\nMedian BTC/USD price:     ${:.2}\n", median_price);
    Ok(median_price)
}
