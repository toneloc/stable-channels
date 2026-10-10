import CryptoKit
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
        lnurlParams: LNURLPayParams?,
        lnurlComment: String,
        appState: AppState,
        lnurlService: LNURLServiceProtocol?
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
            // Fail closed: without a known wallet network there is no validator for the invoice.
            guard let lnurlService else {
                throw NSError(
                    domain: "Send",
                    code: 3,
                    userInfo: [
                        NSLocalizedDescriptionKey: "Wallet network is not initialized. Please wait until connected."
                    ]
                )
            }
            return try await sendLNURL(
                params: lnurlParams,
                comment: lnurlComment,
                effectiveSats: effectiveSats,
                price: price,
                appState: appState,
                lnurlService: lnurlService
            )
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

    private static func sendLNURL(
        params: LNURLPayParams?,
        comment: String,
        effectiveSats: UInt64,
        price: Double,
        appState: AppState,
        lnurlService: LNURLServiceProtocol
    ) async throws -> SendPaymentResult {
        guard let params else {
            throw NSError(domain: "LNURL", code: 1, userInfo: [NSLocalizedDescriptionKey: "Missing LNURL parameters"])
        }
        let msat = effectiveSats * 1000
        let trimmedComment = comment.trimmingCharacters(in: .whitespacesAndNewlines)
        let resp = try await lnurlService.fetchInvoice(
            params: params,
            amountMsat: msat,
            comment: trimmedComment.isEmpty ? nil : trimmedComment
        )
        let bolt11 = try Bolt11Invoice.fromStr(invoiceStr: resp.pr)
        guard let invoiceMsat = bolt11.amountMilliSatoshis() else {
            throw LNURLError.errorResponse(reason: "Amountless invoices are not permitted for LNURL pay.")
        }
        if invoiceMsat != msat {
            throw LNURLError.invoiceAmountMismatch(expectedMsat: msat, actualMsat: invoiceMsat)
        }

        // LUD-06 Security: Verify invoice description hash equals SHA256(metadata)
        let metadataDigest = SHA256.hash(data: Data(params.metadata.utf8))
        let expectedHashHex = metadataDigest.map { String(format: "%02x", $0) }.joined()
        switch bolt11.invoiceDescription() {
        case .hash(let hash):
            guard hash.lowercased() == expectedHashHex.lowercased() else {
                throw LNURLError.errorResponse(reason: "Invoice description hash does not match payee metadata.")
            }
        case .direct:
            // LUD-06 requires h tag (description_hash). Invoices using a direct
            // description field instead of a hash are non-compliant.
            throw LNURLError
                .errorResponse(reason: "Invoice uses direct description instead of required description hash (h tag).")
        }

        // Verify invoice has not expired
        guard !bolt11.isExpired() else {
            throw LNURLError.errorResponse(reason: "The invoice returned by the LNURL service has expired.")
        }

        // Verify invoice network matches active node network.
        // Fail closed: if the node network is unknown, reject to avoid cross-network payment.
        guard let activeNetwork = appState.nodeService.activeNetwork else {
            throw LNURLError.errorResponse(reason: "Cannot verify invoice network: node network is unavailable.")
        }
        guard bolt11.network() == activeNetwork else {
            throw LNURLError.errorResponse(reason: "Invoice network does not match the node network.")
        }

        try appState.ensureNoUnsettledSurplus(amountMsat: msat)
        let paymentId = try appState.nodeService.sendPaymentUsingAmount(invoice: bolt11, amountMsat: msat)
        recordPayment(id: "\(paymentId)", type: "lnurl", msat: msat, price: price, appState: appState)
        return SendPaymentResult(sentAmountSats: effectiveSats, outcome: .lightning(paymentId: "\(paymentId)"))
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
        if SendChannelSpendPolicy
            .isSpliceOut(channels: appState.nodeService.channels, isSweeping: appState.isSweeping) {
            guard !appState.isSweeping else {
                throw NSError(
                    domain: "Send",
                    code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "A splice is already in progress"]
                )
            }
            let rate = feeRateSatVb ?? 10.0
            let estimatedFee = PaymentFeeEstimator.estimateSpliceOutFee(feeRateSatVb: rate)
            let requiredTotal = effectiveSats + estimatedFee
            guard let channel = SendChannelSpendPolicy.selectSpliceChannel(
                channels: appState.nodeService.channels,
                requiredSats: requiredTotal
            ) else {
                throw NSError(
                    domain: "Send",
                    code: 3,
                    userInfo: [
                        NSLocalizedDescriptionKey: "No active channel has sufficient capacity to fund this on-chain splice."
                    ]
                )
            }
            try appState.beginSpliceOut(amountSats: effectiveSats, address: address)
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
        // Guard against downgrading: if the event handler already marked
        // this payment completed or failed, backfill details and do not insert "pending".
        if let existing = appState.databaseService?.paymentRepo.payment(paymentId: id) {
            let status = existing.status
            if status == "completed" || status == "succeeded" || status == "failed" {
                let usd: Double? = price > 0 ? (Double(msat) / 1000.0 / Double(Constants.satsInBTC)) * price : nil
                try? appState.databaseService?.paymentRepo.backfillPaymentDetails(
                    paymentId: id,
                    amountMsat: msat,
                    amountUSD: usd,
                    btcPrice: price > 0 ? price : nil,
                    counterparty: nil,
                    address: address,
                    txid: txid
                )
                return
            }
        }

        let usd: Double? = price > 0 ? (Double(msat) / 1000.0 / Double(Constants.satsInBTC)) * price : nil
        _ = try? appState.databaseService?.paymentRepo.recordPayment(
            paymentId: id,
            paymentType: type,
            direction: "sent",
            amountMsat: msat,
            amountUSD: usd,
            btcPrice: price > 0 ? price : nil,
            counterparty: nil,
            status: "pending",
            txid: txid,
            address: address
        )
    }

    typealias SettlementOutcome = PaymentSettlementOutcome

    static func awaitPaymentSettlement(
        paymentId: String,
        timeoutSeconds: TimeInterval,
        appState: AppState? = nil
    ) async -> SettlementOutcome {
        await PaymentSettlementObserver.awaitSettlement(
            paymentId: paymentId,
            timeoutSeconds: timeoutSeconds,
            appState: appState
        )
    }
}
