import Foundation
import LDKNode
import SwiftUI

extension SendFlowModel {
    /// Coordinates authentication, balance checks, payment dispatch, and settlement observation.
    func executeSend(appState: AppState) async {
        guard !isSending else { return }
        guard let dest = destination else {
            resetToken += 1
            return
        }

        let quotedInvoice: LNURLInvoiceQuote?
        switch dest {
        case .lightningAddress, .lnurlPay:
            guard resolvedLNURLService(appState: appState) != nil else {
                errorMessage = "Wallet network is not initialized. Please wait until connected."
                resetToken += 1
                return
            }
            guard let quote = payableLNURLQuote(appState: appState) else {
                resetToken += 1
                return
            }
            quotedInvoice = quote
        default:
            quotedInvoice = nil
        }

        let sendsAll = sendsEntireOnchainBalance(appState: appState)
        await performSend(appState: appState, authReason: "Confirm payment to \(dest.displayTitle)") { sats in
            let result: SendPaymentResult
            if sendsAll, case .onchain(let address, _) = dest {
                result = try await SendPaymentExecutor.sendAllOnchain(
                    address: address,
                    price: appState.accountingBTCPrice,
                    feeRateSatVb: effectiveFeeRateSatVb,
                    appState: appState
                )
            } else {
                do {
                    result = try await SendPaymentExecutor.execute(
                        destination: dest,
                        effectiveSats: sats,
                        feeRateSatVb: effectiveFeeRateSatVb,
                        lnurlQuote: quotedInvoice,
                        appState: appState
                    )
                } catch {
                    refreshLNURLQuoteAfterFailure(appState: appState)
                    throw error
                }
            }

            switch result.outcome {
            case .onchain(let txid):
                // Already broadcast into the mempool.
                showSuccess(sentSats: sendsAll ? sats : result.sentAmountSats, txid: txid, isPending: false)

            case .spliceOut:
                // The splice confirms on-chain; there is no payment to await and no txid until
                // negotiation completes. Home shows it as a pending move meanwhile.
                showSuccess(sentSats: result.sentAmountSats, isPending: true)

            case .lightning(let pid):
                let timeout: TimeInterval
                if case .bolt12 = dest {
                    timeout = 10.0
                } else {
                    timeout = 7.0
                }

                let outcome = await PaymentSettlementObserver.awaitSettlement(
                    paymentId: pid,
                    timeoutSeconds: timeout,
                    appState: appState
                )
                applySettlement(outcome, sentSats: result.sentAmountSats, paymentId: pid, appState: appState)
            }
        }
    }

    // MARK: - LNURL invoice quote

    /// False while an LNURL send has no invoice to show, so the slide stays disabled.
    var isLNURLQuoteReady: Bool {
        switch destination {
        case .lightningAddress, .lnurlPay: lnurlQuote != nil
        default: true
        }
    }

    /// Fetches the invoice for the confirm step unless a quote is shown or a fetch is in flight.
    func loadLNURLQuoteIfNeeded(appState: AppState) {
        guard step == .confirm, lnurlQuote == nil, lnurlQuoteTask == nil else { return }
        switch destination {
        case .lightningAddress, .lnurlPay: startLNURLQuoteFetch(appState: appState)
        default: return
        }
    }

    /// Replaces any quote with a fresh fetch for the current amount and comment. A replaced or
    /// cancelled fetch checks cancellation after its await, so its late result never lands.
    func startLNURLQuoteFetch(appState: AppState) {
        invalidateLNURLQuote()
        guard let service = resolvedLNURLService(appState: appState) else {
            lnurlQuoteError = "Wallet network is not initialized. Please wait until connected."
            return
        }
        guard let params = lnurlParams else {
            lnurlQuoteError = "Missing LNURL parameters"
            return
        }
        let amountMsat = sendAmountSats(appState: appState) * 1000
        let comment = LNURLInvoiceQuote.normalizedComment(lnurlComment)
        isFetchingLNURLQuote = true
        lnurlQuoteTask = Task { [weak self] in
            let result: Result<LNURLInvoiceQuote, Error>
            do {
                result = try await .success(service.fetchInvoice(
                    params: params,
                    amountMsat: amountMsat,
                    comment: comment
                ))
            } catch {
                result = .failure(error)
            }
            guard let self, !Task.isCancelled else { return }
            switch result {
            case .success(let quote):
                lnurlQuote = quote
            case .failure(let error):
                lnurlQuoteError = WalletErrorMessages.operation(error, fallback: error.localizedDescription)
            }
            isFetchingLNURLQuote = false
            lnurlQuoteTask = nil
        }
    }

    /// A failed LNURL payment leaves the user on confirm; a retry must pay a fresh invoice, not the failed one.
    func refreshLNURLQuoteAfterFailure(appState: AppState) {
        switch destination {
        case .lightningAddress, .lnurlPay: startLNURLQuoteFetch(appState: appState)
        default: return
        }
    }

    /// Drops the quote and abandons any in-flight fetch.
    func invalidateLNURLQuote() {
        lnurlQuoteTask?.cancel()
        lnurlQuoteTask = nil
        lnurlQuote = nil
        lnurlQuoteError = nil
        isFetchingLNURLQuote = false
    }

    /// The quote the slide may pay. An expired quote is
    /// replaced by a fresh fetch the user must review and slide again for: never auto-paid.
    private func payableLNURLQuote(appState: AppState) -> LNURLInvoiceQuote? {
        guard let quote = lnurlQuote else { return nil }
        guard !quote.isExpired(at: Date()) else {
            startLNURLQuoteFetch(appState: appState)
            errorMessage = String(
                localized: "error_lnurl_invoice_refreshed",
                defaultValue: "The invoice expired. Review the new invoice and slide again."
            )
            return nil
        }
        return quote
    }

    /// The single pre-dispatch gate for every send screen. The send is marked in flight before the
    /// first await, so a second slide during authentication cannot dispatch twice. Runs the preflight
    /// rules, transaction authentication, and the LSP reconnect, then hands the amount to `dispatch`.
    /// Failures land in `errorMessage`; the slider resets once the attempt ends.
    func performSend(
        appState: AppState,
        authReason: String,
        dispatch: (_ amountSats: UInt64) async throws -> Void
    ) async {
        guard !isSending else { return }
        isSending = true
        errorMessage = nil
        defer {
            isSending = false
            resetToken += 1
        }

        let sats = sendAmountSats(appState: appState)
        let decision = SendPreflight.evaluate(SendPreflight.Input(
            isOnline: appState.isOnline,
            isFeeRateReady: isFeeRateReady,
            amountSats: sats,
            totalDebitSats: totalDebitSats(appState: appState),
            availableSats: availableSpendableSats(appState: appState),
            sendsAllFromOnchainWallet: sendsAllFromOnchainWallet
        ))
        if let message = Self.message(for: decision) {
            errorMessage = message
            return
        }

        if UserDefaults.standard.bool(forKey: Constants.transactionAuthEnabledKey) {
            guard await appState.authenticate(reason: authReason) else {
                errorMessage = appState.authError ?? Self.authenticationRequiredMessage
                return
            }
        }

        appState.ensureLSPConnected()
        do {
            try await dispatch(sats)
        } catch {
            errorMessage = WalletErrorMessages.operation(error, fallback: error.localizedDescription)
        }
    }

    private static func message(for decision: SendPreflight.Decision) -> String? {
        switch decision {
        case .proceed:
            nil
        case .offline:
            String(
                localized: "error_offline_send",
                defaultValue: "You’re offline. Payments cannot be sent until network connectivity is restored."
            )
        case .feeRateNotReady:
            feeRateNotReadyMessage
        case .invalidAmount:
            "Invalid amount."
        case .exceedsBalance(let available):
            "Amount exceeds your balance. Available: \(available.btcSpacedFormatted) BTC"
        }
    }

    /// Maps a Lightning settlement outcome to the result screen. A timeout is not a settlement:
    /// it must stay pending so an unconfirmed payment never renders as "Payment Sent".
    func applySettlement(
        _ outcome: PaymentSettlementOutcome,
        sentSats: UInt64,
        paymentId: String,
        appState: AppState
    ) {
        switch outcome {
        case .settled:
            showSuccess(sentSats: sentSats, paymentId: paymentId, isPending: false)
        case .failed(let reason):
            errorMessage = reason
            refreshLNURLQuoteAfterFailure(appState: appState)
        case .timedOut:
            showSuccess(sentSats: sentSats, paymentId: paymentId, isPending: true)
        }
    }

    /// A receipt shown as pending follows the payment to its outcome: settled completes it, failed
    /// returns to confirm with the reason, and a timeout keeps it pending.
    func applyLateSettlement(_ outcome: PaymentSettlementOutcome, appState: AppState) {
        guard isPendingSettlement, step == .success else { return }
        switch outcome {
        case .settled:
            isPendingSettlement = false
        case .failed(let reason):
            isPendingSettlement = false
            errorMessage = reason
            step = .confirm
            refreshLNURLQuoteAfterFailure(appState: appState)
        case .timedOut:
            break
        }
    }

    /// Keeps observing a pending Lightning receipt until it resolves or the screen goes away.
    func observePendingSettlement(appState: AppState) async {
        while !Task.isCancelled, isPendingSettlement, step == .success, let paymentId = successPaymentId {
            let outcome = await PaymentSettlementObserver.awaitSettlement(
                paymentId: paymentId,
                timeoutSeconds: 60,
                appState: appState
            )
            applyLateSettlement(outcome, appState: appState)
        }
    }

    func showSuccess(sentSats: UInt64, paymentId: String? = nil, txid: String? = nil, isPending: Bool) {
        sentAmountSats = sentSats
        successPaymentId = paymentId
        successTxid = txid
        isPendingSettlement = isPending
        step = .success
    }
}
