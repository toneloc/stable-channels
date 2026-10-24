import Foundation

/// Bitcoin amounts that arrive from outside the wallet, such as the mempool WebSocket feed.
/// They are checked before any arithmetic, so a hostile value is rejected instead of
/// overflowing and crashing the app.
enum OnchainAmount {
    /// 21 million BTC in sats: no real output, or sum of outputs, can be larger.
    static let maxSats: Int64 = 2_100_000_000_000_000

    /// The sum of `values`, or nil when any value is negative or the total exceeds ``maxSats``.
    static func total(of values: some Sequence<Int64>) -> Int64? {
        var total: Int64 = 0
        for value in values {
            guard value >= 0 else { return nil }
            let (sum, overflow) = total.addingReportingOverflow(value)
            guard !overflow, sum <= maxSats else { return nil }
            total = sum
        }
        return total
    }

    /// `sats` in millisatoshis, or nil when `sats` is negative or above ``maxSats``.
    static func msats(fromSats sats: Int64) -> UInt64? {
        guard (0...maxSats).contains(sats) else { return nil }
        return UInt64(sats) * 1000
    }
}
