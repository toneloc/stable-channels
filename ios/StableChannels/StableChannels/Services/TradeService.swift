import Foundation
import LDKNode

/// Handles buy/sell BTC trades and custom TLV message protocol.
class TradeService {
    private let nodeService: NodeService

    init(nodeService: NodeService) {
        self.nodeService = nodeService
    }

    // MARK: - Trade Execution

    /// Execute a buy BTC trade. Returns the PaymentId on success so the caller
    /// can store a PendingTradePayment. Trade is NOT applied until payment confirms.
    func executeBuy(
        sc: StableChannel,
        amountUSD: Double,
        feeUSD: Double,
        price: Double
    ) throws -> (paymentId: String, newExpectedUSD: Double, btcAmount: Double)? {
        guard amountUSD > 0, amountUSD <= sc.expectedUSD.amount, price > 0 else { return nil }

        let netAmount = amountUSD - feeUSD
        let newExpectedUSD = max(sc.expectedUSD.amount - amountUSD, 0)
        let btcAmount = netAmount / price

        // Send trade message to counterparty — payment must succeed before we apply the trade
        let paymentId = try sendTradeMessage(
            expectedUSD: newExpectedUSD,
            feeUSD: feeUSD,
            price: price,
            channelId: sc.channelId,
            userChannelId: sc.userChannelId,
            counterparty: sc.counterparty
        )

        return (paymentId: paymentId, newExpectedUSD: newExpectedUSD, btcAmount: btcAmount)
    }

    /// Execute a sell BTC trade. Returns the PaymentId on success.
    func executeSell(
        sc: StableChannel,
        amountUSD: Double,
        feeUSD: Double,
        price: Double,
        maxUSD: Double
    ) throws -> (paymentId: String, newExpectedUSD: Double, btcAmount: Double)? {
        guard amountUSD > 0, price > 0 else { return nil }

        let netAmount = amountUSD - feeUSD
        let newExpectedUSD = min(sc.expectedUSD.amount + netAmount, maxUSD)
        let btcAmount = netAmount / price

        let paymentId = try sendTradeMessage(
            expectedUSD: newExpectedUSD,
            feeUSD: feeUSD,
            price: price,
            channelId: sc.channelId,
            userChannelId: sc.userChannelId,
            counterparty: sc.counterparty
        )

        return (paymentId: paymentId, newExpectedUSD: newExpectedUSD, btcAmount: btcAmount)
    }

    // MARK: - Custom TLV Messages

    /// Send a TRADE_V1 message to the counterparty. Returns the PaymentId string.
    /// The fee is sent as the keysend payment amount (matching desktop behavior).
    @discardableResult
    func sendTradeMessage(
        expectedUSD: Double,
        feeUSD: Double = 0,
        price: Double = 0,
        channelId: String,
        userChannelId: String,
        counterparty: String
    ) throws -> String {
        let payload: [String: Any] = [
            "type": Constants.tradeMessageType,
            "channel_id": channelId,
            "user_channel_id": "\(userChannelId)",
            "expected_usd": expectedUSD,
        ]

        guard let payloadData = try? JSONSerialization.data(withJSONObject: payload),
              let payloadStr = String(data: payloadData, encoding: .utf8) else {
            throw TradeError.encodingFailed
        }

        let signature = try nodeService.signMessage(Array(payloadStr.utf8))

        let envelope: [String: Any] = [
            "payload": payloadStr,
            "signature": signature,
        ]

        guard let envelopeData = try? JSONSerialization.data(withJSONObject: envelope),
              let envelopeStr = String(data: envelopeData, encoding: .utf8) else {
            throw TradeError.encodingFailed
        }

        let tlv = CustomTlvRecord(
            typeNum: Constants.stableChannelTLVType,
            value: Array(envelopeStr.utf8)
        )

        // Send fee as payment amount (matches desktop: fee_msats.max(1))
        let feeMsat: UInt64
        if price > 0 && feeUSD > 0 {
            let feeBtc = feeUSD / price
            let feeSats = UInt64(feeBtc * 100_000_000)
            feeMsat = max(feeSats * 1000, 1)
        } else {
            feeMsat = 1
        }

        let paymentId = try nodeService.sendKeysendWithTLV(
            amountMsat: feeMsat,
            to: counterparty,
            tlvs: [tlv]
        )
        return "\(paymentId)"
    }

    /// Parse and verify an incoming TLV message (TRADE_V1 or SYNC_V1).
    static func parseIncomingTLV(
        data: [UInt8],
        expectedCounterparty: String,
        verifySignature: (([UInt8], String, String) -> Bool)
    ) -> (type: String, expectedUSD: Double, userChannelId: String)? {
        guard let envelopeStr = String(bytes: data, encoding: .utf8),
              let envelopeData = envelopeStr.data(using: .utf8),
              let envelope = try? JSONSerialization.jsonObject(with: envelopeData) as? [String: Any],
              let payloadStr = envelope["payload"] as? String,
              let signature = envelope["signature"] as? String else {
            return nil
        }

        // Verify signature
        guard verifySignature(Array(payloadStr.utf8), signature, expectedCounterparty) else {
            return nil
        }

        // Parse payload
        guard let payloadData = payloadStr.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: payloadData) as? [String: Any],
              let msgType = payload["type"] as? String,
              let expectedUSD = payload["expected_usd"] as? Double else {
            return nil
        }

        let userChannelId = payload["user_channel_id"] as? String ?? ""

        return (type: msgType, expectedUSD: expectedUSD, userChannelId: userChannelId)
    }
}

enum TradeError: LocalizedError {
    case encodingFailed

    var errorDescription: String? {
        switch self {
        case .encodingFailed: return "Failed to encode trade message"
        }
    }
}
