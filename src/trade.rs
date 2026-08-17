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
    ChannelBusy,
    StaleState,
    InvalidConfirmation,
    ConfirmationExpired,
    ReservationInvalidated,
    ClientCancelled,
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
        Self::ChannelBusy,
        Self::StaleState,
        Self::InvalidConfirmation,
        Self::ConfirmationExpired,
        Self::ReservationInvalidated,
        Self::ClientCancelled,
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
            Self::ChannelBusy => "channel_busy",
            Self::StaleState => "stale_state",
            Self::InvalidConfirmation => "invalid_confirmation",
            Self::ConfirmationExpired => "confirmation_expired",
            Self::ReservationInvalidated => "reservation_invalidated",
            Self::ClientCancelled => "client_cancelled",
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
            Self::ChannelBusy => "Another order is already being reviewed for this channel.",
            Self::StaleState => {
                "The channel changed while the order was prepared. Refresh and retry."
            }
            Self::InvalidConfirmation => "The order confirmation was invalid. Refresh and retry.",
            Self::ConfirmationExpired => "The order confirmation expired. Refresh the quote.",
            Self::ReservationInvalidated => {
                "The channel changed after confirmation. Refresh and retry."
            }
            Self::ClientCancelled => "The order was cancelled.",
        }
    }
}

/// Signed LSP reservation. The wallet verifies every field before enabling the fee payment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmTradeV1 {
    #[serde(rename = "type")]
    pub kind: String,
    pub channel_id: String,
    pub user_channel_id: String,
    pub trade_id: String,
    pub proposal_payment_id: String,
    pub proposal_hash: String,
    pub confirmation_id: String,
    pub expected_usd: f64,
    pub quote_price: f64,
    pub fee_msat: u64,
    pub base_sync_version: u64,
    pub confirmed_at: u64,
    pub expires_at: u64,
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

pub fn is_confirmation_id(value: &str) -> bool {
    is_canonical_32_byte_hex(value)
}

impl ConfirmTradeV1 {
    pub fn has_valid_shape(&self) -> bool {
        self.kind == crate::constants::CONFIRM_TRADE_MESSAGE_TYPE
            && is_channel_id(&self.channel_id)
            && is_user_channel_id(&self.user_channel_id)
            && is_trade_id(&self.trade_id)
            && is_payment_id(&self.proposal_payment_id)
            && is_request_hash(&self.proposal_hash)
            && is_confirmation_id(&self.confirmation_id)
            && self.expected_usd.is_finite()
            && self.expected_usd >= 0.0
            && self.quote_price.is_finite()
            && self.quote_price > 0.0
            && self.fee_msat > 0
            && self.base_sync_version < i64::MAX as u64
            && self.confirmed_at <= self.expires_at
            && self.expires_at.saturating_sub(self.confirmed_at)
                == crate::constants::TRADE_CONFIRMATION_TTL_SECS
    }

    /// Convert the LSP's absolute expiry into a conservative wallet-local deadline.
    ///
    /// A future-dated LSP clock makes `expires_at` look farther away to the wallet. The signed
    /// proposal timestamp is on the wallet's clock, so cap the usable deadline at one confirmation
    /// TTL after that timestamp. For an aligned or slow LSP clock, the signed expiry remains the
    /// tighter bound.
    pub fn wallet_deadline(&self, proposal_timestamp: u64) -> Option<u64> {
        self.has_valid_shape().then(|| {
            self.expires_at.min(
                proposal_timestamp.saturating_add(crate::constants::TRADE_CONFIRMATION_TTL_SECS),
            )
        })
    }
}

pub fn is_channel_id(value: &str) -> bool {
    is_canonical_32_byte_hex(value)
}

/// Upgraded desktop messages carry the wallet's LDK-local identifier as canonical decimal u128.
pub fn is_user_channel_id(value: &str) -> bool {
    value
        .parse::<u128>()
        .is_ok_and(|parsed| parsed.to_string() == value)
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
        assert!(is_confirmation_id(&valid));
        assert!(!is_trade_id(&valid.to_ascii_uppercase()));
        assert!(!is_trade_id("abcd"));
        assert!(is_user_channel_id("0"));
        assert!(is_user_channel_id(&u128::MAX.to_string()));
        assert!(!is_user_channel_id("01"));
        assert!(!is_user_channel_id("not-decimal"));
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

    #[test]
    fn confirmation_shape_is_strict_and_time_bounded() {
        let id = "11".repeat(32);
        let confirmation = ConfirmTradeV1 {
            kind: crate::constants::CONFIRM_TRADE_MESSAGE_TYPE.to_owned(),
            channel_id: "22".repeat(32),
            user_channel_id: "7".to_owned(),
            trade_id: id.clone(),
            proposal_payment_id: "33".repeat(32),
            proposal_hash: "44".repeat(32),
            confirmation_id: id,
            expected_usd: 25.0,
            quote_price: 100_000.0,
            fee_msat: 2_000,
            base_sync_version: 4,
            confirmed_at: 100,
            expires_at: 160,
        };
        assert!(confirmation.has_valid_shape());
        let mut bad = confirmation.clone();
        bad.expires_at += 1;
        assert!(!bad.has_valid_shape());
        let mut value = serde_json::to_value(confirmation).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ConfirmTradeV1>(value).is_err());
    }

    #[test]
    fn confirmation_wallet_deadline_caps_future_lsp_clock() {
        let mut confirmation = ConfirmTradeV1 {
            kind: crate::constants::CONFIRM_TRADE_MESSAGE_TYPE.to_string(),
            channel_id: "11".repeat(32),
            user_channel_id: "42".to_string(),
            trade_id: "22".repeat(32),
            proposal_payment_id: "33".repeat(32),
            proposal_hash: "44".repeat(32),
            confirmation_id: "55".repeat(32),
            expected_usd: 10.0,
            quote_price: 100_000.0,
            fee_msat: 1_000,
            base_sync_version: 0,
            confirmed_at: 160,
            expires_at: 220,
        };

        assert_eq!(confirmation.wallet_deadline(100), Some(160));

        confirmation.confirmed_at = 40;
        confirmation.expires_at = 100;
        assert_eq!(confirmation.wallet_deadline(100), Some(100));
    }
}
