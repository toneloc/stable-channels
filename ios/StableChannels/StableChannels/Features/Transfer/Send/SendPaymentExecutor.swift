import Foundation
import LDKNode

/// How a dispatched payment will settle. A caller must handle every case: a splice-out has
/// neither a payment id to await nor a txid until negotiation completes, so it is its own case
/// rather than a pair of nils.
enum SendPaymentOutcome: Equatable, Sendable {
    /// Lightning payment in flight; await settlement by payment id.
    case lightning(paymentId: String)
    /// On-chain transaction already broadcast.
    case onchain(txid: String)
    /// Splice-out initiated from the channel; it confirms on-chain and Home tracks it as pending.
    case spliceOut
}

/// Output result from a successfully dispatched payment.
struct SendPaymentResult: Equatable, Sendable {
    let sentAmountSats: UInt64
    let outcome: SendPaymentOutcome

    var paymentId: String? {
        if case .lightning(let paymentId) = outcome { return paymentId }
        return nil
    }

    var txid: String? {
        if case .onchain(let txid) = outcome { return txid }
        return nil
    }
}

/// Orchestrates payment dispatch across Lightning, LNURL, and Onchain subsystems.
@MainActor
struct SendPaymentExecutor {
    static func execute(
        destination: SendDestination,
        effectiveSats: UInt64,
        feeRateSatVb: Double? = nil,
        lnurlQuote: LNURLInvoiceQuote?,
        appState: AppState
    ) async throws -> SendPaymentResult {
        let price = appState.accountingBTCPrice

        switch destination {
        case .bolt11(let invoice, _, let msat):
            return try await sendBolt11(
                invoice: invoice,
                msat: msat,
                effectiveSats: effectiveSats,
                price: price,
                appState: appState
            )
        case .bolt12(let offer, _):
            return try await sendBolt12(offer: offer, effectiveSats: effectiveSats, price: price, appState: appState)
        case .lightningAddress, .lnurlPay:
            return try sendLNURL(quote: lnurlQuote, price: price, appState: appState)
        case .onchain(let address, _):
            return try await sendOnchain(
                address: address,
                effectiveSats: effectiveSats,
                price: price,
                feeRateSatVb: feeRateSatVb,
                appState: appState
            )
        }
    }

    private static func sendBolt11(
        invoice: Bolt11Invoice,
        msat: UInt64?,
        effectiveSats: UInt64,
        price: Double,
        appState: AppState
    ) async throws -> SendPaymentResult {
        guard !invoice.isExpired() else {
            throw NSError(
                domain: "Send",
                code: 4,
                userInfo: [NSLocalizedDescriptionKey: InvoicePayability.expired.userMessage ?? ""]
            )
        }
        let actualMsat: UInt64
        let paymentId: PaymentId
        if let msat, msat > 0 {
            actualMsat = msat
            try appState.ensureNoUnsettledSurplus(amountMsat: actualMsat)
            paymentId = try appState.nodeService.sendPayment(invoice: invoice)
        } else {
            actualMsat = effectiveSats * 1000
            guard actualMsat > 0 else { throw NSError(
                domain: "Send",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Invalid amount"]
            ) }
            try appState.ensureNoUnsettledSurplus(amountMsat: actualMsat)
            paymentId = try appState.nodeService.sendPaymentUsingAmount(invoice: invoice, amountMsat: actualMsat)
        }
        recordPayment(id: "\(paymentId)", type: "lightning", msat: actualMsat, price: price, appState: appState)
        return SendPaymentResult(sentAmountSats: actualMsat / 1000, outcome: .lightning(paymentId: "\(paymentId)"))
    }

    private static func sendBolt12(offer: Offer, effectiveSats: UInt64, price: Double,
                                   appState: AppState) async throws -> SendPaymentResult {
        let msat = effectiveSats * 1000
        guard msat > 0 else { throw NSError(
            domain: "Send",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: "Invalid amount"]
        ) }
        try appState.ensureNoUnsettledSurplus(amountMsat: msat)
        let paymentId = try appState.nodeService.sendBolt12UsingAmount(offer: offer, amountMsat: msat)
        recordPayment(id: "\(paymentId)", type: "bolt12", msat: msat, price: price, appState: appState)
        return SendPaymentResult(sentAmountSats: effectiveSats, outcome: .lightning(paymentId: "\(paymentId)"))
    }

    /// Pays exactly the invoice the confirm step showed; it was validated by LNURLService when quoted.
    private static func sendLNURL(
        quote: LNURLInvoiceQuote?,
        price: Double,
        appState: AppState
    ) throws -> SendPaymentResult {
        guard let quote else {
            throw NSError(domain: "LNURL", code: 1, userInfo: [NSLocalizedDescriptionKey: "Missing LNURL invoice"])
        }
        let msat = quote.amountMsat
        let bolt11 = try Bolt11Invoice.fromStr(invoiceStr: quote.invoice)
        try appState.ensureNoUnsettledSurplus(amountMsat: msat)
        let paymentId = try appState.nodeService.sendPaymentUsingAmount(invoice: bolt11, amountMsat: msat)
        if quote.isDescriptionUnbound {
            AuditService.log("LNURL_UNBOUND_INVOICE_PAID", data: [
                "binding": "\(quote.binding)",
                "payment_id": "\(paymentId)"
            ])
        }
        recordPayment(id: "\(paymentId)", type: "lnurl", msat: msat, price: price, appState: appState)
        return SendPaymentResult(sentAmountSats: msat / 1000, outcome: .lightning(paymentId: "\(paymentId)"))
    }

    static func sendOnchain(
        address: String,
        effectiveSats: UInt64,
        price: Double,
        feeRateSatVb: Double?,
        appState: AppState
    ) async throws -> SendPaymentResult {
        guard effectiveSats > 0 else {
            throw NSError(
                domain: "Send",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Invalid amount"]
            )
        }
        // A sweep has committed the whole on-chain balance to a splice-in; any on-chain send now
        // (splice-out or plain) would spend the same UTXOs. Checked before the branch because
        // isSpliceOut is false while sweeping, which would route the send around this guard.
        guard !appState.isSweeping else {
            throw NSError(
                domain: "Send",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: "A splice is already in progress"]
            )
        }
        if SendChannelSpendPolicy
            .isSpliceOut(channels: appState.nodeService.channels, isSweeping: appState.isSweeping) {
            let rate = feeRateSatVb ?? Constants.placeholderFeeRateSatVb
            guard let channel = SendChannelSpendPolicy.selectSpliceChannel(
                channels: appState.nodeService.channels,
                amountSats: effectiveSats,
                feeRateSatVb: rate
            ) else {
                throw NSError(
                    domain: "Send",
                    code: 3,
                    userInfo: [
                        NSLocalizedDescriptionKey: "No active channel has sufficient capacity to fund this on-chain splice."
                    ]
                )
            }
            try appState.beginSpliceOut(
                amountSats: effectiveSats,
                address: address,
                channel: SpliceChannelIdentity(
                    userChannelId: channel.userChannelId,
                    channelId: channel.channelId,
                    previousFundingTxid: channel.fundingTxo?.txid
                )
            )
            do {
                try appState.nodeService.spliceOut(
                    userChannelId: channel.userChannelId,
                    counterpartyNodeId: channel.counterpartyNodeId,
                    address: address,
                    amountSats: effectiveSats
                )
            } catch {
                appState.cancelPendingSpliceStart()
                throw error
            }
            return SendPaymentResult(sentAmountSats: effectiveSats, outcome: .spliceOut)
        } else {
            let txid = try appState.nodeService.sendOnchain(
                address: address,
                amountSats: effectiveSats,
                feeRateSatVb: feeRateSatVb
            )
            appState.onchainSendBroadcasted(amountSats: effectiveSats, isSendAll: false, txid: txid)
            recordPayment(
                id: txid,
                type: "onchain",
                msat: effectiveSats * 1000,
                price: price,
                address: address,
                txid: txid,
                appState: appState
            )
            return SendPaymentResult(sentAmountSats: effectiveSats, outcome: .onchain(txid: txid))
        }
    }

    static func sendAllOnchain(
        address: String,
        price: Double,
        feeRateSatVb: Double?,
        appState: AppState
    ) async throws -> SendPaymentResult {
        guard !appState.isSweeping else {
            throw NSError(
                domain: "Send",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: "A splice is already in progress"]
            )
        }
        let onchainSats = appState.spendableOnchainSats
        let txid = try appState.nodeService.sendAllOnchain(
            address: address,
            feeRateSatVb: feeRateSatVb
        )
        recordPayment(
            id: txid,
            type: "onchain",
            msat: onchainSats * 1000,
            price: price,
            address: address,
            txid: txid,
            appState: appState
        )
        appState.onchainSendBroadcasted(amountSats: onchainSats, isSendAll: true, txid: txid)
        return SendPaymentResult(sentAmountSats: onchainSats, outcome: .onchain(txid: txid))
    }

    private static func recordPayment(
        id: String,
        type: String,
        msat: UInt64,
        price: Double,
        address: String? = nil,
        txid: String? = nil,
        appState: AppState
    ) {
        // Home and History list pending rows only when this epoch moves.
        defer { appState.notifyPaymentsChanged() }
        try? appState.databaseService?.paymentRepo.recordOutgoingPending(
            paymentId: id,
            paymentType: type,
            amountMsat: msat,
            amountUSD: price > 0 ? BitcoinUnits.usd(msats: msat, price: price) : nil,
            btcPrice: price > 0 ? price : nil,
            address: address,
            txid: txid
        )
    }
}
