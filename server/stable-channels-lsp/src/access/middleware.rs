//! The session layer: a GUI route runs only for a signed request from a live session whose token allows it.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use ldk_server_client::ldk_server_grpc::error::ErrorCode;
use sc_protos::access::SESSION_UNKNOWN;

use crate::access::sessions::VerifyError;
use crate::access::unix_now;
use crate::auth::read_capped_body;
use crate::handlers::error_response;
use crate::state::AppState;

/// What the GUI is told when LDK Server gives no answer, so it doesn't treat an outage as a bad token.
pub const CANT_REACH_LDK: &str = "Can't reach LDK Server";

/// Checks the request's session signature and the route's permission, then hands the handler the `Caller`.
pub async fn session_layer(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let header = req.headers().get("x-auth").and_then(|value| value.to_str().ok()).map(str::to_owned);
    let path = req.uri().path();
    let route = path.strip_prefix('/').unwrap_or(path).to_owned();
    let (mut parts, body) = req.into_parts();
    let body = match read_capped_body(&parts.headers, body).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(access) = state.access.get() else {
        return refusal(VerifyError::Unknown);
    };
    match access.sessions.verify(header.as_deref(), &route, &body, unix_now()).await {
        Ok(caller) => {
            parts.extensions.insert(caller);
            next.run(Request::from_parts(parts, Body::from(body))).await
        }
        Err(error) => refusal(error),
    }
}

/// The answer for a request the session layer refused.
fn refusal(error: VerifyError) -> Response {
    let message = match error {
        VerifyError::Unreachable => return error_response(ErrorCode::InternalServerError, CANT_REACH_LDK),
        VerifyError::Unknown => SESSION_UNKNOWN.to_string(),
        VerifyError::Stale => "request too old: check this computer's clock".to_string(),
        VerifyError::BadMac => "request signature does not match".to_string(),
        VerifyError::Replayed => "replayed request".to_string(),
        VerifyError::Revoked => "token revoked or expired".to_string(),
        VerifyError::Lacks(permission) => format!("this token lacks {permission}"),
        VerifyError::UnknownRoute => "this route is not open to GUI sessions".to_string(),
    };
    error_response(ErrorCode::AuthError, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use ldk_server_client::ldk_server_grpc::error::ErrorResponse;
    use prost::Message;

    #[tokio::test]
    async fn each_refusal_has_its_code_and_words() {
        let auth = (StatusCode::UNAUTHORIZED, ErrorCode::AuthError);
        let cases = [
            (VerifyError::Unknown, auth, SESSION_UNKNOWN),
            (VerifyError::Stale, auth, "request too old: check this computer's clock"),
            (VerifyError::BadMac, auth, "request signature does not match"),
            (VerifyError::Replayed, auth, "replayed request"),
            (VerifyError::Revoked, auth, "token revoked or expired"),
            (VerifyError::Lacks("payments:send"), auth, "this token lacks payments:send"),
            (VerifyError::UnknownRoute, auth, "this route is not open to GUI sessions"),
            (
                VerifyError::Unreachable,
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalServerError),
                "Can't reach LDK Server",
            ),
        ];
        for (error, (status, code), message) in cases {
            let response = refusal(error);
            assert_eq!(response.status(), status, "{error:?}");
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body = ErrorResponse::decode(&bytes[..]).unwrap();
            assert_eq!((body.error_code, body.message.as_str()), (code as i32, message), "{error:?}");
        }
    }
}
