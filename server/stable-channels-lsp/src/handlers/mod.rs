//! REST handler infrastructure: error mapping, response encoding.

pub mod audit_log;
pub mod price;
pub mod proxy;
pub mod stable_channels;

use axum::body::Bytes;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use ldk_server_client::error::{LdkServerError, LdkServerErrorCode};
use ldk_server_client::ldk_server_grpc::error::{ErrorCode, ErrorResponse};
use prost::Message;

/// Wrap a successful protobuf response into an HTTP 200 with application/octet-stream.
pub fn ok_response<M: Message>(msg: M) -> Response {
    let bytes = msg.encode_to_vec();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    )
        .into_response()
}

/// Encode an ErrorResponse with the given code and message as the appropriate HTTP status.
pub fn error_response(code: ErrorCode, message: impl Into<String>) -> Response {
    let body = ErrorResponse {
        message: message.into(),
        error_code: code as i32,
    };
    let bytes = body.encode_to_vec();
    let status = match code {
        ErrorCode::UnknownError => StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::InvalidRequestError => StatusCode::BAD_REQUEST,
        ErrorCode::AuthError => StatusCode::UNAUTHORIZED,
        ErrorCode::LightningError => StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    )
        .into_response()
}

/// Map an LdkServerError (from the gRPC client) into our REST ErrorResponse.
pub fn map_grpc_error(e: LdkServerError) -> Response {
    let code = match e.error_code {
        LdkServerErrorCode::InvalidRequestError => ErrorCode::InvalidRequestError,
        LdkServerErrorCode::AuthError => ErrorCode::AuthError,
        LdkServerErrorCode::LightningError => ErrorCode::LightningError,
        LdkServerErrorCode::InternalServerError => ErrorCode::InternalServerError,
        LdkServerErrorCode::InternalError => ErrorCode::UnknownError,
    };
    error_response(code, e.message)
}

/// Decode a prost-encoded request body, returning a 400 ErrorResponse on failure.
pub fn decode_body<M: Message + Default>(body: &Bytes) -> Result<M, Response> {
    M::decode(&body[..]).map_err(|e| {
        error_response(
            ErrorCode::InvalidRequestError,
            format!("Failed to decode request body: {}", e),
        )
    })
}
