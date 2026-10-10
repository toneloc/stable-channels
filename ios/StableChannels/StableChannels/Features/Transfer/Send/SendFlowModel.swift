import Foundation
import LDKNode
import SwiftUI

/// State and business orchestration for the Send workflow.
@Observable
@MainActor
final class SendFlowModel {
    var step: SendFlowStep = .recipient
    var inputText: String = "" {
        didSet { onInputChanged() }
    }

    var destination: SendDestination?
    var classification: PaymentDestinationClassification = .empty
    var amountUnit: SendAmountUnit = .usd
    var amountInputText: String = ""

    var lnurlParams: LNURLPayParams?
    var lnurlComment: String = ""
    var isFetchingLNURL: Bool = false
    var isSending: Bool = false
    var errorMessage: String?
    var successPaymentId: String?
    var successTxid: String?
    var sentAmountSats: UInt64 = 0
    var isPendingSettlement: Bool = false
    var feeRateSatVb: Double?
    var recommendedFees: RecommendedFees?
    var selectedFeeTier: NetworkFeeSpeedTier = .standard
    var resetToken: Int = 0

    /// Returns the effective fee rate using live recommended fees when available,
    /// or nil when the rate has not loaded and standard tier is selected.
    var effectiveFeeRateSatVb: Double? {
        if let rec = recommendedFees {
            return selectedFeeTier.effectiveRate(baseRate: rec.halfHourFee, recommendedFees: rec)
        }
        guard let base = feeRateSatVb else {
            return nil
        }
        return selectedFeeTier.effectiveRate(baseRate: base)
    }

    /// True when the fee rate has loaded or the selected tier does not need one.
    var isFeeRateReady: Bool {
        if case .onchain = destination {
            return recommendedFees != nil || feeRateSatVb != nil || selectedFeeTier == .standard
        }
        return true
    }

    let expectedNetwork: Network?
    let customLNURLService: LNURLServiceProtocol?

    var lnurlService: LNURLServiceProtocol? {
        customLNURLService ?? expectedNetwork.map { LNURLService(expectedNetwork: $0) }
    }

    init(expectedNetwork: Network? = nil, lnurlService: LNURLServiceProtocol? = nil) {
        self.expectedNetwork = expectedNetwork
        self.customLNURLService = lnurlService
    }

    convenience init(appState: AppState, lnurlService: LNURLServiceProtocol? = nil) {
        self.init(expectedNetwork: appState.nodeService.activeNetwork, lnurlService: lnurlService)
    }

    convenience init(nodeService: NodeService, lnurlService: LNURLServiceProtocol? = nil) {
        self.init(expectedNetwork: nodeService.activeNetwork, lnurlService: lnurlService)
    }

    func resolvedLNURLService(appState: AppState) -> LNURLServiceProtocol? {
        if let customLNURLService {
            return customLNURLService
        }
        guard let net = expectedNetwork ?? appState.nodeService.activeNetwork else {
            return nil
        }
        return LNURLService(expectedNetwork: net)
    }

    func onInputChanged() {
        errorMessage = nil
        classification = PaymentDestinationClassifier.classify(inputText)
        switch classification {
        case .valid(let target):
            if destination != target {
                destination = target
                lnurlParams = nil
                lnurlComment = ""
                if case .onchain(_, let amountSats) = target, let amountSats, amountSats > 0 {
                    amountUnit = .sats
                    amountInputText = "\(amountSats)"
                } else if case .bolt11(_, _, let msat) = target, let msat, msat > 0 {
                    amountUnit = .sats
                    amountInputText = "\(msat / 1000)"
                } else {
                    amountInputText = ""
                }
            }
        case .invalid, .empty:
            destination = nil
            lnurlParams = nil
            lnurlComment = ""
            amountInputText = ""
        }
    }

    func proceedFromRecipient(appState: AppState) async {
        guard let dest = destination else { return }
        errorMessage = nil

        switch dest {
        case .lightningAddress(_, _, let url), .lnurlPay(let url):
            guard let service = resolvedLNURLService(appState: appState) else {
                errorMessage = "Wallet network is not initialized. Please wait until connected."
                return
            }
            isFetchingLNURL = true
            defer { isFetchingLNURL = false }
            do {
                let params = try await service.fetchPayParams(from: url)
                self.lnurlParams = params
                self.step = .amount
            } catch {
                errorMessage = WalletErrorMessages.operation(error, fallback: error.localizedDescription)
            }
        case .bolt11(_, _, let msat):
            if let msat, msat > 0 {
                let requiredSats = msat / 1000
                let available = availableSpendableSats(appState: appState)
                if requiredSats > available || available == 0 {
                    errorMessage = "Amount exceeds your balance for this invoice. Available: \(available.btcSpacedFormatted) BTC"
                    return
                }
                self.step = .confirm
            } else {
                self.step = .amount
            }
        case .bolt12:
            self.step = .amount
        case .onchain(_, let amountSats):
            if let amountSats, amountSats > 0, amountInputText.isEmpty {
                amountUnit = .sats
                amountInputText = "\(amountSats)"
            }
            self.step = .amount
        }
    }

    func availableSpendableSats(appState: AppState) -> UInt64 {
        guard let dest = destination else { return appState.totalBalanceSats }
        switch dest {
        case .bolt11, .bolt12, .lightningAddress, .lnurlPay:
            let readyChannels = appState.nodeService.channels.filter(\.isChannelReady)
            if !readyChannels.isEmpty {
                let channelOutbound = readyChannels.map(\.outboundCapacityMsat).reduce(0, +) / 1000
                return min(channelOutbound, appState.lightningBalanceSats)
            } else {
                return appState.lightningBalanceSats
            }
        case .onchain:
            // Mirror SendPaymentExecutor.sendOnchain: with a ready channel the send is a
            // splice-out funded from the channel, so only the channel's outbound balance is
            // spendable — not the combined lightning + on-chain total. Keyed on the live channel
            // list, not the cached hasReadyChannel flag, so the guard and the executor agree.
            let readyChannels = appState.nodeService.channels.filter(\.isChannelReady)
            if !readyChannels.isEmpty && !appState.isSweeping {
                let channelOutbound = readyChannels.map(\.outboundCapacityMsat).reduce(0, +) / 1000
                return min(channelOutbound, appState.lightningBalanceSats)
            }
            return appState.spendableOnchainSats
        }
    }

    func estimatedFeeSats(appState: AppState) -> UInt64 {
        let sats = computeEffectiveSats(btcPrice: appState.accountingBTCPrice)
        return estimatedFeeSatsForAmount(sats: sats, appState: appState)
    }

    func estimatedFeeSatsForAmount(sats: UInt64, appState: AppState) -> UInt64 {
        switch destination {
        case .bolt11, .bolt12, .lightningAddress, .lnurlPay:
            let readyChannel = appState.nodeService.channels.first(where: \.isChannelReady)
            let base = readyChannel?.counterpartyForwardingInfoFeeBaseMsat
                .map { UInt64($0) }
                ?? UInt64(Constants.lightningDefaultForwardingFeeBaseMsat)
            let prop = readyChannel?.counterpartyForwardingInfoFeeProportionalMillionths
                .map { UInt64($0) }
                ?? UInt64(Constants.lightningDefaultForwardingFeeProportionalMillionths)
            return PaymentFeeEstimator.estimateLightningFee(sats: sats, baseMsat: base, proportionalMillionths: prop)
        case .onchain:
            let isSpliceOut = appState.nodeService.channels.contains { $0.isChannelReady } && !appState.isSweeping
            if isSpliceOut {
                return 0
            }
            let rate = effectiveFeeRateSatVb ?? (feeRateSatVb ?? 10.0)
            return PaymentFeeEstimator.estimateOnchainFee(
                feeRateSatVb: rate,
                isSendAll: false
            )
        case .none:
            return 0
        }
    }

    /// Largest amount such that amount + fee(amount) fits inside the available balance.
    /// A proportional routing fee makes the fee depend on the amount, so a fixed number of
    /// passes can land on a candidate whose own fee no longer fits; iterate to a fixed point and
    /// then step down until the candidate provably fits.
    func calculateMaxSendableSats(appState: AppState) -> UInt64 {
        let available = availableSpendableSats(appState: appState)
        guard available > 0 else { return 0 }

        func fits(_ amount: UInt64) -> Bool {
            let fee = estimatedFeeSatsForAmount(sats: amount, appState: appState)
            return fee <= available && amount <= available - fee
        }

        var candidate = available
        for _ in 0..<8 {
            let fee = estimatedFeeSatsForAmount(sats: candidate, appState: appState)
            let next = available > fee ? (available - fee) : 0
            if next == candidate { break }
            candidate = next
        }
        var steps = 0
        while candidate > 0 && !fits(candidate) && steps < 64 {
            candidate -= 1
            steps += 1
        }
        return fits(candidate) ? candidate : 0
    }

    func isInsufficientBalance(appState: AppState) -> Bool {
        let sats = computeEffectiveSats(btcPrice: appState.accountingBTCPrice)
        let totalDebit = sats + estimatedFeeSats(appState: appState)
        let available = availableSpendableSats(appState: appState)
        return totalDebit > available || available == 0
    }

    func proceedFromAmount(appState: AppState) {
        normalizeAmountInput()
        errorMessage = nil

        let available = availableSpendableSats(appState: appState)
        guard available > 0 else {
            errorMessage = "Insufficient balance. Your available balance is 0 sats."
            return
        }

        let sats = computeEffectiveSats(btcPrice: appState.accountingBTCPrice)
        guard sats > 0 else {
            errorMessage = "Please enter an amount greater than 0."
            return
        }

        if let params = lnurlParams {
            if sats < params.minSats || sats > params.maxSats {
                errorMessage = "Amount must be between \(params.minSats) and \(params.maxSats) sats."
                return
            }
        }

        guard sats <= available else {
            let price = appState.accountingBTCPrice
            let availableUSD = price > 0 ? (Double(available) / Double(Constants.satsInBTC)) * price : 0
            if amountUnit == .usd && price > 0 {
                errorMessage = "Amount exceeds your balance. Available: $\(String(format: "%.2f", availableUSD)) (\(available.btcSpacedFormatted) BTC)"
            } else {
                errorMessage = "Amount exceeds your balance. Available: \(available.btcSpacedFormatted) BTC"
            }
            return
        }

        self.step = .confirm
    }

    func normalizeAmountInput() {
        amountInputText = SendAmountCalculator.normalizeInput(amountInputText, unit: amountUnit)
    }

    func computeEffectiveSats(btcPrice: Double) -> UInt64 {
        SendAmountCalculator.computeEffectiveSats(
            destination: destination,
            inputText: amountInputText,
            unit: amountUnit,
            btcPrice: btcPrice
        )
    }

    func switchUnit(to newUnit: SendAmountUnit, btcPrice: Double) {
        guard newUnit != amountUnit else { return }
        let sats = computeEffectiveSats(btcPrice: btcPrice)
        amountUnit = newUnit
        amountInputText = SendAmountCalculator.formatSatsForUnit(sats, unit: newUnit, btcPrice: btcPrice)
    }

    func applyPercentage(_ percent: Int, totalBalanceSats: UInt64, btcPrice: Double, appState: AppState? = nil) {
        if percent == 100, let appState {
            let targetSats = calculateMaxSendableSats(appState: appState)
            amountInputText = SendAmountCalculator.formatSatsForUnit(targetSats, unit: amountUnit, btcPrice: btcPrice)
        } else {
            amountInputText = SendAmountCalculator.calculatePercentageAmount(
                percent: percent,
                totalBalanceSats: totalBalanceSats,
                unit: amountUnit,
                btcPrice: btcPrice
            )
        }
    }

    func resetFlow() {
        step = .recipient
        inputText = ""
        destination = nil
        classification = .empty
        amountInputText = ""
        amountUnit = .usd
        lnurlParams = nil
        lnurlComment = ""
        errorMessage = nil
        successPaymentId = nil
        successTxid = nil
        sentAmountSats = 0
        isPendingSettlement = false
        isSending = false
        isFetchingLNURL = false
    }

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
            resetToken += 1
            return
        }
        let available = availableSpendableSats(appState: appState)
        let totalDebit = sats + estimatedFeeSats(appState: appState)
        guard totalDebit <= available, available > 0 else {
            errorMessage = "Amount exceeds your balance. Available: \(available.btcSpacedFormatted) BTC"
            resetToken += 1
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
                sentAmountSats = result.sentAmountSats
                successTxid = txid
                successPaymentId = nil
                isPendingSettlement = false
                step = .success

            case .spliceOut:
                // The splice confirms on-chain; there is no payment to await and no txid until
                // negotiation completes. Home shows it as a pending move meanwhile.
                sentAmountSats = result.sentAmountSats
                successTxid = nil
                successPaymentId = nil
                isPendingSettlement = true
                step = .success

            case .lightning(let pid):
                let timeout: TimeInterval
                if case .bolt12 = dest {
                    timeout = 10.0
                } else {
                    timeout = 7.0
                }

                let outcome = await SendPaymentExecutor.awaitPaymentSettlement(
                    paymentId: pid,
                    timeoutSeconds: timeout,
                    appState: appState
                )
                switch outcome {
                case .settled:
                    sentAmountSats = result.sentAmountSats
                    successPaymentId = pid
                    successTxid = nil
                    isPendingSettlement = false
                    step = .success
                case .failed(let reason):
                    errorMessage = reason
                case .timedOut:
                    sentAmountSats = result.sentAmountSats
                    successPaymentId = pid
                    successTxid = nil
                    isPendingSettlement = true
                    step = .success
                }
            }
        } catch {
            errorMessage = WalletErrorMessages.operation(error, fallback: error.localizedDescription)
        }
    }
}
