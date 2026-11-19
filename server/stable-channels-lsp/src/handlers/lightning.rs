//! REST proxies for LDK Server Lightning send/receive endpoints (Bolt11, Bolt12, spontaneous).

use axum::body::Bytes;
use axum::Extension;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    Bolt11ReceiveRequest, Bolt11SendRequest, Bolt12ReceiveRequest, Bolt12SendRequest,
    SpontaneousSendRequest,
};

use crate::access::sessions::Caller;
use crate::handlers::{decode_body, map_grpc_error, ok_response};

pub async fn bolt11_receive(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: Bolt11ReceiveRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.bolt11_receive(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn bolt11_send(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: Bolt11SendRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.bolt11_send(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn bolt12_receive(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: Bolt12ReceiveRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.bolt12_receive(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn bolt12_send(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: Bolt12SendRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.bolt12_send(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn spontaneous_send(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: SpontaneousSendRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.spontaneous_send(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
