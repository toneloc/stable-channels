//! RegisterPush handler: wallets POST a signed APNs/FCM device token and Lightning node id, stored in push_tokens.db.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::VerifySignatureRequest;
use sc_protos::stable::{RegisterPushRequest, RegisterPushResponse};

use crate::handlers::{decode_body, ok_response};
use crate::push::registrations::RegisterOutcome;
use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

/// Freshness window for the signed timestamp, in seconds.
const PUSH_SIG_WINDOW_SECS: u64 = 300;
/// Longest device token stored: APNs tokens are 64 hex characters and FCM tokens a few hundred, but neither provider promises a length.
const MAX_DEVICE_TOKEN_LEN: usize = 4096;

/// The first field of a registration refused before anything about it is stored, or None when every field is acceptable.
fn invalid_registration_field(token: &str, platform: &str, node_id: &str, environment: &str) -> Option<&'static str> {
    if token.is_empty() || token.len() > MAX_DEVICE_TOKEN_LEN || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Some("device_token");
    }
    if !matches!(platform, "ios" | "android") {
        return Some("platform");
    }
    // Tokens are only ever looked up by node, so one without a node's public key could never wake anyone.
    let node_key = node_id.len() == 66 && (node_id.starts_with("02") || node_id.starts_with("03")) && node_id.bytes().all(|b| b.is_ascii_hexdigit());
    if !node_key {
        return Some("node_id");
    }
    if !matches!(environment, "" | "sandbox" | "production") {
        return Some("environment");
    }
    None
}

/// Store a checked registration, or hold it until the node has a channel, and record a hold.
async fn store_registration(state: &AppState, token: &str, platform: &str, node_id: &str, environment: &str) -> RegisterOutcome {
    let outcome = state.push_registrations.register(token, platform, node_id, environment).await;
    if outcome == RegisterOutcome::Held {
        stable_channels::audit::audit_event(
            "REGISTER_PUSH_HELD",
            serde_json::json!({ "node_id": node_id.to_ascii_lowercase(), "platform": platform }),
        );
    }
    outcome
}

/// Verify a RegisterPush node-ownership proof: the signature over the canonical
/// {type,node_id,token,ts} JSON must verify against node_id, and ts must be within the window.
pub async fn verify_push_registration(
    ldk: &dyn LdkServerCalls,
    node_id: &str,
    token: &str,
    signature: &str,
    timestamp: u64,
    now: u64,
    window_secs: u64,
) -> bool {
    if now.abs_diff(timestamp) > window_secs {
        return false;
    }
    let message = crate::messages::register_push_signed_bytes(node_id, token, timestamp);
    match ldk
        .verify_signature(VerifySignatureRequest {
            message: message.into(),
            signature: signature.to_string(),
            public_key: node_id.to_string(),
        })
        .await
    {
        Ok(r) => r.valid,
        Err(_) => false,
    }
}

pub async fn register_push(State(state): State<AppState>, body: Bytes) -> Response {
    let req: RegisterPushRequest = match decode_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    if let Some(field) = invalid_registration_field(&req.token, &req.platform, &req.node_id, &req.environment) {
        stable_channels::audit::audit_event(
            "REGISTER_PUSH_INVALID",
            serde_json::json!({ "field": field }),
        );
        return ok_response(RegisterPushResponse { ok: false });
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    let ok = verify_push_registration(
        ldk,
        &req.node_id,
        &req.token,
        &req.signature,
        req.timestamp,
        now,
        PUSH_SIG_WINDOW_SECS,
    )
    .await;
    if !ok {
        stable_channels::audit::audit_event(
            "REGISTER_PUSH_SIGNATURE_INVALID",
            serde_json::json!({ "node_id": req.node_id }),
        );
        return ok_response(RegisterPushResponse { ok: false });
    }

    if store_registration(&state, &req.token, &req.platform, &req.node_id, &req.environment).await == RegisterOutcome::Failed {
        return ok_response(RegisterPushResponse { ok: false });
    }
    stable_channels::audit::audit_event(
        "REGISTER_PUSH_OK",
        serde_json::json!({ "node_id": req.node_id }),
    );
    ok_response(RegisterPushResponse { ok: true })
}

/// The unsigned registration body every deployed wallet build sends (fork contract).
#[derive(serde::Deserialize)]
pub struct LegacyPushRequest {
    pub device_token: String,
    pub platform: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub environment: String,
}

/// A refused unsigned registration: what the caller is told, and an audit record that keeps no text from the request.
pub struct LegacyRejection {
    pub status: axum::http::StatusCode,
    pub body: String,
    pub audit: serde_json::Value,
}

/// Parse and check an unsigned registration before anything about it is stored or recorded.
pub fn parse_legacy_registration(body: &[u8]) -> Result<LegacyPushRequest, LegacyRejection> {
    let req: LegacyPushRequest = serde_json::from_slice(body).map_err(|e| LegacyRejection {
        status: axum::http::StatusCode::BAD_REQUEST,
        // The caller still gets the parse error back; the record keeps only its kind and place, since the error quotes the request.
        body: e.to_string(),
        audit: serde_json::json!({ "reason": "unparseable", "category": format!("{:?}", e.classify()), "line": e.line(), "column": e.column() }),
    })?;
    if let Some(field) = invalid_registration_field(&req.device_token, &req.platform, &req.node_id, &req.environment) {
        return Err(LegacyRejection {
            status: axum::http::StatusCode::BAD_REQUEST,
            body: format!("invalid {field}"),
            audit: serde_json::json!({ "reason": "invalid_field", "field": field }),
        });
    }
    Ok(req)
}

/// Legacy push registration: deployed wallets POST unsigned JSON to /api/register-push
/// (Apache-proxied, no HMAC, no node-ownership proof). Mounted OUTSIDE the auth layer.
/// Response shape mirrors the fork: 200 `"ok"` (stored, or held until the node has a channel),
/// 400 with the parse error or the refused field. Remove once every install signs registrations (see verify_push_registration).
pub async fn register_push_legacy(State(state): State<AppState>, body: Bytes) -> Response {
    let text = |status: axum::http::StatusCode, body: String| {
        Response::builder()
            .status(status)
            .header("content-type", "text/plain")
            .body(axum::body::Body::from(body))
            .unwrap()
    };
    let req = match parse_legacy_registration(&body) {
        Ok(req) => req,
        Err(rejection) => {
            stable_channels::audit::audit_event("REGISTER_PUSH_LEGACY_INVALID", rejection.audit);
            return text(rejection.status, rejection.body);
        }
    };
    // A request waiting for its write turn keeps only the checked fields.
    drop(body);

    match store_registration(&state, &req.device_token, &req.platform, &req.node_id, &req.environment).await {
        RegisterOutcome::Saved => {},
        RegisterOutcome::Held => {
            return Response::builder()
                .status(axum::http::StatusCode::OK)
                .header("content-type", "application/json")
                .body(axum::body::Body::from("\"ok\""))
                .unwrap();
        },
        RegisterOutcome::Failed => {
            return text(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "push registration failed".to_owned());
        },
    }
    // Flagged as LEGACY: this token was stored without a node-ownership proof.
    stable_channels::audit::audit_event(
        "REGISTER_PUSH_LEGACY_OK",
        serde_json::json!({ "node_id": req.node_id, "platform": req.platform }),
    );
    Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("content-type", "application/json")
        .body(axum::body::Body::from("\"ok\""))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ldk_server_client::error::LdkServerError;
    use ldk_server_client::ldk_server_grpc::api::{
        ListChannelsRequest, ListChannelsResponse, SignMessageRequest, SignMessageResponse,
        SpontaneousSendRequest, SpontaneousSendResponse, VerifySignatureRequest,
        VerifySignatureResponse,
    };

    struct VerifyFake {
        valid: bool,
    }

    #[async_trait]
    impl LdkServerCalls for VerifyFake {
        async fn list_channels(
            &self,
            _req: ListChannelsRequest,
        ) -> Result<ListChannelsResponse, LdkServerError> {
            Ok(ListChannelsResponse { channels: vec![] })
        }
        async fn spontaneous_send(
            &self,
            _req: SpontaneousSendRequest,
        ) -> Result<SpontaneousSendResponse, LdkServerError> {
            Ok(SpontaneousSendResponse { payment_id: String::new() })
        }
        async fn sign_message(
            &self,
            _req: SignMessageRequest,
        ) -> Result<SignMessageResponse, LdkServerError> {
            Ok(SignMessageResponse { signature: String::new() })
        }
        async fn verify_signature(
            &self,
            _req: VerifySignatureRequest,
        ) -> Result<VerifySignatureResponse, LdkServerError> {
            Ok(VerifySignatureResponse { valid: self.valid })
        }
    }

    #[tokio::test]
    async fn valid_fresh_signature_accepted() {
        let fake = VerifyFake { valid: true };
        let ok = verify_push_registration(&fake, "node", "token", "sig", 1000, 1000, 300).await;
        assert!(ok);
    }

    #[tokio::test]
    async fn stale_timestamp_rejected() {
        let fake = VerifyFake { valid: true };
        let ok = verify_push_registration(&fake, "node", "token", "sig", 1000, 2000, 300).await;
        assert!(!ok); // 1000s drift > 300s window
    }

    #[tokio::test]
    async fn invalid_signature_rejected() {
        let fake = VerifyFake { valid: false };
        let ok = verify_push_registration(&fake, "node", "token", "sig", 1000, 1000, 300).await;
        assert!(!ok);
    }

    #[test]
    fn legacy_body_parses_with_and_without_optional_fields() {
        // Exact shape shipped iOS/Android builds send.
        let full: LegacyPushRequest = serde_json::from_str(
            r#"{"device_token":"tok","platform":"ios","node_id":"02ab","environment":"production"}"#,
        )
        .unwrap();
        assert_eq!(full.environment, "production");

        // node_id/environment are optional in the fork contract.
        let minimal: LegacyPushRequest =
            serde_json::from_str(r#"{"device_token":"tok","platform":"android"}"#).unwrap();
        assert_eq!(minimal.node_id, "");
        assert_eq!(minimal.environment, "");
    }

    const NODE: &str = "02465ed5be53d04fde66c9418ff14a5f2267723810176c9212b722e542dc1afb1b";

    #[test]
    fn registration_fields_are_checked_before_anything_is_stored() {
        let apns = "ab".repeat(32);
        assert_eq!(invalid_registration_field(&apns, "ios", NODE, "production"), None);
        assert_eq!(invalid_registration_field("fcm-Token_1:APA91b", "android", NODE, ""), None);
        assert_eq!(invalid_registration_field(&"a".repeat(MAX_DEVICE_TOKEN_LEN + 1), "ios", NODE, ""), Some("device_token"));
        assert_eq!(invalid_registration_field("", "ios", NODE, ""), Some("device_token"));
        assert_eq!(invalid_registration_field("tok en", "ios", NODE, ""), Some("device_token"));
        assert_eq!(invalid_registration_field(&apns, "web", NODE, ""), Some("platform"));
        assert_eq!(invalid_registration_field(&apns, "ios", "", ""), Some("node_id"), "a token no node can be woken through is not stored");
        assert_eq!(invalid_registration_field(&apns, "ios", "02ab", ""), Some("node_id"));
        assert_eq!(invalid_registration_field(&apns, "ios", &"zz".repeat(33), ""), Some("node_id"));
        assert_eq!(invalid_registration_field(&apns, "ios", NODE, "staging"), Some("environment"));
    }

    #[test]
    fn a_rejected_body_is_never_recorded() {
        let payload = "x".repeat(512 * 1024);
        let body = serde_json::to_vec(&payload).unwrap();
        let rejection = parse_legacy_registration(&body).err().expect("a string is not a registration");
        assert_eq!(rejection.status, axum::http::StatusCode::BAD_REQUEST);
        let recorded = rejection.audit.to_string();
        assert!(recorded.len() < 200 && !recorded.contains("xxxx"), "the audit keeps no text from the request: {recorded}");
    }

    #[test]
    fn an_oversized_legacy_registration_is_refused() {
        let body = serde_json::json!({ "device_token": "a".repeat(900 * 1024), "platform": "android", "node_id": NODE }).to_string();
        let rejection = parse_legacy_registration(body.as_bytes()).err().expect("the token is far too long");
        assert_eq!(rejection.audit["field"], "device_token");
        let shipped = serde_json::json!({ "device_token": "ab".repeat(32), "platform": "ios", "node_id": NODE, "environment": "production" }).to_string();
        assert!(parse_legacy_registration(shipped.as_bytes()).is_ok(), "what released apps send still registers");
    }

    #[test]
    fn legacy_body_rejects_missing_required_fields() {
        assert!(serde_json::from_str::<LegacyPushRequest>(r#"{"platform":"ios"}"#).is_err());
        assert!(serde_json::from_str::<LegacyPushRequest>(r#"{"device_token":"tok"}"#).is_err());
    }
}
