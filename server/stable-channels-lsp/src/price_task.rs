//! Background task that keeps the `stable_channels::price_feeds` cache warm and publishes each fresh price on a watch channel so startup can wait for the first non-zero value.

use std::time::Duration;

use stable_channels::constants::PRICE_CACHE_REFRESH_SECS;
use stable_channels::price_feeds::refresh_cached_price;
use tokio::sync::watch;
use tokio::time::interval;
use tracing::{info, warn};

/// Drive the price cache forever, publishing only freshly validated prices on `price_tx`.
pub async fn run(price_tx: watch::Sender<f64>) {
    match tokio::task::spawn_blocking(refresh_cached_price).await {
        Ok(Ok(first)) => {
            info!("initial BTC/USD price = ${:.2}", first);
            let _ = price_tx.send(first);
        }
        Ok(Err(error)) => warn!("initial price refresh failed: {}", error),
        Err(error) => warn!("initial price refresh task failed: {}", error),
    }

    let mut tick = interval(Duration::from_secs(PRICE_CACHE_REFRESH_SECS));
    tick.tick().await;
    loop {
        tick.tick().await;
        match tokio::task::spawn_blocking(refresh_cached_price).await {
            Ok(Ok(price)) => {
                let _ = price_tx.send(price);
                tracing::debug!("price refresh: ${:.2}", price);
            }
            Ok(Err(error)) => warn!("price refresh rejected: {}", error),
            Err(error) => warn!("price refresh task failed: {}", error),
        }
    }
}
