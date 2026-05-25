//! HMAC-SHA256 authentication for the REST surface. X-Auth header: `HMAC <ts>:<hex>`.

use bitcoin_hashes::hmac::{Hmac, HmacEngine};
use bitcoin_hashes::{sha256, Hash, HashEngine};
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum allowed clock skew between client and server, in seconds.
pub const TIMESTAMP_TOLERANCE_SECS: u64 = 60;

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    MissingHeader,
    InvalidFormat,
    InvalidTimestamp,
    TimestampExpired,
    HmacMismatch,
    SystemTimeError,
}

/// Validate an X-Auth header against an api_key and request body.
pub fn validate_auth(
    auth_header: Option<&str>,
    api_key: &[u8],
    body: &[u8],
) -> Result<(), AuthError> {
    let header = auth_header.ok_or(AuthError::MissingHeader)?;
    let auth_data = header.strip_prefix("HMAC ").ok_or(AuthError::InvalidFormat)?;
    let (ts_str, hex_str) = auth_data.split_once(':').ok_or(AuthError::InvalidFormat)?;
    let timestamp = ts_str.parse::<u64>().map_err(|_| AuthError::InvalidTimestamp)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthError::SystemTimeError)?
        .as_secs();
    if now.abs_diff(timestamp) > TIMESTAMP_TOLERANCE_SECS {
        return Err(AuthError::TimestampExpired);
    }

    let mut eng: HmacEngine<sha256::Hash> = HmacEngine::new(api_key);
    eng.input(&timestamp.to_be_bytes());
    eng.input(body);
    let expected = Hmac::<sha256::Hash>::from_engine(eng);
    let expected_hex = format!("{:x}", expected);

    if expected_hex == hex_str {
        Ok(())
    } else {
        Err(AuthError::HmacMismatch)
    }
}

/// Build an X-Auth header for the given api_key and body.
pub fn build_auth_header(api_key: &[u8], body: &[u8]) -> String {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let mut eng: HmacEngine<sha256::Hash> = HmacEngine::new(api_key);
    eng.input(&timestamp.to_be_bytes());
    eng.input(body);
    let hmac = Hmac::<sha256::Hash>::from_engine(eng);
    format!("HMAC {}:{:x}", timestamp, hmac)
}

use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::handlers::error_response;
use crate::state::AppState;
use ldk_server_client::ldk_server_grpc::error::ErrorCode;

/// Axum middleware that validates `X-Auth` against the configured api_key.
pub async fn auth_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let auth_header = req
        .headers()
        .get("x-auth")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            return error_response(
                ErrorCode::InvalidRequestError,
                format!("Failed to read body: {}", e),
            );
        },
    };

    if validate_auth(auth_header.as_deref(), &state.api_key, &body_bytes).is_err() {
        return error_response(ErrorCode::AuthError, "Invalid X-Auth header");
    }

    let new_req = Request::from_parts(parts, Body::from(body_bytes));
    next.run(new_req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_succeeds() {
        let key = b"deadbeefdeadbeefdeadbeefdeadbeef";
        let body = b"some-prost-bytes";
        let header = build_auth_header(key, body);
        assert!(validate_auth(Some(&header), key, body).is_ok());
    }

    #[test]
    fn missing_header_fails() {
        assert_eq!(validate_auth(None, b"k", b""), Err(AuthError::MissingHeader));
    }

    #[test]
    fn malformed_prefix_fails() {
        assert_eq!(
            validate_auth(Some("Basic abcdef"), b"k", b""),
            Err(AuthError::InvalidFormat),
        );
    }

    #[test]
    fn missing_colon_fails() {
        assert_eq!(
            validate_auth(Some("HMAC 1234567890abcdef"), b"k", b""),
            Err(AuthError::InvalidFormat),
        );
    }

    #[test]
    fn invalid_timestamp_fails() {
        assert_eq!(
            validate_auth(Some("HMAC notanumber:deadbeef"), b"k", b""),
            Err(AuthError::InvalidTimestamp),
        );
    }

    #[test]
    fn old_timestamp_fails() {
        let key = b"k";
        let body = b"";
        let stale_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 3600;
        let mut eng: HmacEngine<sha256::Hash> = HmacEngine::new(key);
        eng.input(&stale_ts.to_be_bytes());
        eng.input(body);
        let hmac = Hmac::<sha256::Hash>::from_engine(eng);
        let header = format!("HMAC {}:{:x}", stale_ts, hmac);
        assert_eq!(
            validate_auth(Some(&header), key, body),
            Err(AuthError::TimestampExpired),
        );
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = b"key-one";
        let key2 = b"key-two";
        let body = b"body";
        let header = build_auth_header(key1, body);
        assert_eq!(
            validate_auth(Some(&header), key2, body),
            Err(AuthError::HmacMismatch),
        );
    }

    #[test]
    fn wrong_body_fails() {
        let key = b"key";
        let header = build_auth_header(key, b"body-a");
        assert_eq!(
            validate_auth(Some(&header), key, b"body-b"),
            Err(AuthError::HmacMismatch),
        );
    }
}
