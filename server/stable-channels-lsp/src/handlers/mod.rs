//! REST handler infrastructure: error mapping, response encoding.

pub mod audit_log;
pub mod channel_exists;
pub mod channel_ledger;
pub mod channels;
pub mod funding_outpoints;
pub mod graph;
pub mod ldk_log;
pub mod lightning;
pub mod onchain;
pub mod payments;
pub mod peers;
pub mod price;
pub mod proxy;
pub mod queued_splices;
pub mod register_email;
pub mod register_push;
pub mod revenue;
pub mod stable_channels;
pub mod tools;

use std::str::FromStr;

use axum::body::Bytes;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use ldk_node::bitcoin::secp256k1::PublicKey;
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
        // The daemon's macaroon lacks this RPC's permission; the REST enum has no separate code.
        LdkServerErrorCode::AuthorizationError => ErrorCode::AuthError,
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

/// A zbase32 node signature is 104 characters; a much longer one is refused before any work.
pub const MAX_SIGNATURE_CHARS: usize = 150;

/// Whether `signature` is worth verifying: present and no longer than MAX_SIGNATURE_CHARS.
pub fn is_plausible_signature(signature: &str) -> bool {
    !signature.is_empty() && signature.len() <= MAX_SIGNATURE_CHARS
}

/// Returns whether the node_id is a valid 33-byte compressed pubkey hex string.
pub fn is_valid_node_id(node_id: &str) -> bool {
    node_id.len() == 66
        && node_id.chars().all(|c| c.is_ascii_hexdigit())
        && (node_id.starts_with("02") || node_id.starts_with("03"))
}

/// The node's public key, or None when node_id is not a valid compressed pubkey hex string.
pub fn parse_node_key(node_id: &str) -> Option<PublicKey> {
    is_valid_node_id(node_id)
        .then(|| PublicKey::from_str(node_id).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: &str = "024d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766";

    #[test]
    fn a_node_key_parses_only_from_a_valid_node_id() {
        assert_eq!(
            parse_node_key(NODE).map(|k| k.to_string()).as_deref(),
            Some(NODE)
        );
        assert!(parse_node_key(&format!("04{}", &NODE[2..])).is_none());
        assert!(parse_node_key(&NODE[..64]).is_none());
        assert!(
            parse_node_key(&format!("02{}", "0".repeat(64))).is_none(),
            "not a point on the curve"
        );
    }

    #[tokio::test]
    async fn a_missing_macaroon_permission_reaches_the_client_as_a_401_with_ldks_message() {
        let error = LdkServerError::new(LdkServerErrorCode::AuthorizationError, "macaroon requires permission: channels:read");

        let response = map_grpc_error(error);

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = ErrorResponse::decode(&bytes[..]).unwrap();
        assert_eq!(body.error_code, ErrorCode::AuthError as i32);
        assert_eq!(body.message, "macaroon requires permission: channels:read");
    }

    #[test]
    fn a_signature_is_plausible_when_present_and_not_too_long() {
        assert!(!is_plausible_signature(""));
        assert!(is_plausible_signature(&"s".repeat(MAX_SIGNATURE_CHARS)));
        assert!(!is_plausible_signature(
            &"s".repeat(MAX_SIGNATURE_CHARS + 1)
        ));
    }
}
