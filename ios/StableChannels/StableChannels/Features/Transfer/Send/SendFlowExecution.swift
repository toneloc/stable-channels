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
        guard !isSending else { return }
        errorMessage = nil

        guard appState.isOnline else {
            errorMessage = String(
                localized: "error_offline_send",
                defaultValue: "You’re offline. Payments cannot be sent until network connectivity is restored."
            )
            resetToken += 1
            return
        }

        // For onchain sends, block if fee rate has not loaded (non-standard tier)
        if case .onchain = dest, !isFeeRateReady {
            errorMessage = Self.feeRateNotReadyMessage
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

        let authReason = "Confirm payment to \(dest.displayTitle)"
        let authEnabled = UserDefaults.standard.bool(forKey: "transactionAuthEnabled")
        if authEnabled {
            let passed = await appState.authenticate(reason: authReason)
            guard passed else {
                errorMessage = appState.authError ?? Self.authenticationRequiredMessage
                resetToken += 1
                return
            }
        }

        isSending = true
        defer {
            isSending = false
            resetToken += 1
        }

        appState.ensureLSPConnected()
        let sats = sendAmountSats(appState: appState)
        let sendsAll = sendsEntireOnchainBalance(appState: appState)
        guard sats > 0 else {
            errorMessage = "Invalid amount."
            return
        }
        let available = availableSpendableSats(appState: appState)
        let totalDebit = PaymentFeeEstimator.saturatingAdd(sats, estimatedFeeSats(appState: appState))
        guard totalDebit <= available, available > 0 else {
            errorMessage = "Amount exceeds your balance. Available: \(available.btcSpacedFormatted) BTC"
            return
        }

        do {
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
        } catch {
            errorMessage = WalletErrorMessages.operation(error, fallback: error.localizedDescription)
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
