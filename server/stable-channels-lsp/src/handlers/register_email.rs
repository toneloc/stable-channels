//! RegisterEmail handler: wallets POST a signed envelope to share, change or remove an optional email, stored in emails.db.
//! Unsigned by HMAC like the other wallet routes: the node-key signature proves which node sent it, so a wallet with no channel can still use it.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;

use crate::emails::{Emails, SaveOutcome};
use crate::handlers::{is_valid_node_id, verify_node_signature};
use crate::messages::SignedEnvelope;
use crate::stable_manager::LdkServerCalls;
use crate::state::AppState;

const REGISTER_EMAIL_TYPE: &str = "REGISTER_EMAIL_V1";
/// Freshness window for the signed timestamp, in seconds.
const EMAIL_SIG_WINDOW_SECS: u64 = 300;
const MAX_EMAIL_LEN: usize = 254;
const MAX_EMAIL_LOCAL_LEN: usize = 64;

/// What the caller is told, and the audit record, which never keeps the email or any other text from the request.
#[derive(Debug)]
pub struct Reply {
    pub status: StatusCode,
    pub body: serde_json::Value,
    pub audit: Option<(&'static str, serde_json::Value)>,
}

fn invalid(field: &'static str) -> Reply {
    Reply {
        status: StatusCode::BAD_REQUEST,
        body: serde_json::json!({ "error": format!("invalid {field}") }),
        audit: Some((
            "REGISTER_EMAIL_INVALID",
            serde_json::json!({ "field": field }),
        )),
    }
}

fn unauthorized(node_id: &str, reason: &str) -> Reply {
    Reply {
        status: StatusCode::UNAUTHORIZED,
        body: serde_json::json!({ "error": "invalid signature" }),
        audit: Some((
            "REGISTER_EMAIL_SIGNATURE_INVALID",
            serde_json::json!({ "node_id": node_id, "reason": reason }),
        )),
    }
}

fn unavailable() -> Reply {
    Reply {
        status: StatusCode::SERVICE_UNAVAILABLE,
        body: serde_json::json!({ "error": "unavailable" }),
        audit: None,
    }
}

/// Refused by the cap or the hourly limit; recorded so a flood of new keys shows up in the audit log.
fn held_back(node_id: &str, reason: &str) -> Reply {
    Reply {
        audit: Some((
            "REGISTER_EMAIL_LIMITED",
            serde_json::json!({ "node_id": node_id, "reason": reason }),
        )),
        ..unavailable()
    }
}

/// A shareable address, already lowercased by the wallet: a dot-separated local part of plain address characters, '@', and a
/// dotted ASCII domain with a TLD of two or more characters. Quotes, markup and non-ASCII are refused so a stored address is
/// safe to show or mail as is.
pub fn is_valid_email(email: &str) -> bool {
    if email.len() > MAX_EMAIL_LEN {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    let local_char =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "!#$%&'*+/=?^_`{|}~-".contains(c);
    let local_ok = local.len() <= MAX_EMAIL_LOCAL_LEN
        && local
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(local_char));
    let label_ok = |l: &str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    let labels: Vec<&str> = domain.split('.').collect();
    local_ok
        && labels.len() >= 2
        && labels.iter().all(|l| label_ok(l))
        && labels.last().is_some_and(|tld| tld.len() >= 2)
}

/// Check a signed email change and store it; every refusal is decided before anything is stored.
/// `has_channel` says whether a node has a channel with the LSP; those nodes skip the cap and the hourly limit.
pub async fn register(
    ldk: &dyn LdkServerCalls,
    emails: &Emails,
    has_channel: &(dyn Fn(&str) -> bool + Sync),
    body: &[u8],
    now: u64,
) -> Reply {
    let Ok(envelope) = serde_json::from_slice::<SignedEnvelope>(body) else {
        return invalid("envelope");
    };
    let Ok(serde_json::Value::Object(payload)) =
        serde_json::from_str::<serde_json::Value>(&envelope.payload)
    else {
        return invalid("payload");
    };
    if payload.get("type").and_then(|v| v.as_str()) != Some(REGISTER_EMAIL_TYPE) {
        return invalid("type");
    }
    let Some(node_id) = payload
        .get("node_id")
        .and_then(|v| v.as_str())
        .filter(|n| is_valid_node_id(n))
    else {
        return invalid("node_id");
    };
    let Some(email) = payload
        .get("email")
        .and_then(|v| v.as_str())
        .filter(|e| e.is_empty() || is_valid_email(e))
    else {
        return invalid("email");
    };
    let Some(ts) = payload.get("ts").and_then(|v| v.as_u64()) else {
        return invalid("ts");
    };
    if now.abs_diff(ts) > EMAIL_SIG_WINDOW_SECS {
        return unauthorized(node_id, "stale");
    }
    match verify_node_signature(
        ldk,
        node_id,
        envelope.payload.as_bytes().to_vec(),
        &envelope.signature,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return unauthorized(node_id, "signature"),
        Err(_) => return unavailable(),
    }
    // Lowercase is the form every other node-keyed store uses.
    let node_id = node_id.to_ascii_lowercase();
    // The window check above keeps ts far below i64::MAX.
    match emails
        .save(&node_id, email, ts as i64, has_channel(&node_id))
        .await
    {
        SaveOutcome::Saved => Reply {
            status: StatusCode::OK,
            body: serde_json::json!({ "ok": true }),
            audit: Some((
                "REGISTER_EMAIL_OK",
                serde_json::json!({ "node_id": node_id, "removed": email.is_empty() }),
            )),
        },
        SaveOutcome::Stale => unauthorized(&node_id, "replay"),
        SaveOutcome::Full => held_back(&node_id, "full"),
        SaveOutcome::Limited => held_back(&node_id, "hourly_limit"),
        SaveOutcome::Failed => unavailable(),
    }
}

/// POST /api/register-email. Mounted OUTSIDE the auth layer; see the module docs.
pub async fn register_email(State(state): State<AppState>, body: Bytes) -> Response {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ldk = state.ldk_server.as_ref() as &dyn LdkServerCalls;
    let registrations = &state.push_registrations;
    let Some(emails) = state.emails.as_deref() else {
        return respond(unavailable());
    };
    let reply = register(
        ldk,
        emails,
        &|node| registrations.has_channel(node),
        &body,
        now,
    )
    .await;
    respond(reply)
}

/// Record the audit event, if any, and send the JSON reply.
fn respond(reply: Reply) -> Response {
    if let Some((event, data)) = reply.audit {
        stable_channels::audit::audit_event(event, data);
    }
    Response::builder()
        .status(reply.status)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(reply.body.to_string()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::register_push::tests::VerifyFake;
    use ldk_server_client::error::LdkServerError;
    use ldk_server_client::ldk_server_grpc::api::{
        ListChannelsRequest, ListChannelsResponse, SignMessageRequest, SignMessageResponse,
        SpontaneousSendRequest, SpontaneousSendResponse, VerifySignatureRequest,
        VerifySignatureResponse,
    };

    const NODE: &str = "02465ed5be53d04fde66c9418ff14a5f2267723810176c9212b722e542dc1afb1b";
    const NOW: u64 = 1_762_790_000;
    const EMAIL: &str = "secret.person@example.com";

    fn body(email: &str, node_id: &str, ts: u64) -> Vec<u8> {
        let payload = serde_json::json!({ "email": email, "node_id": node_id, "ts": ts, "type": REGISTER_EMAIL_TYPE }).to_string();
        serde_json::json!({ "payload": payload, "signature": "sig" })
            .to_string()
            .into_bytes()
    }

    async fn send(valid: Option<bool>, emails: &Emails, body: &[u8]) -> Reply {
        register(&VerifyFake { valid }, emails, &|_| false, body, NOW).await
    }

    fn error_of(reply: &Reply) -> &str {
        reply.body["error"].as_str().unwrap_or_default()
    }

    #[test]
    fn email_rules() {
        for ok in [
            "a@b.co",
            "you@example.com",
            "first.last+tag@mail.example.org",
            &format!("{}@example.com", "a".repeat(64)),
        ] {
            assert!(is_valid_email(ok), "{ok}");
        }
        let too_long = format!("a@{}.com", "b".repeat(250));
        let long_local = format!("{}@example.com", "a".repeat(65));
        for bad in [
            "",
            "a",
            "@example.com",
            "a@",
            "a@b",
            "a@b.c",
            "a@@b.co",
            "a@b@c.co",
            "a@.co",
            "a@b..co",
            "a@b.co.",
            " a@b.co",
            "a@b.co ",
            "a b@c.co",
            "a@b\u{7}.co",
            "A@b.co",
            "a@B.co",
            &too_long,
            &long_local,
        ] {
            assert!(!is_valid_email(bad), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn malformed_requests_name_the_field() {
        let emails = Emails::in_memory(10);
        let payload = |p: serde_json::Value| {
            serde_json::json!({ "payload": p.to_string(), "signature": "sig" })
                .to_string()
                .into_bytes()
        };
        let cases: Vec<(Vec<u8>, &str)> = vec![
            (b"not json".to_vec(), "envelope"),
            (serde_json::to_vec(&"x".repeat(1024)).unwrap(), "envelope"),
            (br#"{"payload":"{}"}"#.to_vec(), "envelope"),
            (
                br#"{"payload":"not json","signature":"sig"}"#.to_vec(),
                "payload",
            ),
            (payload(serde_json::json!([1])), "payload"),
            (
                payload(
                    serde_json::json!({ "email": EMAIL, "node_id": NODE, "ts": NOW, "type": "REGISTER_PUSH_V1" }),
                ),
                "type",
            ),
            (
                payload(serde_json::json!({ "email": EMAIL, "node_id": NODE, "ts": NOW })),
                "type",
            ),
            (body(EMAIL, "02ab", NOW), "node_id"),
            (body(EMAIL, &format!("04{}", &NODE[2..]), NOW), "node_id"),
            (body("Secret@example.com", NODE, NOW), "email"),
            (body("secret", NODE, NOW), "email"),
            (
                payload(
                    serde_json::json!({ "node_id": NODE, "ts": NOW, "type": REGISTER_EMAIL_TYPE }),
                ),
                "email",
            ),
            (
                payload(
                    serde_json::json!({ "email": EMAIL, "node_id": NODE, "ts": "now", "type": REGISTER_EMAIL_TYPE }),
                ),
                "ts",
            ),
            (
                payload(
                    serde_json::json!({ "email": EMAIL, "node_id": NODE, "ts": -1, "type": REGISTER_EMAIL_TYPE }),
                ),
                "ts",
            ),
        ];
        for (body, field) in cases {
            let reply = send(Some(true), &emails, &body).await;
            assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{field}");
            assert_eq!(error_of(&reply), format!("invalid {field}"));
            assert_eq!(
                reply.audit.unwrap().1,
                serde_json::json!({ "field": field })
            );
        }
        assert_eq!(
            emails.email_of(NODE).await,
            None,
            "nothing refused is stored"
        );
    }

    #[tokio::test]
    async fn a_signed_fresh_request_saves_and_removes() {
        let emails = Emails::in_memory(10);
        let reply = send(Some(true), &emails, &body(EMAIL, NODE, NOW)).await;
        assert_eq!(
            (reply.status, reply.body),
            (StatusCode::OK, serde_json::json!({ "ok": true }))
        );
        assert_eq!(
            reply.audit.unwrap(),
            (
                "REGISTER_EMAIL_OK",
                serde_json::json!({ "node_id": NODE, "removed": false })
            )
        );
        assert_eq!(emails.email_of(NODE).await.as_deref(), Some(EMAIL));

        let reply = send(Some(true), &emails, &body("", NODE, NOW + 1)).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.audit.unwrap().1["removed"], true);
        assert_eq!(emails.email_of(NODE).await.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn a_bad_signature_or_stale_timestamp_is_refused() {
        let emails = Emails::in_memory(10);
        for (valid, ts, reason) in [
            (Some(false), NOW, "signature"),
            (Some(true), NOW - 301, "stale"),
            (Some(true), NOW + 301, "stale"),
        ] {
            let reply = send(valid, &emails, &body(EMAIL, NODE, ts)).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
            assert_eq!(error_of(&reply), "invalid signature");
            assert_eq!(
                reply.audit.unwrap(),
                (
                    "REGISTER_EMAIL_SIGNATURE_INVALID",
                    serde_json::json!({ "node_id": NODE, "reason": reason })
                )
            );
        }
        assert_eq!(
            send(Some(true), &emails, &body(EMAIL, NODE, NOW - 300))
                .await
                .status,
            StatusCode::OK,
            "the window edge is inside"
        );
        let reply = send(None, &emails, &body(EMAIL, NODE, NOW)).await;
        assert_eq!(
            (reply.status, error_of(&reply)),
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        );
    }

    #[tokio::test]
    async fn a_replayed_save_cannot_undo_a_removal() {
        let emails = Emails::in_memory(10);
        let save = body(EMAIL, NODE, NOW - 10);
        assert_eq!(
            send(Some(true), &emails, &save).await.status,
            StatusCode::OK
        );
        assert_eq!(
            send(Some(true), &emails, &body("", NODE, NOW)).await.status,
            StatusCode::OK
        );
        let reply = send(Some(true), &emails, &save).await;
        assert_eq!(
            (reply.status, error_of(&reply)),
            (StatusCode::UNAUTHORIZED, "invalid signature")
        );
        assert_eq!(reply.audit.unwrap().1["reason"], "replay");
        assert_eq!(emails.email_of(NODE).await.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn a_full_store_refuses_a_new_node_but_updates_a_stored_one() {
        let emails = Emails::in_memory(1);
        assert_eq!(
            send(Some(true), &emails, &body(EMAIL, NODE, NOW - 1))
                .await
                .status,
            StatusCode::OK
        );
        let other = format!("03{}", &NODE[2..]);
        let reply = send(Some(true), &emails, &body(EMAIL, &other, NOW)).await;
        assert_eq!(
            (reply.status, error_of(&reply)),
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        );
        assert_eq!(
            send(Some(true), &emails, &body("new@example.com", NODE, NOW))
                .await
                .status,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn when_full_a_new_node_is_held_back_but_a_customer_still_saves() {
        let emails = Emails::in_memory(1);
        assert_eq!(
            send(Some(true), &emails, &body(EMAIL, NODE, NOW))
                .await
                .status,
            StatusCode::OK
        );
        let stranger = format!("03{}", &NODE[2..]);
        let reply = send(Some(true), &emails, &body(EMAIL, &stranger, NOW)).await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            reply.audit.unwrap(),
            (
                "REGISTER_EMAIL_LIMITED",
                serde_json::json!({ "node_id": stranger, "reason": "full" })
            )
        );

        let customer = |node: &str| node == stranger;
        let reply = register(
            &VerifyFake { valid: Some(true) },
            &emails,
            &customer,
            &body(EMAIL, &stranger, NOW),
            NOW,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn no_reply_or_audit_record_contains_the_email() {
        let emails = Emails::in_memory(1);
        let other = format!("03{}", &NODE[2..]);
        let bodies = [
            body(EMAIL, NODE, NOW - 1),
            body(EMAIL, NODE, NOW - 1),
            body(EMAIL, &other, NOW),
            body(EMAIL, "02ab", NOW),
            body(&EMAIL.to_uppercase(), NODE, NOW),
            body(EMAIL, NODE, NOW - 1000),
            format!("{{\"payload\":\"{EMAIL}\"}}").into_bytes(),
        ];
        for valid in [Some(true), Some(false), None] {
            for body in &bodies {
                let reply = send(valid, &emails, body).await;
                let recorded = format!("{reply:?}").to_lowercase();
                assert!(!recorded.contains("secret.person"), "{recorded}");
            }
        }
    }

    /// Verifies like LDK Server would: true only for the exact bytes this node signed.
    struct NodeKeyFake {
        node_id: String,
        signed: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl LdkServerCalls for NodeKeyFake {
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
            Ok(SpontaneousSendResponse {
                payment_id: String::new(),
            })
        }
        async fn sign_message(
            &self,
            _req: SignMessageRequest,
        ) -> Result<SignMessageResponse, LdkServerError> {
            Ok(SignMessageResponse {
                signature: String::new(),
            })
        }
        async fn verify_signature(
            &self,
            req: VerifySignatureRequest,
        ) -> Result<VerifySignatureResponse, LdkServerError> {
            let valid = req.public_key.eq_ignore_ascii_case(&self.node_id)
                && req.message.as_ref() == self.signed.as_slice();
            Ok(VerifySignatureResponse { valid })
        }
    }

    fn payload_of(body: &[u8]) -> Vec<u8> {
        let envelope: SignedEnvelope = serde_json::from_slice(body).unwrap();
        envelope.payload.into_bytes()
    }

    #[tokio::test]
    async fn a_signature_only_counts_for_its_own_node_and_payload() {
        let emails = Emails::in_memory(10);
        let signed = body(EMAIL, NODE, NOW);
        let ldk = NodeKeyFake {
            node_id: NODE.to_string(),
            signed: payload_of(&signed),
        };
        assert_eq!(
            register(&ldk, &emails, &|_| false, &signed, NOW)
                .await
                .status,
            StatusCode::OK
        );

        // Same signature, someone else's node id: the key does not match.
        let other = format!("03{}", &NODE[2..]);
        let stolen = serde_json::json!({ "payload": String::from_utf8(payload_of(&body(EMAIL, &other, NOW))).unwrap(), "signature": "sig" });
        let reply = register(
            &ldk,
            &emails,
            &|_| false,
            stolen.to_string().as_bytes(),
            NOW,
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(emails.email_of(&other).await, None);

        // Same node, a different address than the one signed.
        let swapped = body("attacker@example.com", NODE, NOW + 1);
        assert_eq!(
            register(&ldk, &emails, &|_| false, &swapped, NOW)
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(emails.email_of(NODE).await.as_deref(), Some(EMAIL));
    }

    // The exact payload the iOS and Android tests assert; the signature must cover these bytes as sent.
    const APP_PAYLOAD: &str = r#"{"email":"you@example.com","node_id":"02eec7245d6b7d2ccb30380bfbe2a3648cd7a942653f5aa340edcea1f283686619","ts":1762790000,"type":"REGISTER_EMAIL_V1"}"#;

    #[tokio::test]
    async fn the_payload_the_apps_send_is_verified_over_its_exact_bytes() {
        let emails = Emails::in_memory(10);
        let node_id = "02eec7245d6b7d2ccb30380bfbe2a3648cd7a942653f5aa340edcea1f283686619";
        let ldk = NodeKeyFake {
            node_id: node_id.to_string(),
            signed: APP_PAYLOAD.as_bytes().to_vec(),
        };
        let body = serde_json::json!({ "payload": APP_PAYLOAD, "signature": "sig" }).to_string();
        let reply = register(&ldk, &emails, &|_| false, body.as_bytes(), 1_762_790_000).await;
        assert_eq!(
            (reply.status, reply.body),
            (StatusCode::OK, serde_json::json!({ "ok": true }))
        );
        assert_eq!(
            emails.email_of(node_id).await.as_deref(),
            Some("you@example.com")
        );
    }

    #[tokio::test]
    async fn an_uppercase_node_id_hits_the_same_replay_guard() {
        let emails = Emails::in_memory(10);
        assert_eq!(
            send(Some(true), &emails, &body("", NODE, NOW)).await.status,
            StatusCode::OK
        );
        let reply = send(Some(true), &emails, &body(EMAIL, &NODE.to_uppercase(), NOW)).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.audit.unwrap().1["reason"], "replay");
        assert_eq!(emails.email_of(NODE).await.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn an_extreme_timestamp_is_refused_without_overflow() {
        let emails = Emails::in_memory(10);
        for ts in [u64::MAX, i64::MAX as u64 + 1, 0] {
            let reply = send(Some(true), &emails, &body(EMAIL, NODE, ts)).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{ts}");
        }
        assert_eq!(emails.email_of(NODE).await, None);
    }

    #[test]
    fn an_email_with_markup_quotes_or_header_breaks_is_refused() {
        for bad in [
            "a<b@example.com",
            "a>b@example.com",
            "\"a\"@example.com",
            "a@exa<mple.com",
            "a@example.com/x",
            "a@exa_mple.com",
            "a@-example.com",
            "a@example-.com",
            "a\\b@example.com",
            "a@b.co\r\nbcc: x@y.co",
            "ä@example.com",
            "a@exämple.com",
            ".a@example.com",
            "a.@example.com",
            "a..b@example.com",
        ] {
            assert!(!is_valid_email(bad), "{bad:?}");
        }
        for ok in [
            "o'brien@example.com",
            "a_b-c+d@sub-domain.example.co",
            "x@xn--bcher-kva.example",
        ] {
            assert!(is_valid_email(ok), "{ok}");
        }
    }
}
