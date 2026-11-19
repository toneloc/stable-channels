//! Login, Logout and WhoAmI: how the GUI starts, ends and checks a session.

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::response::Response;
use axum::Extension;
use ldk_server_client::ldk_server_grpc::error::ErrorCode;
use sc_protos::access::{LoginRequest, LogoutRequest, LogoutResponse, WhoAmIRequest, SESSION_UNKNOWN};
use tracing::{info, warn};

use crate::access::middleware::CANT_REACH_LDK;
use crate::access::sessions::{Caller, LoginError};
use crate::access::unix_now;
use crate::auth::read_capped_body;
use crate::handlers::{decode_body, error_response, ok_response};
use crate::state::AppState;

/// Turns the token in the body into a session; sits outside the session layer, so it caps the body itself.
pub async fn login(State(state): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let body = match read_capped_body(&parts.headers, body).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let request: LoginRequest = match decode_body(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let Some(access) = state.access.get() else {
        return error_response(ErrorCode::InternalServerError, CANT_REACH_LDK);
    };
    match access.sessions.login(&request.token, unix_now()).await {
        Ok(response) => ok_response(response),
        Err(error) => login_refusal(error),
    }
}

/// The answer for a refused login; it never repeats the token.
fn login_refusal(error: LoginError) -> Response {
    let message = match error {
        LoginError::Unreachable(why) => {
            warn!("GUI login: no answer from LDK Server: {why}");
            return error_response(ErrorCode::InternalServerError, CANT_REACH_LDK);
        }
        LoginError::Refused(why) => {
            info!("GUI login refused by LDK Server: {why}");
            "LDK Server refused this token (wrong, revoked or expired)"
        }
        LoginError::Malformed | LoginError::UnknownId => "LDK Server refused this token (wrong, revoked or expired)",
        LoginError::MethodLimited => "Tokens limited to one method can't log into the GUI",
        LoginError::RateLimited => "too many logins for this token, wait a minute",
        LoginError::Full => "too many sessions",
    };
    error_response(ErrorCode::AuthError, message)
}

pub async fn logout(State(state): State<AppState>, Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    if let Err(response) = decode_body::<LogoutRequest>(&body) {
        return response;
    }
    if let Some(access) = state.access.get() {
        access.sessions.logout(&caller.session_id);
    }
    ok_response(LogoutResponse {})
}

/// The session's token id, name and current permissions, which a re-check may have changed.
pub async fn who_am_i(State(state): State<AppState>, Extension(caller): Extension<Caller>, body: Bytes) -> Response {
    if let Err(response) = decode_body::<WhoAmIRequest>(&body) {
        return response;
    }
    match state.access.get().and_then(|access| access.sessions.whoami(&caller.session_id)) {
        Some(me) => ok_response(me),
        None => error_response(ErrorCode::AuthError, SESSION_UNKNOWN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use ldk_server_client::ldk_server_grpc::error::ErrorResponse;
    use prost::Message;

    #[tokio::test]
    async fn each_login_refusal_has_its_code_and_words() {
        let auth = (StatusCode::UNAUTHORIZED, ErrorCode::AuthError);
        let refused = "LDK Server refused this token (wrong, revoked or expired)";
        let cases = [
            (LoginError::Malformed, auth, refused),
            (LoginError::UnknownId, auth, refused),
            (LoginError::Refused("Macaroon expired".into()), auth, refused),
            (LoginError::MethodLimited, auth, "Tokens limited to one method can't log into the GUI"),
            (LoginError::RateLimited, auth, "too many logins for this token, wait a minute"),
            (LoginError::Full, auth, "too many sessions"),
            (
                LoginError::Unreachable("connection refused".into()),
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalServerError),
                "Can't reach LDK Server",
            ),
        ];
        for (error, (status, code), message) in cases {
            let label = format!("{error:?}");
            let response = login_refusal(error);
            assert_eq!(response.status(), status, "{label}");
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body = ErrorResponse::decode(&bytes[..]).unwrap();
            assert_eq!((body.error_code, body.message.as_str()), (code as i32, message), "{label}");
        }
    }
}
