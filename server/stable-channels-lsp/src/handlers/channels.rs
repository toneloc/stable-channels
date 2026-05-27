//! REST proxies for LDK Server channel-management endpoints (open/close/splice/config).

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    CloseChannelRequest, ForceCloseChannelRequest, OpenChannelRequest, SpliceInRequest,
    SpliceOutRequest, UpdateChannelConfigRequest,
};

use crate::handlers::{decode_body, map_grpc_error, ok_response};
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
    match state.ldk_server.splice_in(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
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
