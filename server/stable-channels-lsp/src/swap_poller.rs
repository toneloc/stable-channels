//! Background task that keeps onboarding-swap provider states fresh so `/status` can answer from
//! the store. Stops polling a row once the wallet reports settlement (OpenReceive rule: the
//! outcome of a settled attempt cannot change) or the row reaches a terminal state.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::interval;
use tracing::warn;

use crate::swap::OnboardingSwaps;

const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Attempts older than this are abandoned: a `completed` row the wallet never settled would
/// otherwise be polled forever (completed is deliberately non-terminal).
const POLL_HORIZON_SECS: u64 = 7 * 86_400;

pub fn spawn(swaps: Arc<OnboardingSwaps>) {
    tokio::spawn(async move {
        let mut tick = interval(POLL_INTERVAL);
        loop {
            tick.tick().await;
            poll_once(&swaps, now_secs()).await;
        }
    });
}

/// One sweep over every pollable row; returns how many rows were refreshed successfully.
pub async fn poll_once(swaps: &OnboardingSwaps, now: u64) -> usize {
    let rows = match swaps.store.pollable(now.saturating_sub(POLL_HORIZON_SECS)) {
        Ok(rows) => rows,
        Err(e) => {
            warn!("[swap] poller could not read store: {}", e);
            return 0;
        }
    };
    let mut refreshed = 0;
    for row in rows {
        match swaps.refresh(&row).await {
            Ok(_) => refreshed += 1,
            Err(e) => warn!("[swap] refresh {} failed: {}", row.payment_hash, e),
        }
    }
    refreshed
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
