//! Long-running SubscribeEvents loop: connects to LDK Server's event stream, reconnects with exponential backoff, and dispatches each EventEnvelope to its handler.

use std::time::Duration;

use tracing::{info, warn};

use ldk_server_client::ldk_server_grpc::events::event_envelope::Event as EventVariant;
use ldk_server_client::ldk_server_grpc::events::{ChannelState, EventEnvelope};

use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

pub fn spawn(state: AppState) {
    tokio::spawn(async move { run(state).await });
}

async fn run(state: AppState) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut stream = match state.ldk_server.subscribe_events().await {
            Ok(s) => {
                backoff = Duration::from_secs(1);
                s
            },
            Err(e) => {
                warn!(
                    "[event_loop] subscribe_events failed: {}; retry in {:?}",
                    e, backoff
                );
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                continue;
            },
        };
        info!("[event_loop] subscribed");
        while let Some(item) = stream.next_message().await {
            dispatch(item, &state).await;
        }
        warn!("[event_loop] stream ended; reconnecting");
    }
}

async fn dispatch(
    item: Result<EventEnvelope, ldk_server_client::error::LdkServerError>,
    state: &AppState,
) {
    let envelope = match item {
        Ok(e) => e,
        Err(e) => {
            warn!("[event_loop] item error: {}", e);
            return;
        },
    };
    let btc_price = stable_channels::price_feeds::get_cached_price_no_fetch();
    let mut mgr = state.stable_manager.lock().await;
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    match envelope.event {
        Some(EventVariant::ChannelStateChanged(e)) => {
            if e.state == ChannelState::Ready as i32 {
                mgr.handle_channel_ready(
                    e.channel_id.clone(),
                    e.user_channel_id.clone(),
                    ldk,
                    btc_price,
                )
                .await;
            } else if e.state == ChannelState::Closed as i32 {
                mgr.handle_channel_closed(e.user_channel_id.clone());
            }
        },
        Some(EventVariant::PaymentReceived(e)) => {
            mgr.handle_payment_received(e.custom_records, ldk, btc_price).await;
        },
        Some(EventVariant::PaymentForwarded(e)) => {
            if let Some(fp) = e.forwarded_payment {
                mgr.handle_payment_forwarded(
                    fp.prev_user_channel_id,
                    fp.outbound_amount_forwarded_msat.unwrap_or(0),
                    fp.total_fee_earned_msat.unwrap_or(0),
                    ldk,
                    btc_price,
                )
                .await;
            }
        },
        _ => {},
    }
}
