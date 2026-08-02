//! Durable TRADE_V1 acceptance/rejection response retry loop.

use std::time::Duration;

use tokio::time::MissedTickBehavior;

use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            let worker_state = state.clone();
            match tokio::spawn(async move { run(worker_state).await }).await {
                Ok(()) => tracing::error!("trade response retry worker exited unexpectedly"),
                Err(error) => {
                    let error_message = error.to_string();
                    tracing::error!("trade response retry worker failed: {error_message}");
                    let _ = std::panic::catch_unwind(move || {
                        stable_channels::audit::audit_event(
                            "TRADE_RESPONSE_RETRY_WORKER_FAILED",
                            serde_json::json!({ "error": error_message }),
                        );
                    });
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

async fn run(state: AppState) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        crate::stable_manager::StableChannelManager::retry_pending_trade_responses(
            state.db.as_ref(),
            state.ldk_server.as_ref() as &dyn LdkServerCalls,
        )
        .await;
    }
}
