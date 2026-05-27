//! REST proxies for LDK Server Lightning send/receive endpoints (Bolt11, Bolt12, spontaneous).

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    Bolt11ReceiveRequest, Bolt11SendRequest, Bolt12ReceiveRequest, Bolt12SendRequest,
    SpontaneousSendRequest,
};

use crate::handlers::{decode_body, map_grpc_error, ok_response};
use crate::state::AppState;

pub async fn bolt11_receive(State(state): State<AppState>, body: Bytes) -> Response {
    let req: Bolt11ReceiveRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.bolt11_receive(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn bolt11_send(State(state): State<AppState>, body: Bytes) -> Response {
    let req: Bolt11SendRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.bolt11_send(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn bolt12_receive(State(state): State<AppState>, body: Bytes) -> Response {
    let req: Bolt12ReceiveRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.bolt12_receive(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn bolt12_send(State(state): State<AppState>, body: Bytes) -> Response {
    let req: Bolt12SendRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.bolt12_send(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn spontaneous_send(State(state): State<AppState>, body: Bytes) -> Response {
    let req: SpontaneousSendRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.spontaneous_send(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
