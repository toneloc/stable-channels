//! GetRevenue pages the revenue snapshot the background task keeps; RefundTradeFee returns a rejected trade's fee.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use ldk_server_client::ldk_server_grpc::error::ErrorCode;
use sc_protos::revenue::{GetRevenueRequest, GetRevenueResponse, RefundTradeFeeRequest, RefundTradeFeeResponse};

use crate::handlers::{decode_body, error_response, ok_response};
use crate::revenue::{page, summarize, DEFAULT_PAGE_LIMIT};
use crate::state::AppState;

pub async fn get_revenue(State(state): State<AppState>, body: Bytes) -> Response {
    let req: GetRevenueRequest = match decode_body(&body) {
        Ok(req) => req,
        Err(resp) => return resp,
    };
    let Some(snapshot) = state.revenue.snapshot() else {
        return error_response(ErrorCode::InternalServerError, "Revenue is still being computed; try again shortly");
    };
    let limit = if req.limit == 0 { DEFAULT_PAGE_LIMIT } else { req.limit as usize };
    let (items, next_cursor) = page(&snapshot.items, req.since, &req.categories, req.cursor.as_deref(), limit);
    ok_response(GetRevenueResponse {
        lines: summarize(&snapshot.items, req.since),
        items,
        next_cursor,
        snapshot_at: snapshot.built_at,
        partial: snapshot.partial(req.since),
        untracked: snapshot.untracked.clone(),
        item_count: snapshot.items.len() as u64,
    })
}

pub async fn refund_trade_fee(State(state): State<AppState>, body: Bytes) -> Response {
    let req: RefundTradeFeeRequest = match decode_body(&body) {
        Ok(req) => req,
        Err(resp) => return resp,
    };
    let ldk: &dyn crate::stable_manager::LdkServerCalls = state.ldk_server.as_ref();
    match crate::revenue::refund_trade_fee(&state.db, ldk, &req.trade_payment_id, crate::revenue::now_secs()).await {
        Ok((refund_payment_id, amount_msat)) => {
            state.revenue.request_rebuild();
            ok_response(RefundTradeFeeResponse { refund_payment_id, amount_msat })
        },
        Err(error) => error_response(ErrorCode::InvalidRequestError, error.message()),
    }
}
