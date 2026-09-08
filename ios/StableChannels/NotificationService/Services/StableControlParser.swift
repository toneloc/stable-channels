import Foundation
import LDKNode

enum StableControlResult {
    case none
    case handled
    case deferToForeground
}

enum SignedSettlementStatus: Equatable {
    case none
    case valid(settlementId: String)
    case duplicate(settlementId: String)
    case invalid(reason: String)
    /// Local channel state is unreadable, which says nothing about the peer's envelope.
    case stateUnavailable
}

enum StableControlParser {
    static func handleStableControl(
        node: LDKNode.Node,
        db: PaymentDatabase,
        priceFetcher: PriceFetcher,
        customRecords: [CustomTlvRecord],
        amountMsat: UInt64
    ) -> StableControlResult {
        for record in customRecords where record.typeNum == Constants.stableChannelTLVType {
            // Senders still attach the legacy [1] marker beside the signed record; it is not
            // control traffic, so skipping it keeps the settlement path reachable.
            if record.value == Data([1]) {
                continue
            }
            guard let message = TradeProtocol.parseSignedControl(
                data: record.value,
                expectedCounterparty: Constants.lspPubkey,
                verifySignature: { msg, signature, publicKey in
                    node.verifySignature(msg: msg, sig: signature, pkey: publicKey)
                }
            ) else {
                // Auth failures and malformed control packets are never accounting input and
                // should not loop forever in the extension.
                return .handled
            }
            guard amountMsat == TradeProtocol.resultControlAmountMsat else { return .handled }
            let price: Double?
            if case .sync(let sync) = message, sync.correlation == nil {
                let fetched = priceFetcher.fetchPrice()
                price = PriceOracle.isPlausibleBitcoinPrice(fetched) ? fetched : nil
                if price == nil { return .deferToForeground }
            } else {
                price = nil
            }
            switch db.applyTradeControl(message, trustedPrice: price) {
            case .applied, .duplicate, .invalid: return .handled
            case .retry: return .deferToForeground
            }
        }
        return .none
    }

    /// A payment is a stability settlement only when it carries a signed
    /// STABILITY_PAYMENT_V1 record: a fully valid, fresh, amount- and channel-bound
    /// settlement is accepted (once per settlement_id); anything invalid or replayed
    /// must not be treated as a settlement. `.invalid` still arrived as sats, so callers record a
    /// Lightning receipt; `.duplicate` is dropped so backing is never credited twice.
    /// `.stateUnavailable` must be left unacked so LDK redelivers once local state is readable.
    /// `.none` means no signed record was attached.
    static func signedSettlementStatus(
        node: LDKNode.Node,
        db: PaymentDatabase,
        customRecords: [CustomTlvRecord],
        amountMsat: UInt64
    ) -> SignedSettlementStatus {
        guard let record = customRecords.first(where: { $0.typeNum == Constants.signedStabilityTLVType }) else {
            return .none
        }
        // Missing local channel state is a retryable local condition, not a bad envelope.
        // Demoting it to a Lightning receipt would dedupe the payment id and make the
        // backing credit unrecoverable once the channel row comes back.
        guard let channelId = db.readChannelState()?.channelId, !channelId.isEmpty else {
            return .stateUnavailable
        }
        switch TradeProtocol.parseSignedStabilitySettlement(
            data: record.value,
            expectedDirection: TradeProtocol.stabilityDirectionLspToUser,
            expectedChannelId: channelId,
            actualAmountMsat: amountMsat,
            expectedCounterparty: Constants.lspPubkey,
            verifySignature: { msg, signature, publicKey in
                node.verifySignature(msg: msg, sig: signature, pkey: publicKey)
            }
        ) {
        case .valid(let settlement):
            if db.isSettlementSeen(settlementId: settlement.settlementId) {
                return .duplicate(settlementId: settlement.settlementId)
            }
            return .valid(settlementId: settlement.settlementId)
        case .invalid(let reason):
            return .invalid(reason: reason)
        }
    }
}
