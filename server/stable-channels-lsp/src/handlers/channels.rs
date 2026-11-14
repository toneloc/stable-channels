//! REST proxies for LDK Server channel-management endpoints (open/close/splice/config).

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    CloseChannelRequest, ForceCloseChannelRequest, OpenChannelRequest, SpliceInRequest,
    SpliceOutRequest, UpdateChannelConfigRequest,
};

use ldk_server_client::ldk_server_grpc::error::ErrorCode;

use crate::handlers::{decode_body, error_response, map_grpc_error, ok_response};
use crate::state::AppState;

pub async fn open_channel(State(state): State<AppState>, body: Bytes) -> Response {
    let req: OpenChannelRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.open_channel(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn close_channel(State(state): State<AppState>, body: Bytes) -> Response {
    let req: CloseChannelRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.close_channel(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn force_close_channel(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ForceCloseChannelRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.force_close_channel(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn splice_in(State(state): State<AppState>, body: Bytes) -> Response {
    let req: SpliceInRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    // A queued splice-in the job is working on, or whose last call may still be pending on the node, owns the channel: a manual one now would splice twice, or be taken for the queued one's lock. Otherwise the call is noted before it is made, in the same transaction as that check, so a queued request waits until the channel shows this splice through, locked or gone.
    let user_channel_id = req.user_channel_id.clone();
    let replaced_a_note = match crate::queued_splice::before_manual_splice(
        &state.db,
        state.ldk_server.as_ref() as &dyn crate::stable_manager::LdkServerCalls,
        &user_channel_id,
        crate::revenue::now_secs(),
    )
    .await
    {
        Ok(replaced_a_note) => replaced_a_note,
        Err(message) => return error_response(ErrorCode::InvalidRequestError, format!("the splice-in was not made: {message}")),
    };
    match state.ldk_server.splice_in(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => {
            crate::queued_splice::after_manual_splice_failed(&state.db, &user_channel_id, replaced_a_note, &e);
            map_grpc_error(e)
        }
    }
}

pub async fn splice_out(State(state): State<AppState>, body: Bytes) -> Response {
    let req: SpliceOutRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.splice_out(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn update_channel_config(State(state): State<AppState>, body: Bytes) -> Response {
    let req: UpdateChannelConfigRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.update_channel_config(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
