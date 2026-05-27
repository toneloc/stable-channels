//! REST proxies for LDK Server signing / verification / scores-export endpoints.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    ExportPathfindingScoresRequest, SignMessageRequest, VerifySignatureRequest,
};

use crate::handlers::{decode_body, map_grpc_error, ok_response};
use crate::state::AppState;

pub async fn sign_message(State(state): State<AppState>, body: Bytes) -> Response {
    let req: SignMessageRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.sign_message(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn verify_signature(State(state): State<AppState>, body: Bytes) -> Response {
    let req: VerifySignatureRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.verify_signature(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn export_pathfinding_scores(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ExportPathfindingScoresRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.export_pathfinding_scores(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
