use crate::constants::{
    get_fallback_usdt_price_feeds, get_usdt_usd_price_feeds, PRICE_CACHE_REFRESH_SECS,
    PRICE_FETCH_TIMEOUT_SECS, PRICE_FETCH_TOTAL_TIMEOUT_SECS,
};
use serde_json::Value;
use std::error::Error;
use std::fmt;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use ureq::Agent;

const MIN_PLAUSIBLE_BTC_USD_PRICE: f64 = 1_000.0;
const MAX_PLAUSIBLE_BTC_USD_PRICE: f64 = 10_000_000.0;
const MIN_AGREEING_PRICE_FEEDS: usize = 3;
const MAX_FEED_DEVIATION_RATIO: f64 = 0.01;
const MAX_MEDIAN_MOVE_RATIO: f64 = 0.10;
const MAX_TRUSTED_PRICE_AGE_SECS: u64 = 60;
const MAX_CONFIRM_DEVIATION_RATIO: f64 = 0.005;
const MAX_CANDIDATE_AGE_SECS: u64 = 120;
const MOVE_WINDOW_SECS: u64 = 600;
const STEADY_RUN_SECS: u64 = 300;
const STEADY_RUN_BAND_RATIO: f64 = 0.01;
const STEADY_RUN_MAX_GAP_SECS: u64 = 120;
const MIN_AGREEING_PEG_FEEDS: usize = 3;
const MIN_AGREEING_EXCHANGE_PEG_FEEDS: usize = 2;
const MAX_USDT_PEG_DEVIATION_FROM_DOLLAR: f64 = 0.005;
const MAX_PEG_FEED_DEVIATION_RATIO: f64 = 0.0025;
const AGGREGATOR_PEG_FEED_NAMES: [&str; 3] = [
    "CoinGecko USDT/USD",
    "CoinPaprika USDT/USD",
    "Coinlore USDT/USD",
];

lazy_static::lazy_static! {
    static ref PRICE_CACHE: Arc<Mutex<PriceCache>> =
        Arc::new(Mutex::new(PriceCache::empty(Instant::now())));
}

/// The last accepted price, plus what decides whether the next consensus is accepted.
pub struct PriceCache {
    price: f64,
    last_update: Instant,
    updating: bool,
    /// The latest consensus near the reference, which the next one must agree with to be used.
    candidate: Option<(f64, Instant)>,
    /// Prices accepted in the last `MOVE_WINDOW_SECS`, oldest first.
    accepted: VecDeque<(Instant, f64)>,
    /// Set while a big move pauses money pricing; it ends only through `admit`.
    paused: bool,
    /// Far answers that agree with each other while paused.
    steady_run: Option<SteadyRun>,
}

#[derive(Debug, Clone, Copy)]
struct SteadyRun {
    first_price: f64,
    started: Instant,
    last_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PriceRefreshErrorKind {
    Transient,
    LargeMove,
    Unconfirmed,
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

    fn unconfirmed(message: impl Into<String>) -> Self {
        Self {
            kind: PriceRefreshErrorKind::Unconfirmed,
            message: message.into(),
        }
    }
}

impl fmt::Display for PriceRefreshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PriceRefreshError {}

impl PriceCache {
    fn empty(now: Instant) -> Self {
        Self {
            price: 0.0,
            // checked_sub avoids a Windows panic when uptime < the offset.
            last_update: now.checked_sub(Duration::from_secs(10)).unwrap_or(now),
            updating: false,
            candidate: None,
            accepted: VecDeque::new(),
            paused: false,
            steady_run: None,
        }
    }

    fn seed(&mut self, price: f64, now: Instant) {
        *self = Self::empty(now);
        self.accept(price, now);
    }

    /// Admit one refresh's consensus; Ok only when the price is accepted for money.
    fn admit(
        &mut self,
        consensus: Result<f64, PriceRefreshError>,
        now: Instant,
    ) -> Result<f64, PriceRefreshError> {
        self.updating = false;
        // A failed refresh says nothing new: a pause and its steady run stay as they are.
        let price = consensus?;
        let window = Duration::from_secs(MOVE_WINDOW_SECS);
        while self.accepted.front().is_some_and(|(at, _)| now.saturating_duration_since(*at) > window) {
            self.accepted.pop_front();
        }
        let largest_move = self.largest_move_from_reference(price);
        if largest_move > MAX_MEDIAN_MOVE_RATIO {
            if !self.paused {
                self.paused = true;
                eprintln!("BTC/USD moved {:.2}% from a recent price; money pricing paused until a new price holds steady", largest_move * 100.0);
            }
            let run = match self.steady_run {
                Some(run)
                    if now.saturating_duration_since(run.last_at) <= Duration::from_secs(STEADY_RUN_MAX_GAP_SECS)
                        && relative_deviation(price, run.first_price) <= STEADY_RUN_BAND_RATIO =>
                {
                    SteadyRun { last_at: now, ..run }
                },
                _ => SteadyRun { first_price: price, started: now, last_at: now },
            };
            self.steady_run = Some(run);
            if now.saturating_duration_since(run.started) >= Duration::from_secs(STEADY_RUN_SECS) {
                // The new level held long enough to become the reference by itself.
                self.accepted.clear();
                self.accept(price, now);
                return Ok(price);
            }
            return Err(PriceRefreshError::large_move(format!(
                "BTC/USD moved {:.2}% from a price accepted in the last {} minutes, above the {:.2}% limit; waiting for a new price to hold steady",
                largest_move * 100.0,
                MOVE_WINDOW_SECS / 60,
                MAX_MEDIAN_MOVE_RATIO * 100.0
            )));
        }
        self.steady_run = None;
        let confirmed = self.candidate.is_some_and(|(candidate, at)| {
            now.saturating_duration_since(at) <= Duration::from_secs(MAX_CANDIDATE_AGE_SECS)
                && relative_deviation(price, candidate) <= MAX_CONFIRM_DEVIATION_RATIO
        });
        if !confirmed {
            self.candidate = Some((price, now));
            return Err(PriceRefreshError::unconfirmed(
                "BTC/USD consensus is waiting for the next refresh to confirm it",
            ));
        }
        self.accept(price, now);
        Ok(price)
    }

    /// The largest move from the prices accepted in the window, or from the last accepted price when none is that recent.
    fn largest_move_from_reference(&self, price: f64) -> f64 {
        let references: Vec<f64> = if self.accepted.is_empty() {
            Some(self.price).filter(|last| is_plausible_price(*last)).into_iter().collect()
        } else {
            self.accepted.iter().map(|(_, accepted)| *accepted).collect()
        };
        references.into_iter().map(|reference| relative_deviation(price, reference)).fold(0.0, f64::max)
    }

    fn accept(&mut self, price: f64, now: Instant) {
        self.price = price;
        self.last_update = now;
        self.accepted.push_back((now, price));
        self.candidate = Some((price, now));
        if self.paused {
            self.paused = false;
            eprintln!("BTC/USD price {price:.2} accepted; money pricing resumed");
        }
        self.steady_run = None;
    }

    /// The price money may use: none while paused, once the accepted price is over a minute old, or while a newer consensus away from it waits to be confirmed.
    fn accounting_price(&self, now: Instant) -> Option<f64> {
        let fresh = now.saturating_duration_since(self.last_update) <= Duration::from_secs(MAX_TRUSTED_PRICE_AGE_SECS);
        // A newer answer away from the accepted price means the market may have moved, so the accepted price is stale.
        let moved_away = self.candidate.is_some_and(|(candidate, at)| {
            at > self.last_update && relative_deviation(candidate, self.price) > MAX_CONFIRM_DEVIATION_RATIO
        });
        (!self.paused && fresh && !moved_away && is_plausible_price(self.price)).then_some(self.price)
    }

    #[cfg(test)]
    fn is_paused(&self) -> bool {
        self.paused
    }

    fn begin_refresh(&mut self) -> Result<(), String> {
        if self.updating {
            return Err("price refresh already in progress".to_string());
        }

        // A recent successful consensus remains valid while the next refresh is in flight.
        // Invalidating it here made every five-second network fetch briefly expose price=0.
        self.updating = true;
        Ok(())
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
    cache.accounting_price(Instant::now()).unwrap_or(0.0)
}

/// Set the cached price directly — for regtest/integration testing.
/// Bypasses all network price fetching.
pub fn set_cached_price(price: f64) {
    let mut cache = PRICE_CACHE.lock().unwrap();
    cache.seed(price, Instant::now());
}

/// Force one network refresh. Unlike `get_cached_price`, this never returns an old value as a
/// successful refresh: Ok only for a consensus the next refresh confirmed, or a new level that held
/// steady after a big move paused pricing. A recent accepted price remains readable during the
/// fetch and after transient failures; a big move pauses it for accounting.
pub fn refresh_cached_price() -> Result<f64, String> {
    {
        let mut cache = PRICE_CACHE.lock().unwrap();
        cache.begin_refresh()?;
    }

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(PRICE_FETCH_TIMEOUT_SECS))
        .timeout(Duration::from_secs(PRICE_FETCH_TOTAL_TIMEOUT_SECS))
        .build();
    let result = get_latest_price_classified(&agent);

    let mut cache = PRICE_CACHE.lock().unwrap();
    cache.admit(result, Instant::now()).map_err(|error| error.to_string())
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
        .timeout_connect(Duration::from_secs(PRICE_FETCH_TIMEOUT_SECS))
        .timeout(Duration::from_secs(PRICE_FETCH_TOTAL_TIMEOUT_SECS))
        .build()
}

fn is_plausible_price(price: f64) -> bool {
    price.is_finite()
        && (MIN_PLAUSIBLE_BTC_USD_PRICE..=MAX_PLAUSIBLE_BTC_USD_PRICE).contains(&price)
}

fn parse_numeric_value(value: &Value) -> Option<f64> {
    let price = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))?;
    (price.is_finite() && price > 0.0).then_some(price)
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

/// One refresh's consensus; whether money may use it is the cache's decision (`PriceCache::admit`).
fn validate_price_consensus(prices: &[(String, f64)]) -> Result<f64, PriceRefreshError> {
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
    // Dropping outliers moves the median, so count again around the price actually returned.
    let plausible = prices.iter().filter(|(_, price)| is_plausible_price(*price)).count();
    let supporters = prices
        .iter()
        .filter(|(_, price)| relative_deviation(*price, median) <= MAX_FEED_DEVIATION_RATIO)
        .count();
    if supporters < MIN_AGREEING_PRICE_FEEDS || supporters * 2 <= plausible {
        return Err(PriceRefreshError::transient(format!(
            "Price consensus needs at least {MIN_AGREEING_PRICE_FEEDS} feeds and more than half of the {plausible} answers near the median; got {supporters}"
        )));
    }

    Ok(median)
}

fn validate_usdt_peg(prices: &[(String, f64)]) -> Result<f64, PriceRefreshError> {
    // Find the market consensus before checking whether USDT is near $1. Filtering by the
    // desired answer first could discard live exchange evidence of a depeg and leave only
    // slower aggregators reporting a stale $1 value.
    let plausible: Vec<(String, f64)> = prices
        .iter()
        .filter(|(_, price)| price.is_finite() && *price > 0.0)
        .cloned()
        .collect();
    let exchange_prices: Vec<(String, f64)> = plausible
        .iter()
        .filter(|(name, _)| !AGGREGATOR_PEG_FEED_NAMES.contains(&name.as_str()))
        .cloned()
        .collect();
    let mut exchange_values: Vec<f64> = exchange_prices.iter().map(|(_, price)| *price).collect();
    exchange_values.sort_by(f64::total_cmp);
    let initial_exchange_median = median_values(&exchange_values).ok_or_else(|| {
        PriceRefreshError::transient("No valid exchange USDT/USD peg prices returned")
    })?;
    let agreeing_exchange_prices: Vec<(String, f64)> = exchange_prices
        .into_iter()
        .filter(|(_, price)| {
            ((*price - initial_exchange_median).abs() / initial_exchange_median)
                <= MAX_PEG_FEED_DEVIATION_RATIO
        })
        .collect();
    if agreeing_exchange_prices.len() < MIN_AGREEING_EXCHANGE_PEG_FEEDS {
        return Err(PriceRefreshError::transient(format!(
            "USDT fallback requires at least {MIN_AGREEING_EXCHANGE_PEG_FEEDS} agreeing exchange peg feeds; got {}",
            agreeing_exchange_prices.len()
        )));
    }

    let mut agreeing_exchange_values: Vec<f64> = agreeing_exchange_prices
        .iter()
        .map(|(_, price)| *price)
        .collect();
    agreeing_exchange_values.sort_by(f64::total_cmp);
    let accepted_exchange_median = median_values(&agreeing_exchange_values).ok_or_else(|| {
        PriceRefreshError::transient("Agreeing exchange USDT/USD feeds produced no median")
    })?;
    if (accepted_exchange_median - 1.0).abs() > MAX_USDT_PEG_DEVIATION_FROM_DOLLAR {
        return Err(PriceRefreshError::transient(format!(
            "Exchange USDT/USD consensus {accepted_exchange_median:.6} is outside the allowed peg band"
        )));
    }

    // Aggregators may confirm the exchange-anchored result, but never select or move it.
    let agreeing: Vec<(String, f64)> = plausible
        .into_iter()
        .filter(|(_, price)| {
            ((*price - accepted_exchange_median).abs() / accepted_exchange_median)
                <= MAX_PEG_FEED_DEVIATION_RATIO
        })
        .collect();
    if agreeing.len() < MIN_AGREEING_PEG_FEEDS {
        return Err(PriceRefreshError::transient(format!(
            "USDT fallback requires at least {MIN_AGREEING_PEG_FEEDS} agreeing peg feeds; got {}",
            agreeing.len()
        )));
    }
    Ok(accepted_exchange_median)
}

fn median_values(sorted_values: &[f64]) -> Option<f64> {
    let midpoint = sorted_values.len() / 2;
    if sorted_values.len().is_multiple_of(2) {
        let lower = *sorted_values.get(midpoint.checked_sub(1)?)?;
        let upper = *sorted_values.get(midpoint)?;
        Some(lower + (upper - lower) / 2.0)
    } else {
        sorted_values.get(midpoint).copied()
    }
}

fn fetch_single_price(agent: &Agent, price_feed: &PriceFeed) -> Option<f64> {
    let url = price_feed
        .url_format
        .replace("{currency_lc}", "usd")
        .replace("{currency}", "USD");

    let response = match agent.get(&url).call() {
        Ok(response) if (200..300).contains(&response.status()) => response,
        Ok(response) => {
            eprintln!(
                "Feed {} failed: HTTP {}",
                price_feed.name,
                response.status()
            );
            return None;
        }
        Err(error) => {
            eprintln!("Feed {} unreachable: {}", price_feed.name, error);
            return None;
        }
    };
    let json: Value = match response.into_json() {
        Ok(json) => json,
        Err(error) => {
            eprintln!(
                "Feed {} returned unparseable JSON: {}",
                price_feed.name, error
            );
            return None;
        }
    };
    let mut data = &json;
    for key in &price_feed.json_path {
        let next = if let Ok(index) = key.parse::<usize>() {
            data.as_array().and_then(|array| array.get(index))
        } else {
            data.get(key)
        };
        let Some(next) = next else {
            eprintln!(
                "Key '{}' not found in response from {}",
                key, price_feed.name
            );
            return None;
        };
        data = next;
    }

    if let Some(first) = data.as_array().and_then(|array| array.first()) {
        data = first;
    }
    let price = parse_numeric_value(data);
    if price.is_some() {
        println!("Price feed {} succeeded", price_feed.name);
    } else {
        eprintln!("Price data for {} is missing or malformed", price_feed.name);
    }
    price
}

pub fn fetch_prices(
    agent: &Agent,
    price_feeds: &[PriceFeed],
) -> Result<Vec<(String, f64)>, Box<dyn Error>> {
    let prices = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for price_feed in price_feeds.iter().cloned() {
            let agent = agent.clone();
            let prices = &prices;
            scope.spawn(move || {
                if let Some(price) = fetch_single_price(&agent, &price_feed) {
                    prices.lock().unwrap().push((price_feed.name, price));
                }
            });
        }
    });
    let mut prices = prices.into_inner().unwrap();
    prices.sort_by(|left, right| left.0.cmp(&right.0));

    if prices.is_empty() {
        return Err("No valid prices fetched.".into());
    }

    Ok(prices)
}

/// One refresh's consensus from the USD feeds, else the peg-normalized USDT feeds; the cache decides whether to use it.
fn get_latest_price_classified(agent: &Agent) -> Result<f64, PriceRefreshError> {
    let usd_prices = fetch_prices(agent, &get_default_price_feeds()).unwrap_or_default();
    match validate_price_consensus(&usd_prices) {
        Ok(price) => {
            println!(
                "Direct USD consensus from {} feeds",
                usd_prices.len()
            );
            return Ok(price);
        }
        Err(error) => eprintln!("Direct USD consensus unavailable: {error}; trying USDT fallback"),
    }

    let usdt_feeds = get_fallback_usdt_price_feeds();
    let peg_feeds = get_usdt_usd_price_feeds();
    let usdt_names: std::collections::HashSet<String> =
        usdt_feeds.iter().map(|feed| feed.name.clone()).collect();
    let peg_names: std::collections::HashSet<String> =
        peg_feeds.iter().map(|feed| feed.name.clone()).collect();
    let mut fallback_feeds = usdt_feeds;
    fallback_feeds.extend(peg_feeds);
    let fallback_prices = fetch_prices(agent, &fallback_feeds).unwrap_or_default();
    let usdt_prices: Vec<(String, f64)> = fallback_prices
        .iter()
        .filter(|(name, _)| usdt_names.contains(name))
        .cloned()
        .collect();
    let peg_prices: Vec<(String, f64)> = fallback_prices
        .iter()
        .filter(|(name, _)| peg_names.contains(name))
        .cloned()
        .collect();
    let peg = validate_usdt_peg(&peg_prices)?;
    let normalized_prices: Vec<(String, f64)> = usdt_prices
        .into_iter()
        .map(|(name, price)| (name, price * peg))
        .collect();
    let price = validate_price_consensus(&normalized_prices)?;
    println!(
        "Peg-normalized USDT consensus from {} feeds (USDT/USD={peg:.6})",
        normalized_prices.len()
    );
    Ok(price)
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
    fn test_numeric_parser_rejects_non_finite_and_non_positive_values() {
        for value in [
            Value::String("NaN".to_string()),
            Value::String("inf".to_string()),
            Value::String("-inf".to_string()),
            Value::String("0".to_string()),
            Value::String("-1".to_string()),
            Value::Null,
        ] {
            assert_eq!(parse_numeric_value(&value), None);
        }

        assert_eq!(
            parse_numeric_value(&Value::String("50000.5".to_string())),
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
    fn test_price_consensus_requires_three_agreeing_feeds() {
        let single = vec![("only".to_string(), 60_000.0)];
        assert!(validate_price_consensus(&single).is_err());

        let two = vec![
            ("low".to_string(), 40_000.0),
            ("nearby".to_string(), 40_100.0),
        ];
        assert!(validate_price_consensus(&two).is_err());
    }

    #[test]
    fn test_price_consensus_ignores_outlier() {
        let prices = vec![
            ("a".to_string(), 65_900.0),
            ("b".to_string(), 66_000.0),
            ("c".to_string(), 66_100.0),
            ("outlier".to_string(), 1_000_000.0),
        ];
        assert_eq!(validate_price_consensus(&prices), Ok(66_000.0));
    }

    #[test]
    fn consensus_needs_three_feeds_near_the_final_price() {
        // A stale quote and an outlier leave only two feeds near the recomputed median.
        let prices = vec![
            ("Gemini".to_string(), 55_000.0),
            ("Coinbase".to_string(), 60_000.0),
            ("Bitstamp".to_string(), 64_000.0),
            ("Kraken".to_string(), 65_000.0),
        ];
        assert!(validate_price_consensus(&prices).is_err());
    }

    #[test]
    fn consensus_needs_more_than_half_of_the_answers() {
        let split: Vec<(String, f64)> = [60_000.0, 60_010.0, 60_020.0, 66_000.0, 66_010.0, 66_020.0]
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("f{i}"), *p))
            .collect();
        assert!(validate_price_consensus(&split).is_err(), "three against three backs no price");
        let majority: Vec<(String, f64)> = [60_000.0, 60_010.0, 60_020.0, 60_030.0, 66_000.0, 66_010.0]
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("f{i}"), *p))
            .collect();
        assert_eq!(validate_price_consensus(&majority), Ok(60_015.0));
    }

    #[test]
    fn consensus_needs_a_majority_even_when_three_agree() {
        let answers = |quotes: &[f64]| -> Vec<(String, f64)> {
            quotes.iter().enumerate().map(|(i, p)| (format!("f{i}"), *p)).collect()
        };
        let half = answers(&[50_000.0, 60_000.0, 60_010.0, 60_020.0, 61_500.0, 70_000.0]);
        assert!(validate_price_consensus(&half).is_err(), "three of six is only half");
        let minority = answers(&[50_000.0, 59_000.0, 60_000.0, 60_010.0, 60_020.0, 61_500.0, 70_000.0]);
        assert!(validate_price_consensus(&minority).is_err(), "three of seven is a minority");
        let majority = answers(&[50_000.0, 60_000.0, 60_010.0, 60_020.0, 60_030.0, 61_500.0, 70_000.0]);
        assert_eq!(validate_price_consensus(&majority), Ok(60_015.0));
    }

    #[test]
    fn half_of_the_answers_cannot_pull_the_price_past_the_band() {
        for wrong in [59_500.0, 59_000.0, 58_000.0, 57_000.0, 54_000.0] {
            let prices = vec![
                ("h1".to_string(), 60_000.0),
                ("h2".to_string(), 60_000.0),
                ("w1".to_string(), wrong),
                ("w2".to_string(), wrong),
            ];
            if let Ok(price) = validate_price_consensus(&prices) {
                assert!(relative_deviation(price, 60_000.0) <= 0.01, "{wrong} pulled the price to {price}");
            }
        }
    }

    #[test]
    fn usdt_peg_requires_three_agreeing_sources_near_one_dollar() {
        let valid = vec![
            ("a".to_string(), 0.9990),
            ("b".to_string(), 0.9991),
            ("c".to_string(), 0.9989),
        ];
        assert_eq!(validate_usdt_peg(&valid), Ok(0.9990));

        let depegged = vec![
            ("a".to_string(), 0.98),
            ("b".to_string(), 0.981),
            ("c".to_string(), 0.979),
        ];
        assert!(validate_usdt_peg(&depegged).is_err());
        assert!(validate_usdt_peg(&valid[..2]).is_err());
    }

    #[test]
    fn usdt_peg_rejects_aggregator_only_consensus() {
        let aggregators = vec![
            ("CoinGecko USDT/USD".to_string(), 1.0000),
            ("CoinPaprika USDT/USD".to_string(), 1.0001),
            ("Coinlore USDT/USD".to_string(), 0.9999),
        ];

        assert!(validate_usdt_peg(&aggregators).is_err());
    }

    #[test]
    fn usdt_peg_rejects_stale_aggregators_during_exchange_depeg() {
        let prices = vec![
            ("Coinbase USDT/USD".to_string(), 0.9800),
            ("Kraken USDT/USD".to_string(), 0.9801),
            ("Bitstamp USDT/USD".to_string(), 0.9799),
            ("Bitfinex USDT/USD".to_string(), 0.9800),
            ("Crypto.com USDT/USD".to_string(), 0.9802),
            ("OKX USDT/USD".to_string(), 0.9798),
            ("CoinGecko USDT/USD".to_string(), 1.0000),
            ("CoinPaprika USDT/USD".to_string(), 1.0001),
            ("Coinlore USDT/USD".to_string(), 0.9999),
        ];

        assert!(validate_usdt_peg(&prices).is_err());
    }

    #[test]
    fn aggregators_cannot_outvote_exchange_peg_consensus() {
        let prices = vec![
            ("Coinbase USDT/USD".to_string(), 0.9800),
            ("Kraken USDT/USD".to_string(), 0.9801),
            ("Bitstamp USDT/USD".to_string(), 0.9799),
            ("Bitfinex USDT/USD".to_string(), 1.0000),
            ("Crypto.com USDT/USD".to_string(), 1.0001),
            ("CoinGecko USDT/USD".to_string(), 1.0000),
            ("CoinPaprika USDT/USD".to_string(), 1.0001),
            ("Coinlore USDT/USD".to_string(), 0.9999),
        ];

        assert!(validate_usdt_peg(&prices).is_err());
    }

    #[test]
    fn usdt_peg_accepts_two_exchanges_with_aggregator_confirmation() {
        let prices = vec![
            ("Crypto.com USDT/USD".to_string(), 0.9990),
            ("OKX USDT/USD".to_string(), 0.9991),
            ("CoinGecko USDT/USD".to_string(), 0.9989),
        ];

        assert_eq!(validate_usdt_peg(&prices), Ok(0.99905));
    }

    #[test]
    fn aggregator_peg_classification_matches_configured_feeds() {
        let configured: std::collections::HashSet<String> = get_usdt_usd_price_feeds()
            .into_iter()
            .map(|feed| feed.name)
            .collect();

        for aggregator in AGGREGATOR_PEG_FEED_NAMES {
            assert!(configured.contains(aggregator));
        }
    }

    #[test]
    fn normalized_usdt_consensus_uses_measured_peg() {
        let peg = 0.999;
        let prices = vec![
            ("a".to_string(), 64_064.0 * peg),
            ("b".to_string(), 64_074.0 * peg),
            ("c".to_string(), 64_054.0 * peg),
        ];
        assert!(
            (validate_price_consensus(&prices).unwrap() - 63_999.936).abs() < 1.0
        );
    }

    fn at(t0: Instant, secs: u64) -> Instant {
        t0 + Duration::from_secs(secs)
    }

    #[test]
    fn refresh_in_progress_keeps_recent_consensus_available() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(65_000.0, t0);

        cache.begin_refresh().unwrap();

        assert!(cache.updating);
        assert_eq!(cache.accounting_price(t0), Some(65_000.0));
    }

    #[test]
    fn transient_refresh_failure_keeps_recent_consensus_until_expiry() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(65_000.0, t0);
        cache.begin_refresh().unwrap();

        let failure = cache.admit(Err(PriceRefreshError::transient("feeds temporarily unavailable")), at(t0, 5));

        assert!(failure.is_err());
        assert!(!cache.updating);
        assert!(!cache.is_paused());
        assert_eq!(cache.accounting_price(at(t0, 5)), Some(65_000.0));
        assert_eq!(cache.accounting_price(at(t0, MAX_TRUSTED_PRICE_AGE_SECS + 1)), None);
    }

    #[test]
    fn a_single_consensus_is_not_used_for_money() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        assert_eq!(cache.admit(Ok(60_000.0), t0).unwrap_err().kind, PriceRefreshErrorKind::Unconfirmed);
        assert_eq!(cache.accounting_price(t0), None);
        assert_eq!(cache.admit(Ok(60_200.0), at(t0, 15)), Ok(60_200.0));
        assert_eq!(cache.accounting_price(at(t0, 15)), Some(60_200.0));
    }

    #[test]
    fn consecutive_answers_further_apart_than_half_a_percent_are_not_used() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        assert!(cache.admit(Ok(60_000.0), t0).is_err());
        assert_eq!(cache.admit(Ok(60_400.0), at(t0, 15)).unwrap_err().kind, PriceRefreshErrorKind::Unconfirmed);
        assert_eq!(cache.admit(Ok(60_500.0), at(t0, 30)), Ok(60_500.0));
    }

    #[test]
    fn a_candidate_older_than_two_minutes_does_not_confirm() {
        let t0 = Instant::now();
        let mut desktop = PriceCache::empty(t0);
        assert!(desktop.admit(Ok(60_000.0), t0).is_err());
        assert_eq!(desktop.admit(Ok(60_010.0), at(t0, 38)), Ok(60_010.0), "the desktop's cadence confirms");
        let mut late = PriceCache::empty(t0);
        assert!(late.admit(Ok(60_000.0), t0).is_err());
        assert_eq!(late.admit(Ok(60_010.0), at(t0, 121)).unwrap_err().kind, PriceRefreshErrorKind::Unconfirmed);
    }

    #[test]
    fn a_price_the_market_has_moved_away_from_is_not_used_for_money() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(60_000.0, t0);
        // A fast 3% fall waits for confirmation, and the old price is not used meanwhile.
        assert_eq!(cache.admit(Ok(58_200.0), at(t0, 15)).unwrap_err().kind, PriceRefreshErrorKind::Unconfirmed);
        assert_eq!(cache.accounting_price(at(t0, 15)), None);
        assert_eq!(cache.admit(Ok(58_250.0), at(t0, 30)), Ok(58_250.0));
        assert_eq!(cache.accounting_price(at(t0, 30)), Some(58_250.0));
        // One odd answer that the next contradicts leaves the accepted price in use again.
        assert!(cache.admit(Ok(59_000.0), at(t0, 45)).is_err());
        assert_eq!(cache.accounting_price(at(t0, 45)), None);
        assert!(cache.admit(Ok(58_260.0), at(t0, 60)).is_err());
        assert_eq!(cache.accounting_price(at(t0, 60)), Some(58_250.0));
    }

    #[test]
    fn seeding_gives_a_price_that_the_next_close_answer_confirms() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(60_000.0, t0);
        assert_eq!(cache.accounting_price(t0), Some(60_000.0));
        assert_eq!(cache.admit(Ok(60_100.0), at(t0, 15)), Ok(60_100.0));
    }

    #[test]
    fn steps_under_ten_percent_are_caught_once_they_add_up() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(60_000.0, t0);
        // Steps minutes apart still add up within the 10-minute window.
        assert!(cache.admit(Ok(65_000.0), at(t0, 150)).is_err());
        assert_eq!(cache.admit(Ok(65_000.0), at(t0, 165)), Ok(65_000.0));
        assert_eq!(cache.admit(Ok(70_000.0), at(t0, 300)).unwrap_err().kind, PriceRefreshErrorKind::LargeMove);
        assert!(cache.is_paused());
        assert_eq!(cache.accounting_price(at(t0, 300)), None);
    }

    #[test]
    fn the_pause_does_not_expire_with_the_price_age() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(50_000.0, t0);
        assert_eq!(cache.admit(Ok(66_000.0), at(t0, 15)).unwrap_err().kind, PriceRefreshErrorKind::LargeMove);
        // Long after the old price aged out and left the window, a far answer is still refused.
        assert_eq!(cache.admit(Ok(66_000.0), at(t0, 900)).unwrap_err().kind, PriceRefreshErrorKind::LargeMove);
        assert!(cache.is_paused());
        assert_eq!(cache.accounting_price(at(t0, 900)), None);
    }

    #[test]
    fn leaving_a_pause_takes_five_steady_minutes() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(50_000.0, t0);
        for secs in (15..315).step_by(15) {
            assert_eq!(
                cache.admit(Ok(66_000.0), at(t0, secs)).unwrap_err().kind,
                PriceRefreshErrorKind::LargeMove,
                "at {secs}s"
            );
        }
        assert_eq!(cache.admit(Ok(66_100.0), at(t0, 315)), Ok(66_100.0));
        assert!(!cache.is_paused());
        assert_eq!(cache.accounting_price(at(t0, 315)), Some(66_100.0));
        // The new level is the reference now.
        assert_eq!(cache.admit(Ok(66_150.0), at(t0, 330)), Ok(66_150.0));
    }

    #[test]
    fn a_jump_or_a_gap_restarts_the_steady_run() {
        let t0 = Instant::now();
        let mut jump = PriceCache::empty(t0);
        jump.seed(50_000.0, t0);
        for secs in (15..=150).step_by(15) {
            assert!(jump.admit(Ok(66_000.0), at(t0, secs)).is_err());
        }
        // 1.5% off the run's first answer starts a new run at 165 s.
        for secs in (165..465).step_by(15) {
            assert!(jump.admit(Ok(67_000.0), at(t0, secs)).is_err(), "at {secs}s");
        }
        assert_eq!(jump.admit(Ok(67_000.0), at(t0, 465)), Ok(67_000.0));

        let mut gap = PriceCache::empty(t0);
        gap.seed(50_000.0, t0);
        assert!(gap.admit(Ok(66_000.0), at(t0, 15)).is_err());
        // 121 s without an answer starts a new run at 136 s.
        for secs in (136..436).step_by(15) {
            assert!(gap.admit(Ok(66_000.0), at(t0, secs)).is_err(), "at {secs}s");
        }
        assert_eq!(gap.admit(Ok(66_000.0), at(t0, 436)), Ok(66_000.0));
    }

    #[test]
    fn a_price_back_near_the_old_one_ends_the_pause() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(60_000.0, t0);
        assert!(cache.admit(Ok(70_000.0), at(t0, 15)).is_err());
        assert!(cache.is_paused());
        // Far answers feed only the steady run, so the seeded price still confirms this one.
        assert_eq!(cache.admit(Ok(60_100.0), at(t0, 30)), Ok(60_100.0));
        assert!(!cache.is_paused());
    }

    #[test]
    fn a_failed_refresh_leaves_a_pause_and_its_run_alone() {
        let t0 = Instant::now();
        let mut cache = PriceCache::empty(t0);
        cache.seed(50_000.0, t0);
        for secs in (15..315).step_by(15) {
            let answer = if secs % 60 == 0 { Err(PriceRefreshError::transient("feeds down")) } else { Ok(66_000.0) };
            assert!(cache.admit(answer, at(t0, secs)).is_err(), "at {secs}s");
            assert!(cache.is_paused(), "at {secs}s");
        }
        assert_eq!(cache.admit(Ok(66_000.0), at(t0, 315)), Ok(66_000.0));
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
        let result = get_latest_price_classified(&agent);
        assert!(result.is_ok());
        let price = result.unwrap();
        assert!(price > 1000.0); // BTC should be > $1000
    }
}
