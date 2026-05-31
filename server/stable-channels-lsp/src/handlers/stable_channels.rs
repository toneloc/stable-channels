//! ListStableChannels handler. Reads from the in-memory StableChannelManager.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use sc_protos::stable::{
    EditStableChannelRequest, EditStableChannelResponse, ListStableChannelsRequest,
    ListStableChannelsResponse, StableChannelInfo,
};
use stable_channels::price_feeds::get_cached_price_no_fetch;

use crate::handlers::{decode_body, ok_response};
use crate::stable_manager::EditOutcome;
use crate::state::AppState;

pub async fn list_stable_channels(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    if let Err(resp) = decode_body::<ListStableChannelsRequest>(&body) {
        return resp;
    }

    let latest_price = get_cached_price_no_fetch();

    let mgr = state.stable_manager.lock().await;
    let channels = mgr
        .stable_channels
        .iter()
        .map(|sc| StableChannelInfo {
            channel_id: sc.channel_id.to_string(),
            counterparty: sc.counterparty.to_string(),
            expected_usd: sc.expected_usd.0,
            expected_msats: sc.backing_sats.saturating_mul(1_000),
            latest_price,
            note: sc.note.clone().unwrap_or_default(),
            is_stable_receiver: sc.is_stable_receiver,
        })
        .collect::<Vec<_>>();
    drop(mgr);

    ok_response(ListStableChannelsResponse { channels })
}

pub async fn edit_stable_channel(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let req: EditStableChannelRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let btc_price = stable_channels::price_feeds::get_cached_price_no_fetch();

    let EditOutcome { ok, status } = {
        let mut mgr = state.stable_manager.lock().await;
        mgr.edit_stable_channel(
            &req.channel_id,
            req.expected_usd,
            req.note,
            state.ldk_server.as_ref() as &dyn crate::stable_manager::LdkServerCalls,
            btc_price,
        )
        .await
    };

    ok_response(EditStableChannelResponse { ok, status })
}
