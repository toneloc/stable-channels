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
    /// Wallet-generated correlation id. New wallets use exactly 32 random bytes encoded as
    /// lowercase hex; the field remains optional for legacy mobile clients.
    #[serde(default)]
    pub trade_id: Option<String>,
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

/// Stable machine-readable rejection reasons carried by `TRADE_REJECTED_V1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeRejectionReason {
    InvalidAmount,
    StaleRequest,
    InvalidFee,
    InvalidQuote,
    QuoteOutOfRange,
    InvalidAllocation,
    InsufficientCapacity,
    DuplicateTrade,
    InternalError,
}

impl TradeRejectionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidAmount => "invalid_amount",
            Self::StaleRequest => "stale_request",
            Self::InvalidFee => "invalid_fee",
            Self::InvalidQuote => "invalid_quote",
            Self::QuoteOutOfRange => "quote_out_of_range",
            Self::InvalidAllocation => "invalid_allocation",
            Self::InsufficientCapacity => "insufficient_capacity",
            Self::DuplicateTrade => "duplicate_trade",
            Self::InternalError => "internal_error",
        }
    }
}

/// LSP-signed rejection returned as a nominal control keysend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeRejectedPayload {
    #[serde(rename = "type")]
    pub kind: String,
    pub channel_id: String,
    pub user_channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trade_id: Option<String>,
    pub trade_payment_id: String,
    pub reason_code: TradeRejectionReason,
    pub explanation: String,
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

/// Build a correlated trade acceptance. Correlation fields are omitted for ordinary/legacy sync
/// messages so old mobile decoders continue to see the exact shape they already accept.
pub fn build_trade_sync_payload(
    channel_id: &str,
    user_channel_id: &str,
    expected_usd: f64,
    backing_sats: u64,
    sync_version: u64,
    trade_id: Option<&str>,
    trade_payment_id: &str,
) -> String {
    let mut value = serde_json::json!({
        "type": stable_channels::constants::SYNC_MESSAGE_TYPE,
        "channel_id": channel_id,
        "user_channel_id": user_channel_id,
        "expected_usd": expected_usd,
        "backing_sats": backing_sats,
        "sync_version": sync_version,
        "trade_payment_id": trade_payment_id,
    });
    if let Some(trade_id) = trade_id {
        value["trade_id"] = serde_json::Value::String(trade_id.to_owned());
    }
    value.to_string()
}

pub fn build_trade_rejected_payload(
    channel_id: &str,
    user_channel_id: &str,
    trade_id: Option<&str>,
    trade_payment_id: &str,
    reason_code: TradeRejectionReason,
    explanation: &str,
    ts: u64,
) -> String {
    serde_json::to_string(&TradeRejectedPayload {
        kind: "TRADE_REJECTED_V1".to_owned(),
        channel_id: channel_id.to_owned(),
        user_channel_id: user_channel_id.to_owned(),
        trade_id: trade_id.map(str::to_owned),
        trade_payment_id: trade_payment_id.to_owned(),
        reason_code,
        explanation: explanation.to_owned(),
        ts,
    })
    .unwrap_or_default()
}

/// A v1 trade id is deliberately strict so alternative spellings cannot bypass deduplication.
pub fn is_valid_trade_id(trade_id: &str) -> bool {
    trade_id.len() == 64
        && trade_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

    #[test]
    fn correlated_sync_fields_are_optional_and_exact() {
        let payload = build_trade_sync_payload(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "7",
            25.0,
            31_250,
            4,
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            "payment",
        );
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["trade_payment_id"], "payment");
        assert_eq!(
            value["trade_id"],
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn rejection_payload_round_trips_all_stable_reason_codes() {
        let reasons = [
            TradeRejectionReason::InvalidAmount,
            TradeRejectionReason::StaleRequest,
            TradeRejectionReason::InvalidFee,
            TradeRejectionReason::InvalidQuote,
            TradeRejectionReason::QuoteOutOfRange,
            TradeRejectionReason::InvalidAllocation,
            TradeRejectionReason::InsufficientCapacity,
            TradeRejectionReason::DuplicateTrade,
            TradeRejectionReason::InternalError,
        ];
        for reason in reasons {
            let payload = build_trade_rejected_payload(
                "channel", "7", None, "payment", reason, "rejected", 123,
            );
            let parsed: TradeRejectedPayload = serde_json::from_str(&payload).unwrap();
            assert_eq!(parsed.reason_code, reason);
            assert_eq!(parsed.trade_payment_id, "payment");
        }
    }

    #[test]
    fn trade_id_requires_canonical_lowercase_hex() {
        assert!(is_valid_trade_id(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!is_valid_trade_id(
            "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF"
        ));
        assert!(!is_valid_trade_id("abcd"));
    }
}
