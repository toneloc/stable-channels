//! Durable accepted/rejected trade-result delivery worker.

use std::time::Duration;

use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            crate::stable_manager::StableChannelManager::retry_pending_trade_responses(
                state.db.as_ref(),
                state.ldk_server.as_ref() as &dyn LdkServerCalls,
            )
            .await;
        }
    });
}
