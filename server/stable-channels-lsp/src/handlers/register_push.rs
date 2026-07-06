//! RegisterPush handler: wallets POST a signed APNs/FCM device token and Lightning node id, stored in push_tokens.db.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;

use ldk_server_client::ldk_server_grpc::api::VerifySignatureRequest;
use sc_protos::stable::{RegisterPushRequest, RegisterPushResponse};

use crate::handlers::{decode_body, ok_response};
use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

/// Freshness window for the signed timestamp, in seconds.
const PUSH_SIG_WINDOW_SECS: u64 = 300;

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

    {
        let push = state.push.lock().await;
        push.register_token(&req.token, &req.platform, &req.node_id, &req.environment);
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

/// Legacy push registration: deployed wallets POST unsigned JSON to /api/register-push
/// (Apache-proxied, no HMAC, no node-ownership proof). Mounted OUTSIDE the auth layer.
/// Response shape mirrors the fork byte-for-byte: 200 `"ok"` or 400 with the parse error.
/// Remove once every install signs registrations (see verify_push_registration).
pub async fn register_push_legacy(State(state): State<AppState>, body: Bytes) -> Response {
    let req: LegacyPushRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            stable_channels::audit::audit_event(
                "REGISTER_PUSH_LEGACY_INVALID",
                serde_json::json!({ "error": e.to_string() }),
            );
            return Response::builder()
                .status(axum::http::StatusCode::BAD_REQUEST)
                .header("content-type", "text/plain")
                .body(axum::body::Body::from(e.to_string()))
                .unwrap();
        }
    };

    {
        let push = state.push.lock().await;
        push.register_token(
            &req.device_token,
            &req.platform,
            &req.node_id,
            &req.environment,
        );
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

    #[test]
    fn legacy_body_rejects_missing_required_fields() {
        assert!(serde_json::from_str::<LegacyPushRequest>(r#"{"platform":"ios"}"#).is_err());
        assert!(serde_json::from_str::<LegacyPushRequest>(r#"{"device_token":"tok"}"#).is_err());
    }
}
