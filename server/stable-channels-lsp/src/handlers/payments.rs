//! REST proxies for LDK Server payment-history endpoints.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::{
    GetPaymentDetailsRequest, ListForwardedPaymentsRequest, ListPaymentsRequest,
};

use crate::handlers::{decode_body, map_grpc_error, ok_response};
use crate::state::AppState;

pub async fn list_payments(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ListPaymentsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.list_payments(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn get_payment_details(State(state): State<AppState>, body: Bytes) -> Response {
    let req: GetPaymentDetailsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.get_payment_details(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}

pub async fn list_forwarded_payments(State(state): State<AppState>, body: Bytes) -> Response {
    let req: ListForwardedPaymentsRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.ldk_server.list_forwarded_payments(req).await {
        Ok(resp) => ok_response(resp),
        Err(e) => map_grpc_error(e),
    }
}
