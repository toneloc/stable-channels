use ureq::Agent;
use serde_json::Value;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use retry::{retry, delay::Fixed};
use crate::audit::audit_event;
use crate::constants::{PRICE_CACHE_REFRESH_SECS, PRICE_FETCH_RETRY_DELAY_MS, PRICE_FETCH_MAX_RETRIES};
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

// Re-export from constants module
pub use crate::constants::{PriceFeedConfig as PriceFeed, get_default_price_feeds};

/// Get the raw cached price without triggering a network fetch.
/// Use this for non-blocking startup. Returns 0.0 if no price is cached.
pub fn get_cached_price_no_fetch() -> f64 {
    let cache = PRICE_CACHE.lock().unwrap();
    cache.price
}

// Get cached price or fetch a new one if needed
pub fn get_cached_price() -> f64 {
    let should_update = {
        let cache = PRICE_CACHE.lock().unwrap();
        cache.last_update.elapsed() > Duration::from_secs(PRICE_CACHE_REFRESH_SECS) && !cache.updating
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
    get_default_price_feeds()
}

pub fn fetch_prices(
    agent: &Agent,
    price_feeds: &[PriceFeed],
) -> Result<Vec<(String, f64)>, Box<dyn Error>> {
    let mut prices = Vec::new();

    'feeds: for price_feed in price_feeds {
        let url: String = price_feed
            .url_format
            .replace("{currency_lc}", "usd")
            .replace("{currency}", "USD");

        let response = retry(Fixed::from_millis(PRICE_FETCH_RETRY_DELAY_MS).take(PRICE_FETCH_MAX_RETRIES), || {
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

        for key in &price_feed.json_path {
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

/// Fetch daily OHLC data from Kraken
/// Returns Vec of (date_string, open, high, low, close, volume)
pub fn fetch_kraken_ohlc(agent: &Agent, since_timestamp: Option<i64>) -> Result<Vec<(String, f64, f64, f64, f64, Option<f64>)>, Box<dyn Error>> {
    let mut url = "https://api.kraken.com/0/public/OHLC?pair=XBTUSD&interval=1440".to_string();
    if let Some(since) = since_timestamp {
        url = format!("{}&since={}", url, since);
    }

    let response = agent.get(&url).call()
        .map_err(|e| -> Box<dyn Error> { Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())) })?;

    let json: Value = response.into_json()?;

    // Check for errors
    if let Some(errors) = json.get("error").and_then(|e| e.as_array()) {
        if !errors.is_empty() {
            return Err(format!("Kraken API error: {:?}", errors).into());
        }
    }

    let mut prices = Vec::new();

    // Get the OHLC data - it's under result.XXBTZUSD (or similar key)
    if let Some(result) = json.get("result") {
        // Find the OHLC array (key varies, but it's the array, not "last")
        for (key, value) in result.as_object().unwrap_or(&serde_json::Map::new()) {
            if key == "last" {
                continue;
            }
            if let Some(ohlc_array) = value.as_array() {
                for candle in ohlc_array {
                    if let Some(arr) = candle.as_array() {
                        // Kraken OHLC format: [time, open, high, low, close, vwap, volume, count]
                        if arr.len() >= 7 {
                            let timestamp = arr[0].as_i64().unwrap_or(0);
                            let date = chrono::DateTime::from_timestamp(timestamp, 0)
                                .map(|dt| dt.format("%Y-%m-%d").to_string())
                                .unwrap_or_default();

                            let open = arr[1].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                            let high = arr[2].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                            let low = arr[3].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                            let close = arr[4].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                            let volume = arr[6].as_str().and_then(|s| s.parse::<f64>().ok());

                            if !date.is_empty() && close > 0.0 {
                                prices.push((date, open, high, low, close, volume));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(prices)
}

/// Fetch intraday OHLC data from Kraken (15-minute candles, last 24 hours)
/// Returns Vec of (unix_timestamp, close_price)
pub fn fetch_kraken_intraday(agent: &Agent) -> Result<Vec<(i64, f64)>, Box<dyn Error>> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64 - 86400;

    let url = format!(
        "https://api.kraken.com/0/public/OHLC?pair=XBTUSD&interval=15&since={}",
        since
    );

    let response = agent.get(&url).call()
        .map_err(|e| -> Box<dyn Error> { Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())) })?;

    let json: Value = response.into_json()?;

    if let Some(errors) = json.get("error").and_then(|e| e.as_array()) {
        if !errors.is_empty() {
            return Err(format!("Kraken API error: {:?}", errors).into());
        }
    }

    let mut prices = Vec::new();

    if let Some(result) = json.get("result") {
        for (key, value) in result.as_object().unwrap_or(&serde_json::Map::new()) {
            if key == "last" { continue; }
            if let Some(ohlc_array) = value.as_array() {
                for candle in ohlc_array {
                    if let Some(arr) = candle.as_array() {
                        if arr.len() >= 5 {
                            let timestamp = arr[0].as_i64().unwrap_or(0);
                            let close = arr[4].as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            if timestamp > 0 && close > 0.0 {
                                prices.push((timestamp, close));
                            }
                        }
                    }
                }
            }
        }
    }

    prices.sort_by_key(|(ts, _)| *ts);
    Ok(prices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_price_feeds_returns_feeds() {
        let feeds = set_price_feeds();
        assert!(!feeds.is_empty());
        assert!(feeds.iter().any(|f| f.name == "Bitstamp"));
        assert!(feeds.iter().any(|f| f.name == "Coinbase"));
    }

    #[test]
    fn test_price_feed_url_format_substitution() {
        let feed = PriceFeed::new(
            "Test",
            "https://example.com/{currency_lc}/{currency}",
            vec!["price"],
        );
        let url = feed.url_format
            .replace("{currency_lc}", "usd")
            .replace("{currency}", "USD");
        assert_eq!(url, "https://example.com/usd/USD");
    }

    // Integration test (requires network)
    #[test]
    #[ignore] // Run with: cargo test -- --ignored
    fn test_fetch_prices_live() {
        let agent = Agent::new();
        let feeds = set_price_feeds();
        let result = fetch_prices(&agent, &feeds);
        assert!(result.is_ok());
        let prices = result.unwrap();
        assert!(!prices.is_empty());
        for (name, price) in &prices {
            assert!(*price > 0.0, "{} returned invalid price", name);
        }
    }

    #[test]
    #[ignore]
    fn test_get_latest_price_returns_median() {
        let agent = Agent::new();
        let result = get_latest_price(&agent);
        assert!(result.is_ok());
        let price = result.unwrap();
        assert!(price > 1000.0); // BTC should be > $1000
    }
}
