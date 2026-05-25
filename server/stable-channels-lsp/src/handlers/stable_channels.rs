//! ListStableChannels handler. Reads from the SC daemon's sqlite store.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use sc_protos::stable::{
    ListStableChannelsRequest, ListStableChannelsResponse, StableChannelInfo,
};
use stable_channels::price_feeds::get_cached_price_no_fetch;
use tracing::warn;

use crate::handlers::{decode_body, error_response, ok_response};
use crate::state::AppState;
use ldk_server_client::ldk_server_grpc::error::ErrorCode;

pub async fn list_stable_channels(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    if let Err(resp) = decode_body::<ListStableChannelsRequest>(&body) {
        return resp;
    }

    let entries = match state.db.load_all_channels() {
        Ok(e) => e,
        Err(e) => {
            warn!("load_all_channels failed: {}", e);
            return error_response(
                ErrorCode::InternalServerError,
                format!("Failed to load stable channels: {}", e),
            );
        },
    };

    let latest_price = get_cached_price_no_fetch();

    let channels = entries
        .into_iter()
        .map(|e| StableChannelInfo {
            channel_id: e.channel_id,
            counterparty: String::new(),
            expected_usd: e.expected_usd,
            expected_msats: e.backing_sats.saturating_mul(1_000),
            latest_price,
            note: e.note.unwrap_or_default(),
            is_stable_receiver: false,
        })
        .collect::<Vec<_>>();

    ok_response(ListStableChannelsResponse { channels })
}
