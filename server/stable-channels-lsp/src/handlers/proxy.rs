//! Proxy handlers. Decode REST request, forward to LDK Server gRPC, encode response.

use axum::body::Bytes;
use axum::Extension;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    GetBalancesRequest, GetNodeInfoRequest, ListChannelsRequest,
};

use crate::access::sessions::Caller;
use crate::handlers::{decode_body, map_grpc_error, ok_response};

pub async fn get_node_info(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: GetNodeInfoRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.get_node_info(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn get_balances(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: GetBalancesRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.get_balances(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn list_channels(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: ListChannelsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.list_channels(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
