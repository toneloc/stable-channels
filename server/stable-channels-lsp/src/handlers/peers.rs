//! REST proxies for LDK Server peer-management endpoints.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    ConnectPeerRequest, DisconnectPeerRequest, ListPeersRequest,
};

use crate::handlers::{decode_body, map_grpc_error, ok_response};
use crate::state::AppState;

pub async fn list_peers(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ListPeersRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.list_peers(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn connect_peer(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ConnectPeerRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.connect_peer(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn disconnect_peer(State(state): State<AppState>, body: Bytes) -> Response {
    let req: DisconnectPeerRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.disconnect_peer(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
