use crate::constants::{
    PRICE_CACHE_REFRESH_SECS, PRICE_FETCH_MAX_RETRIES, PRICE_FETCH_RETRY_DELAY_MS,
};
use retry::{delay::Fixed, retry};
use serde_json::Value;
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ureq::Agent;

const MIN_PLAUSIBLE_BTC_USD_PRICE: f64 = 1_000.0;
const MAX_PLAUSIBLE_BTC_USD_PRICE: f64 = 10_000_000.0;
const MIN_AGREEING_PRICE_FEEDS: usize = 2;
const MAX_FEED_DEVIATION_RATIO: f64 = 0.05;
const MAX_MEDIAN_MOVE_RATIO: f64 = 0.10;
const MAX_TRUSTED_PRICE_AGE_SECS: u64 = 60;

lazy_static::lazy_static! {
    static ref PRICE_CACHE: Arc<Mutex<PriceCache>> = Arc::new(Mutex::new(PriceCache {
        price: 0.0,
        // checked_sub avoids a Windows panic when uptime < the offset.
        last_update: Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(Instant::now),
        updating: false,
        quarantined: false,
    }));
}

pub struct PriceCache {
    price: f64,
    last_update: Instant,
    updating: bool,
    quarantined: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PriceRefreshErrorKind {
    Transient,
    LargeMove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PriceRefreshError {
    kind: PriceRefreshErrorKind,
    message: String,
}

impl PriceRefreshError {
    fn transient(message: impl Into<String>) -> Self {
        Self {
            kind: PriceRefreshErrorKind::Transient,
            message: message.into(),
        }
    }

    fn large_move(message: impl Into<String>) -> Self {
        Self {
            kind: PriceRefreshErrorKind::LargeMove,
            message: message.into(),
        }
    }

    fn quarantines_price(&self) -> bool {
        self.kind == PriceRefreshErrorKind::LargeMove
    }
}

impl fmt::Display for PriceRefreshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PriceRefreshError {}

impl PriceCache {
    fn begin_refresh(&mut self) -> Result<(), String> {
        if self.updating {
            return Err("price refresh already in progress".to_string());
        }

        // A recent successful consensus remains valid while the next refresh is in flight.
        // Invalidating it here made every five-second network fetch briefly expose price=0.
        self.updating = true;
        Ok(())
    }

    fn finish_refresh(&mut self, result: &Result<f64, PriceRefreshError>) {
        self.updating = false;
        match result {
            Ok(price) => {
                self.price = *price;
                self.last_update = Instant::now();
                self.quarantined = false;
            }
            Err(error) if error.quarantines_price() => {
                // Multiple feeds agreeing on a >10% move is evidence that the old anchor may no
                // longer be safe for accounting. Keep it only as raw display history until a new
                // consensus is admitted after the anchor expires.
                self.quarantined = true;
            }
            Err(_) => {
                // A transport failure or temporary lack of consensus says nothing new about the
                // last successful price. Its age limit will expire it naturally.
            }
        }
    }
}

// Re-export from constants module
pub use crate::constants::{get_default_price_feeds, PriceFeedConfig as PriceFeed};

/// Get the raw cached price without triggering a network fetch.
/// Use this for non-blocking startup. Returns 0.0 if no price is cached.
pub fn get_cached_price_no_fetch() -> f64 {
    let cache = PRICE_CACHE.lock().unwrap();
    cache.price
}

/// Return the cached value only while it is recent enough for accounting and protocol decisions.
/// A stale value remains available through `get_cached_price_no_fetch` for display continuity.
pub fn get_fresh_cached_price_no_fetch() -> f64 {
    let cache = PRICE_CACHE.lock().unwrap();
    accounting_price_reference(cache.price, cache.last_update.elapsed(), cache.quarantined)
        .unwrap_or(0.0)
}

/// Set the cached price directly — for regtest/integration testing.
/// Bypasses all network price fetching.
pub fn set_cached_price(price: f64) {
    let mut cache = PRICE_CACHE.lock().unwrap();
    cache.price = price;
    cache.last_update = Instant::now();
    cache.updating = false;
    cache.quarantined = false;
}

/// Force one network refresh. Unlike `get_cached_price`, this never returns an old value as a
/// successful refresh, so background publishers only publish newly validated consensus. A recent
/// successful price remains readable during the fetch and after transient failures; a confirmed
/// large-move conflict quarantines it for accounting until a new consensus is accepted.
pub fn refresh_cached_price() -> Result<f64, String> {
    {
        let mut cache = PRICE_CACHE.lock().unwrap();
        cache.begin_refresh()?;
    }

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build();
    let result = get_latest_price_classified(&agent);

    let mut cache = PRICE_CACHE.lock().unwrap();
    cache.finish_refresh(&result);
    result.map_err(|error| error.to_string())
}

// Get cached price or fetch a new one if needed. On refresh failure this compatibility API returns
// the last known value; callers that require freshness must use `refresh_cached_price`.
pub fn get_cached_price() -> f64 {
    let should_update = {
        let cache = PRICE_CACHE.lock().unwrap();
        cache.last_update.elapsed() > Duration::from_secs(PRICE_CACHE_REFRESH_SECS)
            && !cache.updating
    };

    if should_update {
        if let Ok(new_price) = refresh_cached_price() {
            return new_price;
        }
    }

    let cache = PRICE_CACHE.lock().unwrap();
    cache.price
}

pub fn set_price_feeds() -> Vec<PriceFeed> {
    get_default_price_feeds()
}

/// ureq agent with bounded connect + overall timeouts so a hung/geo-blocked endpoint can't stall the caller.
pub fn bounded_agent() -> Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
}

fn is_plausible_price(price: f64) -> bool {
    price.is_finite()
        && (MIN_PLAUSIBLE_BTC_USD_PRICE..=MAX_PLAUSIBLE_BTC_USD_PRICE).contains(&price)
}

fn parse_price_value(value: &Value) -> Option<f64> {
    let price = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))?;
    is_plausible_price(price).then_some(price)
}

fn calculate_median_price(prices: &[(String, f64)]) -> Option<f64> {
    let mut price_values: Vec<f64> = prices
        .iter()
        .map(|(_, price)| *price)
        .filter(|price| is_plausible_price(*price))
        .collect();
    price_values.sort_by(f64::total_cmp);

    let midpoint = price_values.len() / 2;
    let median = if price_values.len().is_multiple_of(2) {
        let lower = *price_values.get(midpoint.checked_sub(1)?)?;
        let upper = *price_values.get(midpoint)?;
        lower + (upper - lower) / 2.0
    } else {
        *price_values.get(midpoint)?
    };
    is_plausible_price(median).then_some(median)
}

fn relative_deviation(value: f64, reference: f64) -> f64 {
    if !is_plausible_price(value) || !is_plausible_price(reference) {
        return f64::INFINITY;
    }
    (value - reference).abs() / reference
}

fn trusted_price_reference(price: f64, age: Duration) -> Option<f64> {
    (is_plausible_price(price) && age <= Duration::from_secs(MAX_TRUSTED_PRICE_AGE_SECS))
        .then_some(price)
}

fn accounting_price_reference(price: f64, age: Duration, quarantined: bool) -> Option<f64> {
    (!quarantined)
        .then(|| trusted_price_reference(price, age))
        .flatten()
}

fn validate_price_consensus(
    prices: &[(String, f64)],
    last_trusted_price: Option<f64>,
) -> Result<f64, PriceRefreshError> {
    let initial_median = calculate_median_price(prices)
        .ok_or_else(|| PriceRefreshError::transient("No plausible BTC/USD prices were returned"))?;
    let agreeing_prices: Vec<(String, f64)> = prices
        .iter()
        .filter(|(_, price)| {
            is_plausible_price(*price)
                && relative_deviation(*price, initial_median) <= MAX_FEED_DEVIATION_RATIO
        })
        .cloned()
        .collect();

    if agreeing_prices.len() < MIN_AGREEING_PRICE_FEEDS {
        return Err(PriceRefreshError::transient(format!(
            "Price consensus requires at least {MIN_AGREEING_PRICE_FEEDS} agreeing feeds; got {}",
            agreeing_prices.len()
        )));
    }

    let median = calculate_median_price(&agreeing_prices).ok_or_else(|| {
        PriceRefreshError::transient("Agreeing feeds did not produce a valid median")
    })?;
    if let Some(last_price) = last_trusted_price.filter(|price| is_plausible_price(*price)) {
        let deviation = relative_deviation(median, last_price);
        if deviation > MAX_MEDIAN_MOVE_RATIO {
            return Err(PriceRefreshError::large_move(format!(
                "BTC/USD median moved {:.2}% from the last trusted price, above the {:.2}% limit",
                deviation * 100.0,
                MAX_MEDIAN_MOVE_RATIO * 100.0
            )));
        }
    }

    Ok(median)
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

        let response = match retry(
            Fixed::from_millis(PRICE_FETCH_RETRY_DELAY_MS).take(PRICE_FETCH_MAX_RETRIES),
            || match agent.get(&url).call() {
                Ok(resp) => {
                    if resp.status() >= 200 && resp.status() < 300 {
                        Ok(resp)
                    } else {
                        Err(format!("Received status code: {}", resp.status()))
                    }
                }
                Err(e) => Err(e.to_string()),
            },
        ) {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("Feed {} unreachable: {}", price_feed.name, e);
                continue 'feeds;
            }
        };

        let json: Value = match response.into_json() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Feed {} returned unparseable JSON: {}", price_feed.name, e);
                continue 'feeds;
            }
        };
        let mut data = &json;

        for key in &price_feed.json_path {
            if let Some(inner_data) = data.get(key) {
                data = inner_data;
            } else {
                eprintln!(
                    "Key '{}' not found in the response from {}",
                    key, price_feed.name
                );
                continue 'feeds;
            }
        }

        // If the value is an array (e.g., Kraken "c": ["<last>", "<vol>"]), take the first item.
        if let Some(arr) = data.as_array() {
            if let Some(first) = arr.first() {
                data = first;
            }
        }

        if let Some(price) = parse_price_value(data) {
            prices.push((price_feed.name.clone(), price));
        } else {
            eprintln!(
                "Price data for {} is missing, malformed, or outside the plausibility band",
                price_feed.name
            );
        }
    }

    if prices.is_empty() {
        return Err("No valid prices fetched.".into());
    }

    Ok(prices)
}

fn get_latest_price_classified(agent: &Agent) -> Result<f64, PriceRefreshError> {
    let price_feeds = set_price_feeds();
    let prices = fetch_prices(agent, &price_feeds)
        .map_err(|error| PriceRefreshError::transient(error.to_string()))?;

    for (feed_name, price) in &prices {
        println!("{:<25} ${:>1.2}", feed_name, price);
    }

    let last_trusted_price = {
        let cache = PRICE_CACHE.lock().unwrap();
        trusted_price_reference(cache.price, cache.last_update.elapsed())
    };
    let median_price = validate_price_consensus(&prices, last_trusted_price).map_err(|error| {
        eprintln!("Rejected BTC/USD price update: {error}");
        error
    })?;

    println!("\nMedian BTC/USD price:     ${:.2}\n", median_price);
    Ok(median_price)
}

pub fn get_latest_price(agent: &Agent) -> Result<f64, Box<dyn Error>> {
    get_latest_price_classified(agent).map_err(|error| -> Box<dyn Error> { Box::new(error) })
}

/// Fetch daily OHLC data from Kraken
/// Returns Vec of (date_string, open, high, low, close, volume)
#[allow(clippy::type_complexity)]
pub fn fetch_kraken_ohlc(
    agent: &Agent,
    since_timestamp: Option<i64>,
) -> Result<Vec<(String, f64, f64, f64, f64, Option<f64>)>, Box<dyn Error>> {
    let mut url = "https://api.kraken.com/0/public/OHLC?pair=XBTUSD&interval=1440".to_string();
    if let Some(since) = since_timestamp {
        url = format!("{}&since={}", url, since);
    }

    let response = agent
        .get(&url)
        .call()
        .map_err(|e| -> Box<dyn Error> { Box::new(std::io::Error::other(e.to_string())) })?;

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

                            let open = arr[1]
                                .as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            let high = arr[2]
                                .as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            let low = arr[3]
                                .as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            let close = arr[4]
                                .as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
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
        .as_secs() as i64
        - 86400;

    let url = format!(
        "https://api.kraken.com/0/public/OHLC?pair=XBTUSD&interval=15&since={}",
        since
    );

    let response = agent
        .get(&url)
        .call()
        .map_err(|e| -> Box<dyn Error> { Box::new(std::io::Error::other(e.to_string())) })?;

    let json: Value = response.into_json()?;

    if let Some(errors) = json.get("error").and_then(|e| e.as_array()) {
        if !errors.is_empty() {
            return Err(format!("Kraken API error: {:?}", errors).into());
        }
    }

    let mut prices = Vec::new();

    if let Some(result) = json.get("result") {
        for (key, value) in result.as_object().unwrap_or(&serde_json::Map::new()) {
            if key == "last" {
                continue;
            }
            if let Some(ohlc_array) = value.as_array() {
                for candle in ohlc_array {
                    if let Some(arr) = candle.as_array() {
                        if arr.len() >= 5 {
                            let timestamp = arr[0].as_i64().unwrap_or(0);
                            let close = arr[4]
                                .as_str()
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
        let url = feed
            .url_format
            .replace("{currency_lc}", "usd")
            .replace("{currency}", "USD");
        assert_eq!(url, "https://example.com/usd/USD");
    }

    #[test]
    fn test_price_parser_rejects_non_finite_and_non_positive_values() {
        for value in [
            Value::String("NaN".to_string()),
            Value::String("inf".to_string()),
            Value::String("-inf".to_string()),
            Value::String("0".to_string()),
            Value::String("-1".to_string()),
            Value::String("999.99".to_string()),
            Value::String("10000000.01".to_string()),
            Value::Null,
        ] {
            assert_eq!(parse_price_value(&value), None);
        }

        assert_eq!(
            parse_price_value(&Value::String("50000.5".to_string())),
            Some(50000.5)
        );
    }

    #[test]
    fn test_median_filters_invalid_prices_without_panicking() {
        let prices = vec![
            ("nan".to_string(), f64::NAN),
            ("infinity".to_string(), f64::INFINITY),
            ("zero".to_string(), 0.0),
            ("negative".to_string(), -1.0),
            ("low".to_string(), 40_000.0),
            ("high".to_string(), 60_000.0),
        ];
        assert_eq!(calculate_median_price(&prices), Some(50_000.0));

        let invalid = vec![("nan".to_string(), f64::NAN)];
        assert_eq!(calculate_median_price(&invalid), None);
    }

    #[test]
    fn test_price_consensus_requires_two_agreeing_feeds() {
        let single = vec![("only".to_string(), 60_000.0)];
        assert!(validate_price_consensus(&single, None).is_err());

        let split = vec![
            ("low".to_string(), 40_000.0),
            ("high".to_string(), 80_000.0),
        ];
        assert!(validate_price_consensus(&split, None).is_err());
    }

    #[test]
    fn test_price_consensus_ignores_outlier() {
        let prices = vec![
            ("a".to_string(), 65_900.0),
            ("b".to_string(), 66_000.0),
            ("outlier".to_string(), 1_000_000.0),
        ];
        assert_eq!(validate_price_consensus(&prices, None), Ok(65_950.0));
    }

    #[test]
    fn test_price_consensus_rejects_large_move_from_last_trusted_price() {
        let prices = vec![("a".to_string(), 65_900.0), ("b".to_string(), 66_000.0)];
        let error = validate_price_consensus(&prices, Some(50_000.0)).unwrap_err();
        assert_eq!(error.kind, PriceRefreshErrorKind::LargeMove);
        assert_eq!(
            validate_price_consensus(&prices, Some(65_000.0)),
            Ok(65_950.0)
        );
    }

    #[test]
    fn large_move_anchor_expires_instead_of_wedging_forever() {
        let prices = vec![("a".to_string(), 65_900.0), ("b".to_string(), 66_000.0)];
        let recent =
            trusted_price_reference(50_000.0, Duration::from_secs(MAX_TRUSTED_PRICE_AGE_SECS));
        assert_eq!(recent, Some(50_000.0));
        assert!(validate_price_consensus(&prices, recent).is_err());

        let expired = trusted_price_reference(
            50_000.0,
            Duration::from_secs(MAX_TRUSTED_PRICE_AGE_SECS + 1),
        );
        assert_eq!(expired, None);
        assert_eq!(validate_price_consensus(&prices, expired), Ok(65_950.0));
    }

    #[test]
    fn refresh_in_progress_keeps_recent_consensus_available() {
        let mut cache = PriceCache {
            price: 65_000.0,
            last_update: Instant::now(),
            updating: false,
            quarantined: false,
        };

        cache.begin_refresh().unwrap();

        assert!(cache.updating);
        assert_eq!(
            accounting_price_reference(cache.price, cache.last_update.elapsed(), cache.quarantined,),
            Some(65_000.0)
        );
    }

    #[test]
    fn transient_refresh_failure_keeps_recent_consensus_until_expiry() {
        let mut cache = PriceCache {
            price: 65_000.0,
            last_update: Instant::now(),
            updating: true,
            quarantined: false,
        };
        let failure: Result<f64, PriceRefreshError> = Err(PriceRefreshError::transient(
            "feeds temporarily unavailable",
        ));

        cache.finish_refresh(&failure);

        assert!(!cache.updating);
        assert!(!cache.quarantined);
        assert_eq!(
            accounting_price_reference(cache.price, cache.last_update.elapsed(), cache.quarantined,),
            Some(65_000.0)
        );
        assert_eq!(
            accounting_price_reference(
                cache.price,
                Duration::from_secs(MAX_TRUSTED_PRICE_AGE_SECS + 1),
                cache.quarantined,
            ),
            None
        );
    }

    #[test]
    fn large_move_failure_quarantines_old_consensus_until_success() {
        let mut cache = PriceCache {
            price: 50_000.0,
            last_update: Instant::now(),
            updating: true,
            quarantined: false,
        };
        let rejection: Result<f64, PriceRefreshError> = Err(PriceRefreshError::large_move(
            "validated move exceeds limit",
        ));

        cache.finish_refresh(&rejection);

        assert!(cache.quarantined);
        assert_eq!(
            accounting_price_reference(cache.price, cache.last_update.elapsed(), cache.quarantined,),
            None
        );

        cache.begin_refresh().unwrap();
        cache.finish_refresh(&Ok(65_000.0));
        assert!(!cache.quarantined);
        assert_eq!(
            accounting_price_reference(cache.price, cache.last_update.elapsed(), cache.quarantined,),
            Some(65_000.0)
        );
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

    // Test for verifying one feed failure case: when a single feed is
    // unreachable, the remaining feeds must still be tried and their
    // prices returned.
    #[test]
    fn test_one_feed_failure_case() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = r#"{"last":"50000"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let feeds = vec![
            PriceFeed::new("Unreachable", "http://127.0.0.1:1/", vec!["last"]),
            PriceFeed::new("Mock", &format!("http://127.0.0.1:{}/", port), vec!["last"]),
        ];

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(2))
            .timeout(Duration::from_secs(2))
            .build();

        let result = fetch_prices(&agent, &feeds).expect("should get at least one price");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "Mock");
        assert!((result[0].1 - 50000.0).abs() < f64::EPSILON);
    }

    // Pins the contract for the total-outage case: all feeds down must
    // return Err, not Ok(vec![]).
    #[test]
    fn test_fetch_prices_all_unreachable_returns_err() {
        let feeds = vec![
            PriceFeed::new("Dead1", "http://127.0.0.1:1/", vec!["last"]),
            PriceFeed::new("Dead2", "http://127.0.0.1:2/", vec!["last"]),
        ];

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(2))
            .timeout(Duration::from_secs(2))
            .build();

        let err = fetch_prices(&agent, &feeds).expect_err("all feeds dead should be Err");
        assert!(err.to_string().contains("No valid prices"));
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
