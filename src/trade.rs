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

/// Whether the target an LSP answered with honours the target the wallet signed.
///
/// Equal is the ordinary case. Lower means the LSP booked what its own valuation of the wallet's
/// channel side can actually back, rather than funding the shortfall out of its own liquidity;
/// accept that as far as the two sides could honestly disagree about the value of those sats.
/// Higher is not a clamp and is never accepted.
///
/// Both the sync handler and the database's own trade-completion gate have to agree on this, so the
/// rule lives here rather than being spelled out twice — a stricter copy in either place strands the
/// trade it was meant to confirm.
pub fn answered_target_honours_request(requested_usd: f64, answered_usd: f64) -> bool {
    if (requested_usd - answered_usd).abs() <= 0.000000001 {
        return true;
    }
    let clamp_floor =
        requested_usd * (1.0 - crate::constants::MAX_PEER_VALUATION_SPREAD_PERCENT / 100.0);
    answered_usd < requested_usd && answered_usd >= clamp_floor
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
    fn answered_target_accepts_a_capacity_clamp_but_nothing_above_the_request() {
        let spread = crate::constants::MAX_PEER_VALUATION_SPREAD_PERCENT / 100.0;
        assert!(answered_target_honours_request(100.0, 100.0));
        assert!(answered_target_honours_request(100.0, 100.0 * (1.0 - spread / 2.0)));
        assert!(answered_target_honours_request(100.0, 100.0 * (1.0 - spread)));
        assert!(!answered_target_honours_request(100.0, 100.0 * (1.0 - spread) - 0.01));
        assert!(!answered_target_honours_request(100.0, 100.01));
        // Closing a peg answers zero for zero, which is equality rather than a clamp.
        assert!(answered_target_honours_request(0.0, 0.0));
        assert!(!answered_target_honours_request(0.0, 1.0));
    }
}
