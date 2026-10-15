import Foundation
import LDKNode
import SwiftUI

extension SendFlowModel {
    /// Coordinates authentication, balance checks, payment dispatch, and settlement observation.
    func executeSend(appState: AppState) async {
        guard let dest = destination else {
            resetToken += 1
            return
        }

        switch dest {
        case .lightningAddress, .lnurlPay:
            guard resolvedLNURLService(appState: appState) != nil else {
                errorMessage = "Wallet network is not initialized. Please wait until connected."
                resetToken += 1
                return
            }
        default:
            break
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
                result = try await SendPaymentExecutor.execute(
                    destination: dest,
                    effectiveSats: sats,
                    feeRateSatVb: effectiveFeeRateSatVb,
                    lnurlParams: lnurlParams,
                    lnurlComment: lnurlComment,
                    appState: appState,
                    lnurlService: resolvedLNURLService(appState: appState)
                )
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
                applySettlement(outcome, sentSats: result.sentAmountSats, paymentId: pid)
            }
        }
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
    func applySettlement(_ outcome: PaymentSettlementOutcome, sentSats: UInt64, paymentId: String) {
        switch outcome {
        case .settled:
            showSuccess(sentSats: sentSats, paymentId: paymentId, isPending: false)
        case .failed(let reason):
            errorMessage = reason
        case .timedOut:
            showSuccess(sentSats: sentSats, paymentId: paymentId, isPending: true)
        }
    }

    /// A receipt shown as pending follows the payment to its outcome: settled completes it, failed
    /// returns to confirm with the reason, and a timeout keeps it pending.
    func applyLateSettlement(_ outcome: PaymentSettlementOutcome) {
        guard isPendingSettlement, step == .success else { return }
        switch outcome {
        case .settled:
            isPendingSettlement = false
        case .failed(let reason):
            isPendingSettlement = false
            errorMessage = reason
            step = .confirm
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
            applyLateSettlement(outcome)
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
