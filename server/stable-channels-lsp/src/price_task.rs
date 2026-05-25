//! Background task that keeps the `stable_channels::price_feeds` cache warm.

use std::time::Duration;

use stable_channels::constants::PRICE_CACHE_REFRESH_SECS;
use stable_channels::price_feeds::get_cached_price;
use tokio::time::interval;
use tracing::{info, warn};

/// Drive the price cache forever.
pub async fn run() {
    let first = tokio::task::spawn_blocking(get_cached_price).await.unwrap_or(0.0);
    if first > 0.0 {
        info!("initial BTC/USD price = ${:.2}", first);
    } else {
        warn!("initial price fetch returned 0.0, exchanges may be unreachable");
    }

    let mut tick = interval(Duration::from_secs(PRICE_CACHE_REFRESH_SECS));
    tick.tick().await;
    loop {
        tick.tick().await;
        let price = tokio::task::spawn_blocking(get_cached_price).await.unwrap_or(0.0);
        if price > 0.0 {
            tracing::debug!("price refresh: ${:.2}", price);
        } else {
            warn!("price refresh returned 0.0");
        }
    }
}
