use reqwest::blocking::Client;
use serde_json::Value;
use std::error::Error;
use retry::{retry, delay::Fixed};

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
        PriceFeed::new(
            "Coindesk",
            "https://api.coindesk.com/v1/bpi/currentprice/USD.json",
            vec!["bpi", "USD", "rate_float"],
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
    client: &Client,
    price_feeds: &[PriceFeed],
) -> Result<Vec<(String, f64)>, Box<dyn Error>> {
    let mut prices = Vec::new();

    for price_feed in price_feeds {
        let url: String = price_feed
            .urlformat
            .replace("{currency_lc}", "usd")
            .replace("{currency}", "USD");

        let response = retry(Fixed::from_millis(300).take(3), || {
            match client.get(&url).send() {
                Ok(resp) => {
                    if resp.status().is_success() {
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

        let json: Value = response.json()?;
        let mut data = &json;

        for key in &price_feed.jsonpath {
            if let Some(inner_data) = data.get(key) {
                data = inner_data;
            } else {
                println!(
                    "Key '{}' not found in the response from {}",
                    key, price_feed.name
                );
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
            println!(
                "Price data not found or invalid format for {}",
                price_feed.name
            );
        }
    }

    if prices.len() < 5 {
        println!("Fewer than 5 prices fetched.");
    }

    if prices.is_empty() {
        return Err("No valid prices fetched.".into());
    }

    Ok(prices)
}

pub fn calculate_median_price(
    prices: Vec<(String, f64)>,
) -> Result<f64, Box<dyn std::error::Error>> {
    // Print all prices
    for (feed_name, price) in &prices {
        println!("{:<25} ${:>1.2}", feed_name, price);
    }

    // Calculate the median price
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