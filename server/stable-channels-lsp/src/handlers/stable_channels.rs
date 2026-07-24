//! ListStableChannels handler. Reads from the in-memory StableChannelManager.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use sc_protos::stable::{
    EditStableChannelRequest, EditStableChannelResponse, ListSettlementPaymentsRequest,
    ListSettlementPaymentsResponse, ListStableChannelsRequest, ListStableChannelsResponse,
    SettlementPayment, StableChannelInfo,
};
use stable_channels::price_feeds::get_fresh_cached_price_no_fetch;

use crate::handlers::{decode_body, error_response, ok_response};
use crate::stable_manager::EditOutcome;
use crate::state::AppState;

pub async fn list_stable_channels(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    if let Err(resp) = decode_body::<ListStableChannelsRequest>(&body) {
        return resp;
    }

    let latest_price = get_fresh_cached_price_no_fetch();

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
            user_channel_id: format!("{}", sc.user_channel_id),
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

    let btc_price = get_fresh_cached_price_no_fetch();

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

pub async fn list_settlement_payments(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    if let Err(resp) = decode_body::<ListSettlementPaymentsRequest>(&body) {
        return resp;
    }

    let rows = match state.db.list_settlements() {
        Ok(r) => r,
        Err(e) => {
            return error_response(
                ldk_server_client::ldk_server_grpc::error::ErrorCode::InternalServerError,
                format!("list_settlements failed: {}", e),
            )
        }
    };

    let settlements = rows
        .into_iter()
        .map(|(payment_id, kind)| SettlementPayment { payment_id, kind })
        .collect::<Vec<_>>();

    ok_response(ListSettlementPaymentsResponse { settlements })
}
