import Foundation

/// Pure domain value object encapsulating stable vs. native channel balance calculations.
/// Free of any UI or database frameworks (Functional Core).
struct ChannelAllocation: Equatable, Sendable {
    let stableUSD: Double
    let lightningBalanceSats: UInt64
    let btcPrice: Double

    /// Satoshis backing the stable USD position.
    var stableSats: UInt64 {
        guard btcPrice > 0, stableUSD > 0 else { return 0 }
        let calculated = (stableUSD / btcPrice) * Double(Constants.satsInBTC)
        return UInt64(calculated)
    }

    /// Remaining satoshis in the channel belonging to the native Bitcoin position.
    var nativeSats: UInt64 {
        let stable = stableSats
        return lightningBalanceSats > stable ? lightningBalanceSats - stable : 0
    }

    /// Current fiat USD value of the native satoshi position.
    var nativeUSD: Double {
        guard btcPrice > 0, nativeSats > 0 else { return 0.0 }
        return (Double(nativeSats) / Double(Constants.satsInBTC)) * btcPrice
    }

    /// Total USD value combining both stable and native channel balances.
    var totalUSD: Double {
        stableUSD + nativeUSD
    }

    /// Ratio of the channel held in stable USD (0.0 to 1.0).
    var stableFraction: Double {
        let total = totalUSD
        guard total > 0 else { return 0.0 }
        return min(1.0, max(0.0, stableUSD / total))
    }
}
