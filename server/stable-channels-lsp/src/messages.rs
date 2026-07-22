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
    pub expected_usd: f64,
    /// BTC/USD quote used by the wallet to derive the signed sat allocation.
    #[serde(default)]
    pub quote_price: Option<f64>,
    /// Exact stable backing allocation after the trade-fee payment settles.
    #[serde(default)]
    pub backing_sats: Option<u64>,
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
    user_channel_id: u128,
    expected_usd: f64,
    backing_sats: u64,
    sync_version: u64,
) -> String {
    serde_json::json!({
        "type": stable_channels::constants::SYNC_MESSAGE_TYPE,
        "user_channel_id": format!("{}", user_channel_id),
        "expected_usd": expected_usd,
        "backing_sats": backing_sats,
        "sync_version": sync_version,
    })
    .to_string()
}

/// Wrap a signed payload string + signature into the envelope JSON string.
pub fn build_envelope(payload: String, signature: String) -> String {
    serde_json::to_string(&SignedEnvelope { payload, signature }).unwrap_or_default()
}

/// Parse the outer envelope from raw (already UTF-8) TLV bytes.
pub fn parse_envelope(raw: &str) -> Option<SignedEnvelope> {
    serde_json::from_str::<SignedEnvelope>(raw).ok()
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
        let payload = build_sync_payload(7u128, 25.0, 31_250, 4);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["type"], "SYNC_V1");
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
        assert_eq!(t.quote_price, None);
        assert_eq!(t.backing_sats, None);
    }

    #[test]
    fn trade_payload_parses_signed_allocation() {
        let payload = r#"{"type":"TRADE_V1","user_channel_id":"7","expected_usd":25.0,"quote_price":80000.0,"backing_sats":31250,"ts":123}"#;
        let t = parse_trade_payload(payload).unwrap();
        assert_eq!(t.quote_price, Some(80_000.0));
        assert_eq!(t.backing_sats, Some(31_250));
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
        let expected = r#"{"type":"REGISTER_PUSH_V1","node_id":"nodehex","token":"tok:en","ts":1717000000}"#;
        assert_eq!(String::from_utf8(a.clone()).unwrap(), expected);
        let b = register_push_signed_bytes("nodehex", "tok:en", 1717000000);
        assert_eq!(a, b);
    }
}
