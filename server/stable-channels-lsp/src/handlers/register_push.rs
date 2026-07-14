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
        push.register_token(&req.token, &req.platform, &req.node_id, &req.environment, true);
    }
    stable_channels::audit::audit_event(
        "REGISTER_PUSH_OK",
        serde_json::json!({ "node_id": req.node_id }),
    );
    ok_response(RegisterPushResponse { ok: true })
}

/// Short, non-reversible fingerprint of a device token for audit logs, so a
/// token change is traceable without persisting the raw token in the clear.
fn token_fp(token: &str) -> String {
    use bitcoin_hashes::{sha256, Hash};
    let h = sha256::Hash::hash(token.as_bytes());
    format!("{:x}", h)[..12].to_string()
}

/// The registration body deployed wallet builds send. Historically unsigned
/// (fork contract); `signature`/`timestamp` are optional so a signing-capable
/// wallet can prove node ownership over the SAME route (the signed proto route
/// sits behind HMAC and is unreachable from mobile). See issue #162.
#[derive(serde::Deserialize)]
pub struct LegacyPushRequest {
    pub device_token: String,
    pub platform: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub environment: String,
    /// zbase32 secp256k1 signature over register_push_signed_bytes(node_id, device_token, timestamp).
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

/// Legacy push registration: deployed wallets POST JSON to /api/register-push
/// (Apache-proxied, no HMAC). Mounted OUTSIDE the auth layer so mobile can reach it.
/// If a valid node signature is present the token is stored as VERIFIED and wins
/// over any unsigned token for the node; otherwise it is stored unverified (and
/// can never override a verified token). Response shape mirrors the fork
/// byte-for-byte: 200 `"ok"` or 400 with the parse error.
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

    // If a signature is supplied, verify it; a valid one upgrades this token to
    // verified. Absent/invalid signatures fall back to the unsigned path.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let verified = match (&req.signature, req.timestamp) {
        (Some(sig), Some(ts)) => {
            let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
            verify_push_registration(
                ldk,
                &req.node_id,
                &req.device_token,
                sig,
                ts,
                now,
                PUSH_SIG_WINDOW_SECS,
            )
            .await
        }
        _ => false,
    };
    // A signature was presented but did not verify — flag it, still store unsigned.
    if req.signature.is_some() && !verified {
        stable_channels::audit::audit_event(
            "REGISTER_PUSH_SIGNATURE_INVALID",
            serde_json::json!({ "node_id": req.node_id, "route": "legacy" }),
        );
    }

    // Detect a token change / hijack attempt before writing.
    {
        let push = state.push.lock().await;
        let prior = push.node_token_state(&req.node_id);
        if let Some(active) = prior.active_token.as_deref() {
            if active != req.device_token {
                // A different token is being registered for a node that already
                // has one. Unsigned-over-verified can never win (load prefers
                // verified), but surface it so a targeted hijack is visible.
                let hijack_blocked = prior.has_verified && !verified;
                stable_channels::audit::audit_event(
                    if hijack_blocked {
                        "REGISTER_PUSH_HIJACK_BLOCKED"
                    } else {
                        "REGISTER_PUSH_TOKEN_CHANGED"
                    },
                    serde_json::json!({
                        "node_id": req.node_id,
                        "platform": req.platform,
                        "old_token_fp": token_fp(active),
                        "new_token_fp": token_fp(&req.device_token),
                        "new_verified": verified,
                        "had_verified": prior.has_verified,
                    }),
                );
            }
        }
        push.register_token(
            &req.device_token,
            &req.platform,
            &req.node_id,
            &req.environment,
            verified,
        );
    }

    stable_channels::audit::audit_event(
        if verified { "REGISTER_PUSH_SIGNED_OK" } else { "REGISTER_PUSH_LEGACY_OK" },
        serde_json::json!({ "node_id": req.node_id, "platform": req.platform, "verified": verified }),
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
        // Existing unsigned builds send no signature — stays the unsigned path.
        assert!(minimal.signature.is_none());
        assert!(minimal.timestamp.is_none());
    }

    #[test]
    fn legacy_body_parses_optional_signature() {
        let signed: LegacyPushRequest = serde_json::from_str(
            r#"{"device_token":"tok","platform":"ios","node_id":"02ab","signature":"zsig","timestamp":1717000000}"#,
        )
        .unwrap();
        assert_eq!(signed.signature.as_deref(), Some("zsig"));
        assert_eq!(signed.timestamp, Some(1717000000));
    }

    #[test]
    fn token_fp_is_stable_and_truncated() {
        let a = token_fp("device-token-abc");
        let b = token_fp("device-token-abc");
        assert_eq!(a, b);
        assert_eq!(a.len(), 12);
        assert_ne!(a, token_fp("device-token-xyz"));
    }

    #[test]
    fn legacy_body_rejects_missing_required_fields() {
        assert!(serde_json::from_str::<LegacyPushRequest>(r#"{"platform":"ios"}"#).is_err());
        assert!(serde_json::from_str::<LegacyPushRequest>(r#"{"device_token":"tok"}"#).is_err());
    }
}
