//! Wire vocabulary shared by both ends of a stable-channel trade.

use serde::{Deserialize, Serialize};

/// Declare the rejection reasons once, and derive the enum, the wire spelling, and the full set from
/// that single list.
///
/// The wallet used to validate incoming reasons against a hand-typed copy of the daemon's enum with
/// nothing linking the two, so a reason added on the server was silently discarded by the client and
/// stranded the trade row it was meant to resolve. Adding a reason here reaches both sides or fails
/// to compile.
macro_rules! trade_rejection_reasons {
    ($($variant:ident => $code:literal),+ $(,)?) => {
        /// Stable machine-readable rejection reasons carried by `TRADE_REJECTED_V1`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum TradeRejectionReason {
            $($variant),+
        }

        impl TradeRejectionReason {
            /// Every reason a peer may legitimately send.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $code),+
                }
            }
        }
    };
}

trade_rejection_reasons! {
    InvalidAmount => "invalid_amount",
    StaleRequest => "stale_request",
    InvalidFee => "invalid_fee",
    InvalidQuote => "invalid_quote",
    QuoteOutOfRange => "quote_out_of_range",
    InvalidAllocation => "invalid_allocation",
    NoOpTrade => "no_op_trade",
    InsufficientCapacity => "insufficient_capacity",
    DuplicateTrade => "duplicate_trade",
    InternalError => "internal_error",
}

impl TradeRejectionReason {
    /// Parse a reason off the wire. An unknown code is not this protocol's.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|reason| reason.as_str() == code)
    }
}

/// Whether the LSP answered with the exact target the wallet signed.
///
/// Backing is derived per-peer, but the USD target is the trade contract. Accepting a lower target
/// would create an implicit partial fill and can even reverse a trade when the channel is already
/// under-backed. Keep this shared so the sync handler and the transactional DB gate cannot disagree.
pub fn answered_target_matches_request(requested_usd: f64, answered_usd: f64) -> bool {
    requested_usd.is_finite()
        && answered_usd.is_finite()
        && (requested_usd - answered_usd).abs() <= 0.000000001
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_round_trips_through_its_wire_code() {
        for reason in TradeRejectionReason::ALL {
            assert_eq!(TradeRejectionReason::from_code(reason.as_str()), Some(*reason));
            // The serde spelling is what actually crosses the wire, so it must agree with as_str.
            let serialized = serde_json::to_string(reason).unwrap();
            assert_eq!(serialized, format!("\"{}\"", reason.as_str()));
        }
    }

    #[test]
    fn unknown_reason_codes_are_rejected() {
        assert_eq!(TradeRejectionReason::from_code("not_a_reason"), None);
        assert_eq!(TradeRejectionReason::from_code(""), None);
    }

    #[test]
    fn answered_target_requires_the_signed_target_exactly() {
        assert!(answered_target_matches_request(100.0, 100.0));
        assert!(!answered_target_matches_request(100.0, 99.99));
        assert!(!answered_target_matches_request(100.0, 100.01));
        assert!(answered_target_matches_request(0.0, 0.0));
        assert!(!answered_target_matches_request(0.0, 1.0));
        assert!(!answered_target_matches_request(f64::NAN, 100.0));
    }
}
