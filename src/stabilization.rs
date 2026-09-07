//! Trade-entry guard, not a settlement invariant. Existing positions may drift above this limit.

use crate::constants::{
    ABSOLUTE_MIN_NATIVE_SATS, CLIENT_SAFETY_MARGIN_SATS, MAX_STABLE_ALLOCATION_PERCENT,
    SATS_IN_BTC, STABLE_CHANNEL_TRADE_FEE_RATE,
};

/// Spendable means the peer's capacity, excluding that peer's punishment reserve and commitment
/// fees. The LSP passes inbound capacity AFTER the fee payment; clients subtract the fee first.
pub fn backing_cap(post_fee_spendable_sats: u64) -> Option<u64> {
    let after_floor = post_fee_spendable_sats.checked_sub(ABSOLUTE_MIN_NATIVE_SATS)?;
    // Widen before multiplying so even malformed u64 inputs cannot overflow.
    let percent = (u128::from(post_fee_spendable_sats) * u128::from(MAX_STABLE_ALLOCATION_PERCENT)
        / 100) as u64;
    Some(percent.min(after_floor))
}

pub fn client_backing_limit(post_fee_spendable_sats: u64) -> Option<u64> {
    backing_cap(post_fee_spendable_sats)?
        .checked_sub(CLIENT_SAFETY_MARGIN_SATS)
        .filter(|limit| *limit > 0)
}

/// Match the correlated trade protocol's fee derivation from its signed target.
pub fn trade_fee_sats(old_expected: f64, new_expected: f64, price: f64) -> Option<u64> {
    if !old_expected.is_finite()
        || old_expected < 0.0
        || !new_expected.is_finite()
        || new_expected < 0.0
        || !price.is_finite()
        || price <= 0.0
    {
        return None;
    }
    let delta = (new_expected - old_expected).abs();
    let gross = if new_expected > old_expected {
        delta / (1.0 - STABLE_CHANNEL_TRADE_FEE_RATE)
    } else {
        delta
    };
    let sats = gross * STABLE_CHANNEL_TRADE_FEE_RATE / price * SATS_IN_BTC as f64;
    (sats.is_finite() && sats >= 0.0 && sats < (u64::MAX / 1000) as f64).then_some(sats as u64)
}

#[derive(Clone, Copy, Debug)]
pub struct SellSnapshot {
    /// Capacity plus this receiver's reserve, used by the existing drift-preserving math.
    pub receiver_sats: u64,
    /// Own outbound capacity only. Never infer this from channel_value minus the other peer.
    pub spendable_sats: u64,
    pub backing_sats: u64,
    pub expected_usd: f64,
    pub price: f64,
}

impl SellSnapshot {
    pub fn accepts(&self, order_cents: u64) -> bool {
        self.fits(order_cents, false)
    }

    fn fits(&self, order_cents: u64, searching: bool) -> bool {
        if order_cents == 0
            || !self.price.is_finite()
            || self.price <= 0.0
            || !self.expected_usd.is_finite()
            || self.expected_usd < 0.0
        {
            return false;
        }
        let amount = order_cents as f64 / 100.0;
        let required = (amount / self.price * SATS_IN_BTC as f64).ceil();
        let native = self.receiver_sats.saturating_sub(self.backing_sats);
        if !required.is_finite() || required > native as f64 {
            return false;
        }
        let target = crate::stable::normalize_trade_expected_usd(
            self.expected_usd + (amount - amount * STABLE_CHANNEL_TRADE_FEE_RATE),
        );
        if target < self.expected_usd || (!searching && target == self.expected_usd) {
            return false;
        }
        let Some(fee) = trade_fee_sats(self.expected_usd, target, self.price) else {
            return false;
        };
        let Some(receiver) = self.receiver_sats.checked_sub(fee) else {
            return false;
        };
        let Some(limit) = self
            .spendable_sats
            .checked_sub(fee)
            .and_then(client_backing_limit)
        else {
            return false;
        };
        if target > receiver as f64 / SATS_IN_BTC as f64 * self.price {
            return false;
        }
        let backing = crate::stable::trade_backing_after_delta(
            receiver,
            self.backing_sats,
            self.expected_usd,
            target,
            self.price,
        );
        // Ignore the lower (nonzero-backing) bound during the upper-bound search. Otherwise
        // a one-cent no-op can hide a valid two-cent order. Submission still rejects no-ops.
        if searching
            && self.backing_sats == 0
            && (target / self.price * SATS_IN_BTC as f64).floor()
                == (self.expected_usd / self.price * SATS_IN_BTC as f64).floor()
        {
            return true;
        }
        backing.is_some_and(|backing| backing <= limit)
    }

    pub fn max_order_cents(&self) -> u64 {
        if !self.price.is_finite()
            || self.price <= 0.0
            || !self.expected_usd.is_finite()
            || self.expected_usd < 0.0
        {
            return 0;
        }
        let native = self.receiver_sats.saturating_sub(self.backing_sats);
        let cents = (native as f64 / SATS_IN_BTC as f64 * self.price * 100.0).floor();
        if !cents.is_finite() || cents < 1.0 || cents >= i64::MAX as f64 {
            return 0;
        }
        let (mut low, mut high) = (0, cents as u64);
        while low < high {
            let distance = high - low;
            let mid = low + distance / 2 + distance % 2;
            if self.fits(mid, true) {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        // Very low prices can make the first cent normalize to zero. Do not advertise a
        // non-trade when no representable additional USD target fits.
        if low > 0 && self.accepts(low) {
            low
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_cross_platform_vectors() {
        #[derive(serde::Deserialize)]
        struct Vector {
            receiver_sats: u64,
            spendable_sats: u64,
            backing_sats: u64,
            expected_usd: f64,
            price: f64,
            max_cents: u64,
        }
        let vectors: Vec<Vector> =
            serde_json::from_str(include_str!("../tests/fixtures/stabilization-limits.json"))
                .unwrap();
        for v in vectors {
            let s = SellSnapshot {
                receiver_sats: v.receiver_sats,
                spendable_sats: v.spendable_sats,
                backing_sats: v.backing_sats,
                expected_usd: v.expected_usd,
                price: v.price,
            };
            assert_eq!(s.max_order_cents(), v.max_cents, "{s:?}");
            if v.max_cents > 0 {
                assert!(s.accepts(v.max_cents));
            }
            assert!(!s.accepts(v.max_cents + 1));
        }
    }
    #[test]
    fn strict_integer_boundaries() {
        assert_eq!(backing_cap(1_000_001), Some(990_000));
        assert_eq!(backing_cap(100_000), Some(98_000));
        assert_eq!(backing_cap(1_999), None);
        assert_eq!(client_backing_limit(2_050), None);
        assert_eq!(client_backing_limit(2_051), Some(1));
        assert!(backing_cap(u64::MAX).unwrap() < u64::MAX);
    }
    #[test]
    fn maximum_search_matches_exhaustive_small_wallets() {
        for receiver in [2_050u64, 3_000, 9_999, 100_000] {
            for price in [22.0, 100_000.0, 1_000_000.0] {
                for backing in [0, receiver / 2, receiver * 99 / 100] {
                    let s = SellSnapshot {
                        receiver_sats: receiver,
                        spendable_sats: receiver,
                        backing_sats: backing,
                        expected_usd: backing as f64 / SATS_IN_BTC as f64 * price,
                        price,
                    };
                    let high = ((receiver - backing) as f64 / SATS_IN_BTC as f64 * price * 100.0)
                        .ceil() as u64;
                    let exhaustive = (1..=high)
                        .rev()
                        .find(|cents| s.accepts(*cents))
                        .unwrap_or(0);
                    assert_eq!(s.max_order_cents(), exhaustive, "{s:?}");
                }
            }
        }
    }
    #[test]
    fn maximum_and_next_cent_share_validation() {
        for (receiver, spendable, backing, expected, price) in [
            (100_000, 100_000, 0, 0.0, 100_000.0),
            (200_000, 195_000, 50_000, 50.0, 100_000.0),
            (1_000_001, 1_000_001, 500_000, 500.0, 100_000.0),
            (200_000, 195_000, 50_000, 50.0, 80_000.0),
        ] {
            let snapshot = SellSnapshot {
                receiver_sats: receiver,
                spendable_sats: spendable,
                backing_sats: backing,
                expected_usd: expected,
                price,
            };
            let max = snapshot.max_order_cents();
            assert!(max > 0 && snapshot.accepts(max));
            assert!(!snapshot.accepts(max + 1));
        }
    }
    #[test]
    fn small_and_drifted_positions_cannot_increase() {
        for (receiver, backing) in [(2_000, 0), (100_000, 99_000)] {
            let snapshot = SellSnapshot {
                receiver_sats: receiver,
                spendable_sats: receiver,
                backing_sats: backing,
                expected_usd: backing as f64 / 1000.0,
                price: 100_000.0,
            };
            assert_eq!(snapshot.max_order_cents(), 0);
        }
    }
    #[test]
    fn peers_use_their_own_capacity_not_the_other_reserve() {
        let client_capacity = 195_000u64;
        let user_reserve = 5_000;
        let provider_reserve = 20_000;
        let commitment_fee = 3_000;
        let fee = 123;
        let receiver_display = client_capacity + user_reserve;
        let server_inbound_after_fee = client_capacity - fee;
        assert_eq!(
            backing_cap(receiver_display - user_reserve - fee),
            backing_cap(server_inbound_after_fee)
        );
        assert_ne!(
            backing_cap(receiver_display - provider_reserve - fee),
            backing_cap(server_inbound_after_fee)
        );
        assert_ne!(
            backing_cap(server_inbound_after_fee + commitment_fee),
            backing_cap(server_inbound_after_fee)
        );
    }
}
