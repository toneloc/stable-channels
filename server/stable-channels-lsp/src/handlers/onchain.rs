//! REST proxies for LDK Server on-chain send/receive endpoints.

use axum::body::Bytes;
use axum::Extension;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{OnchainReceiveRequest, OnchainSendRequest};

use crate::access::sessions::Caller;
use crate::handlers::{decode_body, map_grpc_error, ok_response};

pub async fn onchain_receive(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: OnchainReceiveRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.onchain_receive(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn onchain_send(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: OnchainSendRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.onchain_send(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
