import Foundation
import LDKNode

struct TradeExecutionResult {
    let paymentId: String
    let newExpectedUSD: Double
    let btcAmount: Double
    let tradeDbId: Int64
}

/// Builds a durable, correlated TRADE_V1 request and sends its non-refundable fee.
final class TradeService {
    private let nodeService: NodeService
    private let databaseService: DatabaseService

    init(nodeService: NodeService, databaseService: DatabaseService) {
        self.nodeService = nodeService
        self.databaseService = databaseService
    }

    func executeBuy(
        sc: StableChannel,
        amountUSD: Double,
        feeUSD: Double,
        price: Double
    ) throws -> TradeExecutionResult? {
        guard amountUSD > 0, amountUSD <= sc.expectedUSD.amount, price > 0 else { return nil }
        let netAmount = amountUSD - feeUSD
        return try preparePersistAndSend(
            sc: sc,
            action: "buy",
            amountUSD: amountUSD,
            amountBTC: netAmount / price,
            feeUSD: feeUSD,
            newExpectedUSD: max(sc.expectedUSD.amount - amountUSD, 0),
            price: price
        )
    }

    func executeSell(
        sc: StableChannel,
        amountUSD: Double,
        feeUSD: Double,
        price: Double,
        maxUSD: Double
    ) throws -> TradeExecutionResult? {
        guard amountUSD > 0, price > 0 else { return nil }
        let netAmount = amountUSD - feeUSD
        return try preparePersistAndSend(
            sc: sc,
            action: "sell",
            amountUSD: amountUSD,
            amountBTC: netAmount / price,
            feeUSD: feeUSD,
            newExpectedUSD: min(sc.expectedUSD.amount + netAmount, maxUSD),
            price: price
        )
    }

    private func preparePersistAndSend(
        sc: StableChannel,
        action: String,
        amountUSD: Double,
        amountBTC: Double,
        feeUSD: Double,
        newExpectedUSD: Double,
        price: Double
    ) throws -> TradeExecutionResult? {
        guard let prepared = TradeProtocol.prepare(
            channelId: sc.channelId,
            userChannelId: sc.userChannelId,
            currentExpectedUSD: sc.expectedUSD.amount,
            currentBackingSats: sc.backingSats,
            receiverSats: sc.stableReceiverBTC.sats,
            action: action,
            amountUSD: amountUSD,
            amountBTC: amountBTC,
            feeUSD: feeUSD,
            newExpectedUSD: newExpectedUSD,
            quotePrice: price
        ) else { return nil }

        // Persist the exact signed payload and local allocation before the fee can leave.
        let tradeDbId = try databaseService.channelRepo.recordPreparedTrade(prepared)
        let paymentIdString: String
        do {
            let signature = try nodeService.signMessage(Array(prepared.requestPayload.utf8))
            let envelope: [String: Any] = [
                "payload": prepared.requestPayload,
                "signature": signature
            ]
            let envelopeData = try JSONSerialization.data(
                withJSONObject: envelope,
                options: [.sortedKeys, .withoutEscapingSlashes]
            )
            let paymentId = try nodeService.sendKeysendWithTLV(
                amountMsat: prepared.feeMsat,
                to: sc.counterparty,
                tlvs: [CustomTlvRecord(
                    typeNum: Constants.stableChannelTLVType,
                    value: envelopeData
                )]
            )
            paymentIdString = "\(paymentId)"
        } catch {
            _ = try? databaseService.channelRepo.markTradeSendFailed(tradeDbId: tradeDbId)
            throw error
        }

        // The payment has left the node at this point. A local bookkeeping failure must not
        // report a send failure (or invite the user to pay the non-refundable fee twice).
        let attached = (try? databaseService.channelRepo.attachTradePaymentId(
            tradeDbId: tradeDbId,
            paymentId: paymentIdString
        )) == true
        if !attached {
            AuditService.log("TRADE_PAYMENT_ID_PERSIST_FAILED", data: [
                "trade_db_id": "\(tradeDbId)",
                "trade_id": prepared.tradeId,
                "payment_id": paymentIdString
            ])
        }
        AuditService.log("TRADE_MESSAGE_SENT", data: [
            "trade_id": prepared.tradeId,
            "request_hash": prepared.requestHash,
            "payment_id": paymentIdString,
            "fee_msat": "\(prepared.feeMsat)",
            "new_expected_usd": "\(prepared.newExpectedUSD)",
            "new_backing_sats": "\(prepared.newBackingSats)"
        ])
        return TradeExecutionResult(
            paymentId: paymentIdString,
            newExpectedUSD: prepared.newExpectedUSD,
            btcAmount: amountBTC,
            tradeDbId: tradeDbId
        )
    }
}
