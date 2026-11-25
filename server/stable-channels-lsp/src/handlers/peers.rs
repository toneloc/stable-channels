//! REST proxies for LDK Server peer-management endpoints.

use axum::body::Bytes;
use axum::Extension;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    ConnectPeerRequest, DisconnectPeerRequest, ListPeersRequest,
};

use crate::access::sessions::Caller;
use crate::handlers::{decode_body, map_grpc_error, ok_response};

pub async fn list_peers(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: ListPeersRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.list_peers(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn connect_peer(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: ConnectPeerRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.connect_peer(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn disconnect_peer(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: DisconnectPeerRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.disconnect_peer(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
