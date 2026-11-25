//! REST proxies for LDK Server signing / verification / scores-export endpoints.

use axum::body::Bytes;
use axum::Extension;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    ExportPathfindingScoresRequest, SignMessageRequest, VerifySignatureRequest,
};

use crate::access::sessions::Caller;
use crate::handlers::{decode_body, map_grpc_error, ok_response};

pub async fn sign_message(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: SignMessageRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.sign_message(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn verify_signature(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: VerifySignatureRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.verify_signature(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn export_pathfinding_scores(Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    let req: ExportPathfindingScoresRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match caller.ldk.export_pathfinding_scores(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
