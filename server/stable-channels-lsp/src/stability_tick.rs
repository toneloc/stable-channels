//! Periodic stability tick: every STABILITY_CHECK_INTERVAL_SECS, run_tick detects USD drift and either sends a stability payment or wakes the offline peer.

use std::time::Duration;

use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    let interval_secs = stable_channels::constants::STABILITY_CHECK_INTERVAL_SECS;
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    info!("[stability_tick] running every {}s", interval_secs);
    loop {
        ticker.tick().await;
        let btc_price = stable_channels::price_feeds::get_cached_price_no_fetch();
        if btc_price <= 0.0 {
            warn!("[stability_tick] price cache cold; skipping");
            continue;
        }
        let mut mgr = state.stable_manager.lock().await;
        mgr.run_tick(
            state.ldk_server.as_ref() as &dyn LdkServerCalls,
            &state.push,
            btc_price,
        )
        .await;
    }
}
