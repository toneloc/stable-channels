//! RegisterEmail handler: wallets POST a signed envelope to share, change or remove an optional email, stored in emails.db.
//! Unsigned by HMAC like the other wallet routes: the node-key signature proves which node sent it, so a wallet with no channel can still use it.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use ldk_node::lightning::util::message_signing;

use crate::emails::{EmailAddress, Emails, SaveOutcome};
use crate::handlers::{is_plausible_signature, parse_node_key};
use crate::messages::SignedEnvelope;
use crate::state::AppState;

const REGISTER_EMAIL_TYPE: &str = "REGISTER_EMAIL_V1";
/// Freshness window for the signed timestamp, in seconds.
const EMAIL_SIG_WINDOW_SECS: u64 = 300;
/// Longest signed payload accepted; real ones are a few hundred bytes, so a bigger one is refused before any parse or verify.
const MAX_PAYLOAD_LEN: usize = 1024;

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

/// Check a signed email change and store it; every refusal is decided before anything is stored.
/// `has_channel` says whether a node has a channel with the LSP; those nodes skip the cap and the hourly limit.
pub async fn register(
    emails: &Emails,
    has_channel: &(dyn Fn(&str) -> bool + Sync),
    body: &[u8],
    now: u64,
) -> Reply {
    let Ok(envelope) = serde_json::from_slice::<SignedEnvelope>(body) else {
        return invalid("envelope");
    };
    if envelope.payload.len() > MAX_PAYLOAD_LEN {
        return invalid("payload");
    }
    if !is_plausible_signature(&envelope.signature) {
        return invalid("signature");
    }
    let Ok(serde_json::Value::Object(payload)) =
        serde_json::from_str::<serde_json::Value>(&envelope.payload)
    else {
        return invalid("payload");
    };
    if payload.get("type").and_then(|v| v.as_str()) != Some(REGISTER_EMAIL_TYPE) {
        return invalid("type");
    }
    let Some((node_id, public_key)) = payload
        .get("node_id")
        .and_then(|v| v.as_str())
        .and_then(|n| parse_node_key(n).map(|key| (n, key)))
    else {
        return invalid("node_id");
    };
    // An empty email asks to remove the address.
    let Some(email) = payload
        .get("email")
        .and_then(|v| v.as_str())
        .and_then(|e| match e {
            "" => Some(None),
            e => EmailAddress::parse(e).map(Some),
        })
    else {
        return invalid("email");
    };
    let Some(ts) = payload.get("ts").and_then(|v| v.as_u64()) else {
        return invalid("ts");
    };
    if now.abs_diff(ts) > EMAIL_SIG_WINDOW_SECS {
        return unauthorized(node_id, "stale");
    }
    // Checked here with LDK's own message signing, so unsigned traffic costs LDK Server nothing and the route works while it is down.
    if !message_signing::verify(
        envelope.payload.as_bytes(),
        &envelope.signature,
        &public_key,
    ) {
        return unauthorized(node_id, "signature");
    }
    // Lowercase is the form every other node-keyed store uses.
    let node_id = node_id.to_ascii_lowercase();
    let removed = email.is_none();
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
                serde_json::json!({ "node_id": node_id, "removed": removed }),
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
    // The clock is never before 1970, so unsigned seconds lose nothing.
    let now = u64::try_from(crate::revenue::now_secs()).unwrap_or(0);
    let registrations = &state.push_registrations;
    let Some(emails) = state.emails.as_deref() else {
        return respond(unavailable());
    };
    let reply = register(emails, &|node| registrations.has_channel(node), &body, now).await;
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
    use crate::handlers::MAX_SIGNATURE_CHARS;
    use ldk_node::bitcoin::secp256k1::SecretKey;

    /// The node id of the test key [2; 32].
    const NODE: &str = "024d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766";
    const NOW: u64 = 1_762_790_000;
    const EMAIL: &str = "secret.person@example.com";

    /// The key behind `node_id`: NODE's, or its negation for the "03" node with the same x; any other id gets NODE's key.
    fn key_of(node_id: &str) -> SecretKey {
        let key = SecretKey::from_slice(&[2; 32]).unwrap();
        if node_id.eq_ignore_ascii_case(&format!("03{}", &NODE[2..])) {
            key.negate()
        } else {
            key
        }
    }

    fn envelope(payload: &str, signature: &str) -> Vec<u8> {
        serde_json::json!({ "payload": payload, "signature": signature })
            .to_string()
            .into_bytes()
    }

    fn payload(email: &str, node_id: &str, ts: u64) -> String {
        serde_json::json!({ "email": email, "node_id": node_id, "ts": ts, "type": REGISTER_EMAIL_TYPE }).to_string()
    }

    /// A request signed by the key behind `node_id`.
    fn body(email: &str, node_id: &str, ts: u64) -> Vec<u8> {
        let payload = payload(email, node_id, ts);
        envelope(
            &payload,
            &message_signing::sign(payload.as_bytes(), &key_of(node_id)),
        )
    }

    /// A request signed by a key that is not behind `node_id`.
    fn forged(email: &str, node_id: &str, ts: u64) -> Vec<u8> {
        let payload = payload(email, node_id, ts);
        let other = SecretKey::from_slice(&[3; 32]).unwrap();
        envelope(&payload, &message_signing::sign(payload.as_bytes(), &other))
    }

    async fn send(emails: &Emails, body: &[u8]) -> Reply {
        register(emails, &|_| false, body, NOW).await
    }

    fn error_of(reply: &Reply) -> &str {
        reply.body["error"].as_str().unwrap_or_default()
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
            (
                body(EMAIL, &format!("02{}", "f".repeat(64)), NOW),
                "node_id",
            ),
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
            let reply = send(&emails, &body).await;
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
        let reply = send(&emails, &body(EMAIL, NODE, NOW)).await;
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

        let reply = send(&emails, &body("", NODE, NOW + 1)).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.audit.unwrap().1["removed"], true);
        assert_eq!(emails.email_of(NODE).await.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn a_bad_signature_or_stale_timestamp_is_refused() {
        let emails = Emails::in_memory(10);
        let junk = envelope(
            &payload(EMAIL, NODE, NOW),
            "d9o8ftxzs3ggfpzwfkqbjzb7rsh1bs5wb5oy9ji8ayahbqzr6rksi4ff4jcb94mgfujbhp3e6z86qucb9mt1iw4cdw7aqyw4bksfb5r",
        );
        for (body, reason) in [
            (forged(EMAIL, NODE, NOW), "signature"),
            (junk, "signature"),
            (body(EMAIL, NODE, NOW - 301), "stale"),
            (body(EMAIL, NODE, NOW + 301), "stale"),
        ] {
            let reply = send(&emails, &body).await;
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
            send(&emails, &body(EMAIL, NODE, NOW - 300)).await.status,
            StatusCode::OK,
            "the window edge is inside"
        );
    }

    #[tokio::test]
    async fn an_empty_or_overlong_signature_is_refused_before_it_is_verified() {
        let emails = Emails::in_memory(10);
        let payload = payload(EMAIL, NODE, NOW);
        for (signature, status, event) in [
            (
                String::new(),
                StatusCode::BAD_REQUEST,
                "REGISTER_EMAIL_INVALID",
            ),
            (
                "x".repeat(MAX_SIGNATURE_CHARS + 1),
                StatusCode::BAD_REQUEST,
                "REGISTER_EMAIL_INVALID",
            ),
            (
                "x".repeat(MAX_SIGNATURE_CHARS),
                StatusCode::UNAUTHORIZED,
                "REGISTER_EMAIL_SIGNATURE_INVALID",
            ),
        ] {
            let reply = send(&emails, &envelope(&payload, &signature)).await;
            assert_eq!(reply.status, status, "{}", signature.len());
            assert_eq!(error_of(&reply), "invalid signature");
            let (name, data) = reply.audit.unwrap();
            assert_eq!(name, event);
            if status == StatusCode::BAD_REQUEST {
                assert_eq!(data, serde_json::json!({ "field": "signature" }));
            }
        }
        assert_eq!(emails.email_of(NODE).await, None);
    }

    #[tokio::test]
    async fn a_replayed_save_cannot_undo_a_removal() {
        let emails = Emails::in_memory(10);
        let save = body(EMAIL, NODE, NOW - 10);
        assert_eq!(send(&emails, &save).await.status, StatusCode::OK);
        assert_eq!(
            send(&emails, &body("", NODE, NOW)).await.status,
            StatusCode::OK
        );
        let reply = send(&emails, &save).await;
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
            send(&emails, &body(EMAIL, NODE, NOW - 1)).await.status,
            StatusCode::OK
        );
        let other = format!("03{}", &NODE[2..]);
        let reply = send(&emails, &body(EMAIL, &other, NOW)).await;
        assert_eq!(
            (reply.status, error_of(&reply)),
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        );
        assert_eq!(
            send(&emails, &body("new@example.com", NODE, NOW))
                .await
                .status,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn when_full_a_new_node_is_held_back_but_a_customer_still_saves() {
        let emails = Emails::in_memory(1);
        assert_eq!(
            send(&emails, &body(EMAIL, NODE, NOW)).await.status,
            StatusCode::OK
        );
        let stranger = format!("03{}", &NODE[2..]);
        let reply = send(&emails, &body(EMAIL, &stranger, NOW)).await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            reply.audit.unwrap(),
            (
                "REGISTER_EMAIL_LIMITED",
                serde_json::json!({ "node_id": stranger, "reason": "full" })
            )
        );

        let customer = |node: &str| node == stranger;
        let reply = register(&emails, &customer, &body(EMAIL, &stranger, NOW), NOW).await;
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
            forged(EMAIL, NODE, NOW),
            envelope(&payload(EMAIL, NODE, NOW), ""),
        ];
        for body in &bodies {
            let reply = send(&emails, body).await;
            let recorded = format!("{reply:?}").to_lowercase();
            assert!(!recorded.contains("secret.person"), "{recorded}");
        }
    }

    fn parts_of(body: &[u8]) -> (String, String) {
        let envelope: SignedEnvelope = serde_json::from_slice(body).unwrap();
        (envelope.payload, envelope.signature)
    }

    #[tokio::test]
    async fn a_signature_only_counts_for_its_own_node_and_payload() {
        let emails = Emails::in_memory(10);
        let signed = body(EMAIL, NODE, NOW);
        assert_eq!(send(&emails, &signed).await.status, StatusCode::OK);
        let (_, signature) = parts_of(&signed);

        // Same signature, someone else's node id: the key does not match.
        let other = format!("03{}", &NODE[2..]);
        let stolen = envelope(&payload(EMAIL, &other, NOW), &signature);
        assert_eq!(
            send(&emails, &stolen).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(emails.email_of(&other).await, None);

        // Same node, a different address than the one signed.
        let swapped = envelope(&payload("attacker@example.com", NODE, NOW + 1), &signature);
        assert_eq!(
            send(&emails, &swapped).await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(emails.email_of(NODE).await.as_deref(), Some(EMAIL));
    }

    // The form the wallets sign: sorted-key JSON {"email","node_id","ts","type"}; the signature must cover these bytes as sent.
    const APP_PAYLOAD: &str = r#"{"email":"you@example.com","node_id":"024d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766","ts":1762790000,"type":"REGISTER_EMAIL_V1"}"#;

    #[tokio::test]
    async fn the_payload_the_apps_send_is_verified_over_its_exact_bytes() {
        let emails = Emails::in_memory(10);
        let signature = message_signing::sign(APP_PAYLOAD.as_bytes(), &key_of(NODE));
        let reply = send(&emails, &envelope(APP_PAYLOAD, &signature)).await;
        assert_eq!(
            (reply.status, reply.body),
            (StatusCode::OK, serde_json::json!({ "ok": true }))
        );
        assert_eq!(
            emails.email_of(NODE).await.as_deref(),
            Some("you@example.com")
        );
    }

    #[tokio::test]
    async fn an_uppercase_node_id_hits_the_same_replay_guard() {
        let emails = Emails::in_memory(10);
        assert_eq!(
            send(&emails, &body("", NODE, NOW)).await.status,
            StatusCode::OK
        );
        let reply = send(&emails, &body(EMAIL, &NODE.to_uppercase(), NOW)).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.audit.unwrap().1["reason"], "replay");
        assert_eq!(emails.email_of(NODE).await.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn an_extreme_timestamp_is_refused_without_overflow() {
        let emails = Emails::in_memory(10);
        for ts in [u64::MAX, i64::MAX as u64 + 1, 0] {
            let reply = send(&emails, &body(EMAIL, NODE, ts)).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{ts}");
        }
        assert_eq!(emails.email_of(NODE).await, None);
    }

    #[tokio::test]
    async fn an_oversized_payload_is_refused_before_its_signature_is_checked() {
        let emails = Emails::in_memory(10);
        let padded = |len: usize| {
            let mut payload = serde_json::json!({ "email": EMAIL, "node_id": NODE, "pad": "", "ts": NOW, "type": REGISTER_EMAIL_TYPE });
            let pad = len - payload.to_string().len();
            payload["pad"] = serde_json::Value::String("x".repeat(pad));
            let payload = payload.to_string();
            assert_eq!(payload.len(), len);
            envelope(
                &payload,
                &message_signing::sign(payload.as_bytes(), &key_of(NODE)),
            )
        };
        // Signed correctly, so a 400 shows the size, not the signature, refused it.
        let reply = send(&emails, &padded(MAX_PAYLOAD_LEN + 1)).await;
        assert_eq!(
            (reply.status, error_of(&reply)),
            (StatusCode::BAD_REQUEST, "invalid payload")
        );
        assert_eq!(
            send(&emails, &padded(MAX_PAYLOAD_LEN)).await.status,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn a_node_with_a_channel_skips_the_hourly_limit_and_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let registrations =
            std::sync::Arc::new(crate::push::registrations::Registrations::new(dir.path()));
        registrations
            .set_channel_peers([NODE.to_string()].into())
            .await
            .unwrap();
        let has_channel = |node: &str| registrations.has_channel(node);
        let stranger = format!("03{}", &NODE[2..]);
        for (emails, reason) in [
            (Emails::in_memory_limited(10, 0), "hourly_limit"),
            (Emails::in_memory_limited(0, 10), "full"),
        ] {
            let reply = register(&emails, &has_channel, &body(EMAIL, &stranger, NOW), NOW).await;
            assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE, "{reason}");
            assert_eq!(reply.audit.unwrap().1["reason"], reason);
            let reply = register(&emails, &has_channel, &body(EMAIL, NODE, NOW), NOW).await;
            assert_eq!(reply.status, StatusCode::OK, "{reason}");
            assert_eq!(emails.email_of(NODE).await.as_deref(), Some(EMAIL));
        }
    }
}
