//! TRADE_V1 / SYNC_V1 / RegisterPush signed-message codec over custom TLV 13377331.

use serde::{Deserialize, Serialize};

/// Max bytes of a custom-TLV value we will attempt to parse (DoS guard).
pub const MAX_TLV_VALUE_BYTES: usize = 8 * 1024;

/// Outer signed envelope: a JSON-string payload plus a zbase32 signature over its bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedEnvelope {
    pub payload: String,
    pub signature: String,
}

/// Inbound TRADE_V1 payload (wallet to LSP). `expected_usd` is required (no default).
#[derive(Debug, Clone, Deserialize)]
pub struct TradePayload {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub user_channel_id: Option<String>,
    /// Desktop correlation id. Omitted by legacy Android/iOS clients.
    #[serde(default)]
    pub trade_id: Option<String>,
    pub expected_usd: f64,
    /// Wallet BTC/USD quote. The LSP uses it only for slippage and fee validation.
    #[serde(default)]
    pub quote_price: Option<f64>,
    /// Unix seconds the wallet signed at; 0 if absent (un-upgraded wallet). Drives replay freshness.
    #[serde(default)]
    pub ts: u64,
}

/// RegisterPush signed body. Field declaration order IS the canonical serialization order;
/// the wallet must serialize an identical struct so the daemon can reconstruct the signed bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterPushSigned {
    #[serde(rename = "type")]
    pub kind: String,
    pub node_id: String,
    pub token: String,
    pub ts: u64,
}

/// Build the SYNC_V1 payload string (the exact bytes the daemon signs and ships).
pub fn build_sync_payload(
    channel_id: &str,
    user_channel_id: &str,
    expected_usd: f64,
    backing_sats: u64,
    sync_version: u64,
) -> String {
    serde_json::json!({
        "type": stable_channels::constants::SYNC_MESSAGE_TYPE,
        "channel_id": channel_id,
        "user_channel_id": user_channel_id,
        "expected_usd": expected_usd,
        "backing_sats": backing_sats,
        "sync_version": sync_version,
    })
    .to_string()
}

/// Build the correlated acceptance returned for a desktop trade. Ordinary and legacy syncs use
/// `build_sync_payload`, which intentionally omits all correlation fields.
#[allow(clippy::too_many_arguments)]
pub fn build_trade_sync_payload(
    channel_id: &str,
    user_channel_id: &str,
    expected_usd: f64,
    backing_sats: u64,
    sync_version: u64,
    trade_id: &str,
    trade_payment_id: &str,
    request_hash: &str,
) -> String {
    serde_json::json!({
        "type": stable_channels::constants::SYNC_MESSAGE_TYPE,
        "channel_id": channel_id,
        "user_channel_id": user_channel_id,
        "expected_usd": expected_usd,
        "backing_sats": backing_sats,
        "sync_version": sync_version,
        "trade_id": trade_id,
        "trade_payment_id": trade_payment_id,
        "request_hash": request_hash,
    })
    .to_string()
}

pub fn build_trade_rejected_payload(
    channel_id: &str,
    trade_id: &str,
    trade_payment_id: &str,
    request_hash: &str,
    reason_code: stable_channels::trade::TradeRejectionReason,
    decided_at: u64,
) -> String {
    serde_json::to_string(&stable_channels::trade::TradeRejectedV1 {
        kind: stable_channels::constants::TRADE_REJECTED_MESSAGE_TYPE.to_owned(),
        channel_id: channel_id.to_owned(),
        trade_id: trade_id.to_owned(),
        trade_payment_id: trade_payment_id.to_owned(),
        request_hash: request_hash.to_owned(),
        reason_code,
        decided_at,
    })
    .unwrap_or_default()
}

/// Wrap a signed payload string + signature into the envelope JSON string.
pub fn build_envelope(payload: String, signature: String) -> String {
    serde_json::to_string(&SignedEnvelope { payload, signature }).unwrap_or_default()
}

/// Parse the outer envelope from raw (already UTF-8) TLV bytes.
pub fn parse_envelope(raw: &str) -> Option<SignedEnvelope> {
    serde_json::from_str::<SignedEnvelope>(raw).ok()
}

/// Return true when a signed envelope declares an inner `TRADE_V1` message.
///
/// This only classifies the payment carrier. The trade handler still parses the complete
/// payload and verifies its signature and channel binding before applying anything.
pub fn is_trade_v1(envelope: &SignedEnvelope) -> bool {
    serde_json::from_str::<serde_json::Value>(&envelope.payload)
        .ok()
        .and_then(|payload| {
            payload
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(|kind| kind == stable_channels::constants::TRADE_MESSAGE_TYPE)
        })
        .unwrap_or(false)
}

/// Parse the inner TRADE payload from the envelope's payload string.
pub fn parse_trade_payload(payload: &str) -> Option<TradePayload> {
    serde_json::from_str::<TradePayload>(payload).ok()
}

/// Canonical RegisterPush signed bytes. Must match the wallet's serialization exactly.
pub fn register_push_signed_bytes(node_id: &str, token: &str, ts: u64) -> Vec<u8> {
    serde_json::to_vec(&RegisterPushSigned {
        kind: "REGISTER_PUSH_V1".to_string(),
        node_id: node_id.to_string(),
        token: token.to_string(),
        ts,
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_payload_has_expected_shape() {
        let channel_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let payload = build_sync_payload(channel_id, "7", 25.0, 31_250, 4);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["type"], "SYNC_V1");
        assert_eq!(v["channel_id"], channel_id);
        assert_eq!(v["user_channel_id"], "7");
        assert_eq!(v["expected_usd"], 25.0);
        assert_eq!(v["backing_sats"], 31_250);
        assert_eq!(v["sync_version"], 4);
    }

    #[test]
    fn envelope_round_trips() {
        let env = build_envelope("the-payload".to_string(), "the-sig".to_string());
        let parsed = parse_envelope(&env).unwrap();
        assert_eq!(parsed.payload, "the-payload");
        assert_eq!(parsed.signature, "the-sig");
    }

    #[test]
    fn trade_v1_is_classified_by_inner_message_type() {
        let trade = SignedEnvelope {
            payload: r#"{"type":"TRADE_V1","expected_usd":12.5}"#.to_string(),
            signature: "sig".to_string(),
        };
        let sync = SignedEnvelope {
            payload: r#"{"type":"SYNC_V1","expected_usd":12.5}"#.to_string(),
            signature: "sig".to_string(),
        };

        assert!(is_trade_v1(&trade));
        assert!(!is_trade_v1(&sync));
    }

    #[test]
    fn trade_payload_parses_wallet_shape() {
        let payload = r#"{"type":"TRADE_V1","channel_id":"abcd","user_channel_id":"189476124653200987495269098788434301048","expected_usd":12.5}"#;
        let t = parse_trade_payload(payload).unwrap();
        assert_eq!(t.kind, "TRADE_V1");
        assert_eq!(t.channel_id.as_deref(), Some("abcd"));
        assert_eq!(
            t.user_channel_id.as_deref(),
            Some("189476124653200987495269098788434301048")
        );
        assert_eq!(t.expected_usd, 12.5);
        assert_eq!(t.trade_id, None);
        assert_eq!(t.quote_price, None);
    }

    #[test]
    fn trade_payload_ignores_legacy_backing_allocation() {
        let payload = r#"{"type":"TRADE_V1","user_channel_id":"7","expected_usd":25.0,"quote_price":80000.0,"backing_sats":31250,"ts":123}"#;
        let t = parse_trade_payload(payload).unwrap();
        assert_eq!(t.quote_price, Some(80_000.0));
        assert_eq!(t.ts, 123);
    }

    #[test]
    fn bad_json_is_none() {
        assert!(parse_envelope("not json").is_none());
        assert!(parse_trade_payload("not json").is_none());
    }

    #[test]
    fn trade_missing_expected_usd_is_none() {
        let payload = r#"{"type":"TRADE_V1","channel_id":"abcd"}"#;
        assert!(parse_trade_payload(payload).is_none());
    }

    #[test]
    fn register_push_bytes_are_canonical_and_deterministic() {
        let a = register_push_signed_bytes("nodehex", "tok:en", 1717000000);
        let expected =
            r#"{"type":"REGISTER_PUSH_V1","node_id":"nodehex","token":"tok:en","ts":1717000000}"#;
        assert_eq!(String::from_utf8(a.clone()).unwrap(), expected);
        let b = register_push_signed_bytes("nodehex", "tok:en", 1717000000);
        assert_eq!(a, b);
    }

    #[test]
    fn correlated_acceptance_contains_all_correlation_fields() {
        let id = "0123456789abcdef".repeat(4);
        let hash = "fedcba9876543210".repeat(4);
        let payload =
            build_trade_sync_payload(&"a".repeat(64), "7", 25.0, 31_250, 4, &id, &id, &hash);
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["trade_id"], id);
        assert_eq!(value["trade_payment_id"], id);
        assert_eq!(value["request_hash"], hash);
    }

    #[test]
    fn rejection_has_only_protocol_fields() {
        let id = "0123456789abcdef".repeat(4);
        let hash = "fedcba9876543210".repeat(4);
        let payload = build_trade_rejected_payload(
            &"a".repeat(64),
            &id,
            &id,
            &hash,
            stable_channels::trade::TradeRejectionReason::InsufficientCapacity,
            1786310000,
        );
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["type"], "TRADE_REJECTED_V1");
        assert_eq!(value["reason_code"], "insufficient_capacity");
        assert_eq!(value["decided_at"], 1786310000_u64);
        assert_eq!(value.as_object().unwrap().len(), 7);
    }
}
