import Foundation
import LDKNode

/// Protocol for payment database operations
protocol PaymentDatabase {
    func recordPayment(
        paymentId: String,
        paymentType: String,
        direction: String,
        amountMsat: UInt64,
        amountUSD: Double,
        btcPrice: Double,
        backingDeltaSats: Int64?,
        userChannelId: String?,
        settlementId: String?
    ) -> PaymentInsertResult

    func paymentExists(paymentId: String) -> Bool
    func readChannelState() -> ChannelState?
    func activeUserChannelId() -> String?
    func applyTradeControl(
        _ message: TradeControlMessage,
        trustedPrice: Double?
    ) -> StableControlDatabaseResult
    func setPendingSendPaymentId(paymentId: String) -> Bool
    func claimPendingSend(amountMsat: UInt64, price: Double) -> Bool
    func loadPendingSend() -> PendingOutgoingStabilityPayment?
    func clearPendingSend()
    func reconcilePendingOutgoingPayment(node: LDKNode.Node) -> Bool
    func isSettlementSeen(settlementId: String) -> Bool
    /// Commits a splice event (and any txid it lets an operation claim) before it is acknowledged.
    func recordSpliceEvent(_ receipt: SpliceEventReceipt) -> Bool
    /// nil when the database cannot be read.
    func hasPendingSplice() -> Bool?
    /// Commit an outbound payment's success; false means do not ack the LDK event.
    func recordOutgoingPaymentSuccess(paymentId: String, feeMsat: UInt64?) -> Bool
    /// Commit an outbound payment's failure; false means do not ack the LDK event.
    func recordOutgoingPaymentFailure(
        paymentId: String,
        reasonCode: String?,
        lookupAmountMsat: () -> UInt64?
    ) -> Bool
}

enum StableControlDatabaseResult {
    case applied
    case duplicate
    case invalid
    case retry
}

/// Result of recording a payment
enum PaymentInsertResult {
    case inserted
    case duplicate
    case failed
    case missingChannelRow
}

/// Pending outgoing stability payment marker
struct PendingOutgoingStabilityPayment {
    let paymentId: String
    let amountMsat: UInt64
    let btcPrice: Double
    let createdAt: Int64
}

/// Channel state read from DB
struct ChannelState {
    let expectedUSD: Double
    let backingSats: UInt64
    let nativeSats: UInt64
    let receiverSats: UInt64
    let latestPrice: Double
    let userChannelId: String
    let channelId: String
}
