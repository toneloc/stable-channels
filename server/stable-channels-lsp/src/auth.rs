//! Body cap for requests the daemon reads in full before it checks them.

use axum::body::{Body, Bytes};
use axum::http::HeaderMap;
use axum::response::Response;
use ldk_server_client::ldk_server_grpc::error::ErrorCode;

use crate::handlers::error_response;

/// Maximum request body the session layer and Login will buffer before checking a request.
///
/// REST payloads here are small prost/JSON documents. The session layer reads the body in full to
/// check its signature, and `axum`'s default 2 MiB extractor limit does NOT apply to a manual
/// `to_bytes` read — so without this cap an unauthenticated client on the public `/api/*` proxy
/// could POST a multi-gigabyte body and OOM-kill the daemon (which also halts stability payments).
/// 1 MiB is generous for every real request and small enough that concurrent maxima stay bounded.
pub const MAX_AUTH_BODY_BYTES: usize = 1024 * 1024;

/// Whether a declared `Content-Length` already exceeds the cap, so the body can be rejected
/// before a single byte is buffered. A missing or unparseable header returns `false` — the
/// streaming read is still hard-capped at [`MAX_AUTH_BODY_BYTES`], so chunked bodies with no
/// declared length cannot bypass the limit.
fn declared_length_exceeds(content_length: Option<&str>, cap: usize) -> bool {
    content_length
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|len| len > cap as u64)
        .unwrap_or(false)
}

/// Reads the whole body, refusing one whose declared or actual length is over MAX_AUTH_BODY_BYTES.
pub async fn read_capped_body(headers: &HeaderMap, body: Body) -> Result<Bytes, Response> {
    let declared_len = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok());
    if declared_length_exceeds(declared_len, MAX_AUTH_BODY_BYTES) {
        return Err(error_response(ErrorCode::InvalidRequestError, "Request body too large"));
    }
    axum::body::to_bytes(body, MAX_AUTH_BODY_BYTES).await.map_err(|e| {
        error_response(
            ErrorCode::InvalidRequestError,
            format!("Failed to read body: {}", e),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_length_over_cap_is_rejected() {
        assert!(declared_length_exceeds(Some("1048577"), MAX_AUTH_BODY_BYTES));
        assert!(declared_length_exceeds(Some("9999999999"), MAX_AUTH_BODY_BYTES));
    }

    #[test]
    fn declared_length_at_or_under_cap_is_allowed() {
        assert!(!declared_length_exceeds(Some("1048576"), MAX_AUTH_BODY_BYTES));
        assert!(!declared_length_exceeds(Some("0"), MAX_AUTH_BODY_BYTES));
        assert!(!declared_length_exceeds(Some("512"), MAX_AUTH_BODY_BYTES));
    }

    #[test]
    fn absent_or_garbage_length_defers_to_streaming_cap() {
        // No pre-read rejection — the capped to_bytes read is the real limit.
        assert!(!declared_length_exceeds(None, MAX_AUTH_BODY_BYTES));
        assert!(!declared_length_exceeds(Some("not-a-number"), MAX_AUTH_BODY_BYTES));
        assert!(!declared_length_exceeds(Some(""), MAX_AUTH_BODY_BYTES));
    }
}
