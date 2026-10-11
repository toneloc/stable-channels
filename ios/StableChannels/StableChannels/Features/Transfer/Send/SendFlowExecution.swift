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
            errorMessage = "Waiting for network fee rate. Please wait a moment."
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
                errorMessage = appState.authError ?? "Authentication required to send."
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
        let sats = computeEffectiveSats(btcPrice: appState.accountingBTCPrice)
        guard sats > 0 else {
            errorMessage = "Invalid amount."
            return
        }
        let available = availableSpendableSats(appState: appState)
        let totalDebit = sats + estimatedFeeSats(appState: appState)
        guard totalDebit <= available, available > 0 else {
            errorMessage = "Amount exceeds your balance. Available: \(available.btcSpacedFormatted) BTC"
            return
        }

        do {
            let service = resolvedLNURLService(appState: appState)
            let result = try await SendPaymentExecutor.execute(
                destination: dest,
                effectiveSats: sats,
                feeRateSatVb: effectiveFeeRateSatVb,
                lnurlParams: lnurlParams,
                lnurlComment: lnurlComment,
                appState: appState,
                lnurlService: service
            )

            switch result.outcome {
            case .onchain(let txid):
                // Already broadcast into the mempool.
                showSuccess(sentSats: result.sentAmountSats, txid: txid, isPending: false)

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
                switch outcome {
                case .settled:
                    showSuccess(sentSats: result.sentAmountSats, paymentId: pid, isPending: false)
                case .failed(let reason):
                    errorMessage = reason
                case .timedOut:
                    showSuccess(sentSats: result.sentAmountSats, paymentId: pid, isPending: true)
                }
            }
        } catch {
            errorMessage = WalletErrorMessages.operation(error, fallback: error.localizedDescription)
        }
    }

    private func showSuccess(sentSats: UInt64, paymentId: String? = nil, txid: String? = nil, isPending: Bool) {
        sentAmountSats = sentSats
        successPaymentId = paymentId
        successTxid = txid
        isPendingSettlement = isPending
        step = .success
    }
}
