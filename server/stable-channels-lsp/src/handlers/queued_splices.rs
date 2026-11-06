//! Operator routes for queued LSP splice-ins: queue one, list them, cancel one.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use sc_protos::stable::{
    CancelQueuedSpliceRequest, CancelQueuedSpliceResponse, ListQueuedSplicesRequest,
    ListQueuedSplicesResponse, QueueSpliceInRequest, QueueSpliceInResponse, QueuedSpliceInfo,
};

use crate::handlers::{decode_body, error_response, ok_response};
use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

pub async fn queue_splice_in(State(state): State<AppState>, body: Bytes) -> Response {
    let req: QueueSpliceInRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let (ok, status) = crate::queued_splice::queue(
        &state.db,
        state.ldk_server.as_ref() as &dyn LdkServerCalls,
        &req.user_channel_id,
        &req.counterparty_node_id,
        req.amount_sats,
        crate::revenue::now_secs(),
    )
    .await;
    ok_response(QueueSpliceInResponse { ok, status })
}

pub async fn list_queued_splices(State(state): State<AppState>, body: Bytes) -> Response {
    if let Err(resp) = decode_body::<ListQueuedSplicesRequest>(&body) {
        return resp;
    }
    let rows = match state.db.list_queued_splices() {
        Ok(rows) => rows,
        Err(e) => {
            return error_response(
                ldk_server_client::ldk_server_grpc::error::ErrorCode::InternalServerError,
                format!("list_queued_splices failed: {}", e),
            )
        }
    };
    let splices = rows
        .into_iter()
        .map(|row| QueuedSpliceInfo {
            user_channel_id: row.user_channel_id,
            counterparty_node_id: row.counterparty,
            amount_sats: row.amount_sats,
            status: row.status,
            attempts: row.attempts,
            last_error: row.last_error.unwrap_or_default(),
            funding_txo: row.funding_txo.unwrap_or_default(),
            created_at: row.created_at,
            updated_at: row.updated_at,
            wake_sent_at: row.wake_sent_at.unwrap_or(0),
            last_attempt_at: row.last_attempt_at.unwrap_or(0),
            wake_note: row.wake_note.unwrap_or_default(),
            pending_on_node: row.pending_on_node,
        })
        .collect();
    ok_response(ListQueuedSplicesResponse { splices })
}

pub async fn cancel_queued_splice(State(state): State<AppState>, body: Bytes) -> Response {
    let req: CancelQueuedSpliceRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let (ok, status) = crate::queued_splice::cancel(
        &state.db,
        &req.user_channel_id,
        req.force,
        crate::revenue::now_secs(),
    );
    ok_response(CancelQueuedSpliceResponse { ok, status })
}
