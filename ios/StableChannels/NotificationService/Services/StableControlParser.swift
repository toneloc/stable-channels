import Foundation
import LDKNode

enum StableControlResult {
    case none
    case handled
    case deferToForeground
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

    static func isStabilityPayment(_ customRecords: [CustomTlvRecord]) -> Bool {
        customRecords.contains { $0.typeNum == Constants.stableChannelTLVType && $0.value == Data([1]) }
    }
}
