//! Wire vocabulary shared by the desktop wallet and LSP trade protocol.

use bitcoin::hashes::{sha256, Hash};
use serde::{Deserialize, Serialize};

/// Authenticated trade failures. These codes are deliberately closed: the wallet never renders
/// peer-provided prose and ignores responses containing a code it does not understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeRejectionReason {
    InvalidAmount,
    StaleRequest,
    InvalidFee,
    InvalidQuote,
    QuoteDeviation,
    InsufficientCapacity,
    SettlementRequired,
    UnsafeAllocation,
    InternalFailure,
}

impl TradeRejectionReason {
    pub const ALL: &'static [Self] = &[
        Self::InvalidAmount,
        Self::StaleRequest,
        Self::InvalidFee,
        Self::InvalidQuote,
        Self::QuoteDeviation,
        Self::InsufficientCapacity,
        Self::SettlementRequired,
        Self::UnsafeAllocation,
        Self::InternalFailure,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidAmount => "invalid_amount",
            Self::StaleRequest => "stale_request",
            Self::InvalidFee => "invalid_fee",
            Self::InvalidQuote => "invalid_quote",
            Self::QuoteDeviation => "quote_deviation",
            Self::InsufficientCapacity => "insufficient_capacity",
            Self::SettlementRequired => "settlement_required",
            Self::UnsafeAllocation => "unsafe_allocation",
            Self::InternalFailure => "internal_failure",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|reason| reason.as_str() == code)
    }

    /// Fixed local copy for the desktop UI. No arbitrary text from the peer is displayed.
    pub const fn user_message(self) -> &'static str {
        match self {
            Self::InvalidAmount => "The trade amount is invalid. Review the amount and retry.",
            Self::StaleRequest => {
                "The quote expired before it could be accepted. Refresh and retry."
            }
            Self::InvalidFee => "The trade fee was invalid. Refresh the quote before retrying.",
            Self::InvalidQuote => "A valid market quote is required. Refresh and retry.",
            Self::QuoteDeviation => "The market moved outside the quote range. Refresh and retry.",
            Self::InsufficientCapacity => {
                "The channel does not have enough capacity for this trade. Reduce the amount."
            }
            Self::SettlementRequired => {
                "Settle the current stability adjustment before retrying this trade."
            }
            Self::UnsafeAllocation => {
                "This trade cannot preserve the current channel allocation safely."
            }
            Self::InternalFailure => "The provider could not process the trade. Try again later.",
        }
    }
}

/// Signed LSP rejection carried inside the existing stable-channel TLV.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradeRejectedV1 {
    #[serde(rename = "type")]
    pub kind: String,
    pub channel_id: String,
    pub trade_id: String,
    pub trade_payment_id: String,
    pub request_hash: String,
    pub reason_code: TradeRejectionReason,
    pub decided_at: u64,
}

/// Canonical identifiers used by correlated trade messages are 32 bytes of lowercase hex.
pub fn is_canonical_32_byte_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn is_trade_id(value: &str) -> bool {
    is_canonical_32_byte_hex(value)
}

pub fn is_payment_id(value: &str) -> bool {
    is_canonical_32_byte_hex(value)
}

pub fn is_request_hash(value: &str) -> bool {
    is_canonical_32_byte_hex(value)
}

pub fn is_channel_id(value: &str) -> bool {
    is_canonical_32_byte_hex(value)
}

/// Hash the exact signed TRADE_V1 payload bytes, without parsing or reserialization.
pub fn request_hash(payload: &[u8]) -> String {
    sha256::Hash::hash(payload).to_string()
}

/// The USD target is the contract; correlated responses never permit partial fills.
pub fn target_matches(requested_usd: f64, answered_usd: f64) -> bool {
    requested_usd.is_finite()
        && answered_usd.is_finite()
        && (requested_usd - answered_usd).abs() <= 0.000000001
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_require_exact_lowercase_hex() {
        let valid = "0123456789abcdef".repeat(4);
        assert!(is_trade_id(&valid));
        assert!(is_payment_id(&valid));
        assert!(is_request_hash(&valid));
        assert!(!is_trade_id(&valid.to_ascii_uppercase()));
        assert!(!is_trade_id("abcd"));
    }

    #[test]
    fn request_hash_uses_exact_payload_bytes() {
        let compact = br#"{"type":"TRADE_V1","expected_usd":1}"#;
        let spaced = br#"{"type": "TRADE_V1", "expected_usd": 1}"#;
        assert_eq!(
            request_hash(compact),
            "53842fe0ffc7d71ad4b6f9d0940152f9803eba24ba30de840b8967c410acbcd4"
        );
        assert_ne!(request_hash(compact), request_hash(spaced));
    }

    #[test]
    fn rejection_reasons_round_trip_with_wire_codes() {
        for reason in TradeRejectionReason::ALL {
            assert_eq!(
                TradeRejectionReason::from_code(reason.as_str()),
                Some(*reason)
            );
            assert_eq!(
                serde_json::to_string(reason).unwrap(),
                format!("\"{}\"", reason.as_str())
            );
            assert!(!reason.user_message().is_empty());
        }
        assert_eq!(TradeRejectionReason::from_code("peer_text"), None);
    }

    #[test]
    fn target_matching_is_exact_with_float_noise_tolerance() {
        assert!(target_matches(10.0, 10.0));
        assert!(!target_matches(10.0, 9.99));
        assert!(!target_matches(f64::NAN, 10.0));
    }
}
