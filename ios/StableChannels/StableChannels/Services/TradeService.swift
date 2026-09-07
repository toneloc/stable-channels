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

    private func liveSnapshot(sc: StableChannel, price: Double) -> StabilizationSnapshot? {
        guard let channel = nodeService.node?.listChannels()
            .first(where: { $0.userChannelId == sc.userChannelId && $0.isChannelReady }) else { return nil }
        let capacity = channel.outboundCapacityMsat / 1000
        return StabilizationSnapshot(receiverSats: capacity + (channel.unspendablePunishmentReserve ?? 0),
                                     spendableSats: capacity, backingSats: sc.backingSats,
                                     expectedUSD: sc.expectedUSD.amount, price: price)
    }

    func maxSellCents(sc: StableChannel, price: Double) -> UInt64 {
        liveSnapshot(sc: sc, price: price)?.maxOrderCents() ?? 0
    }

    func executeBuy(
        sc: StableChannel,
        amountUSD: Double,
        feeUSD: Double,
        price: Double
    ) throws -> TradeExecutionResult? {
        guard amountUSD.isFinite, amountUSD > 0, amountUSD <= sc.expectedUSD.amount, price.isFinite,
              price > 0 else { throw TradeValidationError.invalidAmount }
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
        price: Double
    ) throws -> TradeExecutionResult? {
        guard amountUSD.isFinite, amountUSD > 0, price.isFinite,
              price > 0 else { throw TradeValidationError.invalidAmount }
        let netAmount = amountUSD - feeUSD
        return try preparePersistAndSend(
            sc: sc,
            action: "sell",
            amountUSD: amountUSD,
            amountBTC: netAmount / price,
            feeUSD: feeUSD,
            newExpectedUSD: sc.expectedUSD.amount + netAmount,
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
        guard let snapshot = liveSnapshot(sc: sc, price: price) else { throw TradeValidationError.unavailable }
        if newExpectedUSD > sc.expectedUSD.amount {
            guard amountUSD * 100 < Double(Int64.max),
                  snapshot.accepts(UInt64((amountUSD * 100 + 1e-7).rounded(.down))) else {
                throw TradeValidationError.stabilizationLimit(snapshot.maxOrderCents())
            }
        }
        guard let prepared = TradeProtocol.prepare(
            channelId: sc.channelId,
            userChannelId: sc.userChannelId,
            currentExpectedUSD: sc.expectedUSD.amount,
            currentBackingSats: sc.backingSats,
            receiverSats: snapshot.receiverSats,
            spendableSats: snapshot.spendableSats,
            action: action,
            amountUSD: amountUSD,
            amountBTC: amountBTC,
            feeUSD: feeUSD,
            newExpectedUSD: newExpectedUSD,
            quotePrice: price
        ) else { throw TradeValidationError.unsafeAllocation }

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
