import Foundation
import LDKNode
import SwiftUI

/// State and business orchestration for the Send workflow.
@Observable
@MainActor
final class SendFlowModel {
    static let feeRateNotReadyMessage = "Waiting for network fee rate. Please wait a moment."
    static let authenticationRequiredMessage = "Authentication required to send."

    var step: SendFlowStep = .recipient {
        didSet {
            // A quote belongs to one visit of the confirm step.
            if oldValue == .confirm, step != .confirm { invalidateLNURLQuote() }
        }
    }

    var inputText: String = "" {
        didSet {
            guard inputText != oldValue else { return }
            onInputChanged()
        }
    }

    var destination: SendDestination?
    var classification: PaymentDestinationClassification = .empty
    var amountUnit: SendAmountUnit = .usd
    var amountInputText: String = ""

    var lnurlParams: LNURLPayParams? {
        didSet { payeeAvatar = lnurlParams?.avatarImageData.flatMap(LNURLAvatar.thumbnail(from:)) }
    }

    /// Payee avatar decoded once per params change, bounded in bytes and pixels.
    private(set) var payeeAvatar: CGImage?
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
    /// The user chose Max: the amount is derived from the balance and the selected fee, not typed.
    var isMaxAmount: Bool = false
    /// Settings "Send All": empties the on-chain wallet in one send-all transaction even when a
    /// channel is ready, so unlike Max it is never a splice-out and spends the whole on-chain balance.
    var sendsAllFromOnchainWallet: Bool = false

    /// The LNURL invoice shown on the confirm step; the slide pays exactly this invoice.
    var lnurlQuote: LNURLInvoiceQuote?
    var isFetchingLNURLQuote: Bool = false
    var lnurlQuoteError: String?
    /// In-flight quote fetch; replaced or cancelled so a stale result never lands.
    @ObservationIgnored var lnurlQuoteTask: Task<Void, Never>?

    /// In-flight recipient resolution; owned here so the view can cancel it.
    @ObservationIgnored private var recipientTask: Task<Void, Never>?

    var successStatus: SendSuccessStatus {
        .of(isPendingSettlement: isPendingSettlement, txid: successTxid)
    }

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

    /// Base rate for the fee tiers: the loaded rate, or a placeholder until it loads.
    var baseFeeRateSatVb: Double {
        feeRateSatVb ?? Constants.placeholderFeeRateSatVb
    }

    /// True when the fee rate has loaded or the selected tier does not need one.
    var isFeeRateReady: Bool {
        if case .onchain = destination {
            return recommendedFees != nil || feeRateSatVb != nil
        }
        return true
    }

    /// Network the node runs on; destinations for another network are rejected as typed.
    var expectedNetwork: Network? {
        didSet {
            guard expectedNetwork != oldValue, !inputText.isEmpty else { return }
            onInputChanged()
        }
    }

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
        classification = PaymentDestinationClassifier.classify(inputText, network: expectedNetwork)
        switch classification {
        case .valid(let target):
            if destination != target {
                destination = target
                isMaxAmount = false
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
            isMaxAmount = false
            lnurlParams = nil
            lnurlComment = ""
            amountInputText = ""
        }
    }

    /// Starts recipient resolution unless one is already in flight.
    func startProceedFromRecipient(appState: AppState) {
        guard recipientTask == nil else { return }
        recipientTask = Task { [weak self] in
            guard let self, !Task.isCancelled else { return }
            await proceedFromRecipient(appState: appState)
            // Uncancelled means no cancel or reset replaced the handle, so it is still this task.
            if !Task.isCancelled { recipientTask = nil }
        }
    }

    /// Abandons an in-flight Lightning Address/LNURL fetch and returns to the recipient step.
    func cancelProceedFromRecipient() {
        recipientTask?.cancel()
        recipientTask = nil
        isFetchingLNURL = false
        errorMessage = nil
        step = .recipient
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
            // A cancelled fetch skips the reset: cancel already cleared it, and a newer fetch may own it now.
            defer { if !Task.isCancelled { isFetchingLNURL = false } }
            do {
                let params = try await service.fetchPayParams(from: url)
                guard !Task.isCancelled else { return }
                self.lnurlParams = params
                self.step = .amount
            } catch {
                guard !Task.isCancelled else { return }
                errorMessage = WalletErrorMessages.operation(error, fallback: error.localizedDescription)
            }
        case .bolt11(let invoice, _, let msat):
            let payability = InvoicePayability.evaluate(
                isExpired: invoice.isExpired(),
                existingOutboundPayment: appState.nodeService.outboundPaymentStatus(paymentHash: invoice.paymentHash())
            )
            if let message = payability.userMessage {
                errorMessage = message
                return
            }
            if let msat, msat > 0 {
                let requiredSats = PaymentFeeEstimator.maxLightningDebitSats(amountMsat: msat)
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

    /// What the wallet can debit for this send (amount plus fee). An LNURL payee's `maxSendable`
    /// bounds only the principal, so it is applied to the amount, never to this balance.
    func availableSpendableSats(appState: AppState) -> UInt64 {
        if sendsAllFromOnchainWallet { return appState.spendableOnchainSats }
        return SendChannelSpendPolicy.availableSpendableSats(
            destination: destination,
            channels: appState.nodeService.channels,
            lightningBalanceSats: appState.lightningBalanceSats,
            onchainBalanceSats: appState.spendableOnchainSats,
            totalBalanceSats: appState.totalBalanceSats,
            isSweeping: appState.isSweeping
        )
    }

    func estimatedFeeSats(appState: AppState) -> UInt64 {
        // A fixed-amount invoice is priced from its exact msat amount so no sub-sat remainder escapes the ceiling.
        if case .bolt11(_, _, let msat?) = destination, msat > 0 {
            return PaymentFeeEstimator.maxLightningRoutingFeeSats(amountMsat: msat)
        }
        return estimatedFeeSatsForAmount(sats: sendAmountSats(appState: appState), appState: appState)
    }

    func estimatedFeeSatsForAmount(sats: UInt64, appState: AppState) -> UInt64 {
        switch destination {
        case .bolt11, .bolt12, .lightningAddress, .lnurlPay:
            // The routing-fee ceiling the node enforces, so the shown and reserved fee is a real maximum.
            return PaymentFeeEstimator.maxLightningRoutingFeeSats(sats: sats)
        case .onchain:
            let rate = effectiveFeeRateSatVb ?? baseFeeRateSatVb
            if isSpliceOut(appState: appState) {
                return PaymentFeeEstimator.estimateSpliceOutFee(feeRateSatVb: rate)
            }
            // A Max or on-chain wallet send-all empties the wallet, which is a larger (send-all) transaction.
            return PaymentFeeEstimator.estimateOnchainFee(
                feeRateSatVb: rate,
                isSendAll: isMaxAmount || sendsAllFromOnchainWallet
            )
        case .none:
            return 0
        }
    }

    /// True when an on-chain send is dispatched as a splice-out: a ready channel, no sweep in
    /// progress, and not the on-chain wallet send-all.
    func isSpliceOut(appState: AppState) -> Bool {
        !sendsAllFromOnchainWallet && SendChannelSpendPolicy.isSpliceOut(
            channels: appState.nodeService.channels,
            isSweeping: appState.isSweeping
        )
    }

    /// Largest amount such that amount + fee(amount) fits inside the available balance, capped at an
    /// LNURL payee's `maxSendable`.
    func calculateMaxSendableSats(appState: AppState) -> UInt64 {
        SendChannelSpendPolicy.calculateMaxSendable(
            available: availableSpendableSats(appState: appState),
            principalCap: lnurlParams?.maxSats ?? .max
        ) { [weak self] amount in
            self?.estimatedFeeSatsForAmount(sats: amount, appState: appState) ?? 0
        }
    }

    /// The amount this send moves: the typed amount, or for Max the largest amount that fits after
    /// the fee at the currently selected tier, so every tier stays selectable and nothing is left over.
    /// For the on-chain wallet send-all it is what the recipient receives: the balance less the fee.
    func sendAmountSats(appState: AppState) -> UInt64 {
        // A quoted LNURL invoice fixes the amount, so what is shown, checked and paid cannot drift apart.
        if let lnurlQuote { return lnurlQuote.amountMsat / 1000 }
        if sendsAllFromOnchainWallet {
            let balance = availableSpendableSats(appState: appState)
            let fee = estimatedFeeSatsForAmount(sats: balance, appState: appState)
            return balance > fee ? balance - fee : 0
        }
        return isMaxAmount ? calculateMaxSendableSats(appState: appState) :
            computeEffectiveSats(btcPrice: appState.accountingBTCPrice)
    }

    /// What leaves the wallet: amount plus fee, or the whole on-chain balance for the on-chain wallet send-all.
    func totalDebitSats(appState: AppState) -> UInt64 {
        if sendsAllFromOnchainWallet { return availableSpendableSats(appState: appState) }
        return PaymentFeeEstimator.saturatingAdd(
            sendAmountSats(appState: appState),
            estimatedFeeSats(appState: appState)
        )
    }

    /// Max to an on-chain address without a splice-out: the node builds one transaction that empties
    /// the wallet and pays the selected fee out of it.
    func sendsEntireOnchainBalance(appState: AppState) -> Bool {
        SendChannelSpendPolicy.isOnchainSendAll(
            isMax: isMaxAmount,
            destination: destination,
            channels: appState.nodeService.channels,
            isSweeping: appState.isSweeping
        )
    }

    /// The amount shown for this send: the whole spendable balance for the on-chain wallet send-all.
    func displayedAmountSats(appState: AppState) -> UInt64 {
        sendsAllFromOnchainWallet ? availableSpendableSats(appState: appState) : sendAmountSats(appState: appState)
    }

    /// True when a typed amount is larger than the spendable balance; the send-all never exceeds it.
    func exceedsAvailableAmount(appState: AppState) -> Bool {
        !sendsAllFromOnchainWallet && sendAmountSats(appState: appState) > availableSpendableSats(appState: appState)
    }

    /// True when the amount allows opening the on-chain review: the send-all, or a non-zero amount within balance.
    func canReviewOnchainAmount(appState: AppState) -> Bool {
        sendsAllFromOnchainWallet
            || (sendAmountSats(appState: appState) > 0 && !exceedsAvailableAmount(appState: appState))
    }

    func isInsufficientBalance(appState: AppState) -> Bool {
        // The on-chain wallet send-all spends whatever the wallet holds; the node rejects an empty wallet.
        if sendsAllFromOnchainWallet { return false }
        let totalDebit = totalDebitSats(appState: appState)
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

        let sats = sendAmountSats(appState: appState)
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
            errorMessage = "Amount exceeds your balance. Available: \(balanceDescription(available, appState: appState))"
            return
        }

        let fee = estimatedFeeSatsForAmount(sats: sats, appState: appState)
        guard PaymentFeeEstimator.saturatingAdd(sats, fee) <= available else {
            errorMessage = "Amount plus estimated fee exceeds your balance. Available: \(balanceDescription(available, appState: appState))"
            return
        }

        self.step = .confirm
        loadLNURLQuoteIfNeeded(appState: appState)
    }

    /// "0.00 012 000 BTC (≈ $7.80)", omitting fiat when no price is available.
    private func balanceDescription(_ sats: UInt64, appState: AppState) -> String {
        let price = appState.accountingBTCPrice
        guard price > 0 else { return "\(sats.btcSpacedFormatted) BTC" }
        let usd = (Double(sats) / Double(Constants.satsInBTC)) * price
        return "\(sats.btcSpacedFormatted) BTC (≈ \(usd.usdFormatted))"
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
            isMaxAmount = true
            refreshMaxAmountText(appState: appState)
        } else {
            isMaxAmount = false
            // An LNURL payee's max bounds the principal, so a share of the balance never exceeds it.
            let base = lnurlParams.map { min(totalBalanceSats, $0.maxSats) } ?? totalBalanceSats
            amountInputText = SendAmountCalculator.calculatePercentageAmount(
                percent: percent,
                totalBalanceSats: base,
                unit: amountUnit,
                btcPrice: btcPrice
            )
        }
    }

    /// The user edited the amount field: Max holds only while the text still equals the Max amount.
    func amountEdited(appState: AppState) {
        guard isMaxAmount else { return }
        let maxText = SendAmountCalculator.formatSatsForUnit(
            calculateMaxSendableSats(appState: appState),
            unit: amountUnit,
            btcPrice: appState.accountingBTCPrice
        )
        if amountInputText != maxText { isMaxAmount = false }
    }

    /// Rewrites the amount field to the current Max amount (after a fee tier or rate change).
    /// Applies fetched recommended fees: the selector's tiers and the base rate (the half-hour
    /// tier), then refreshes a Max amount so its text reflects the real rate, not the placeholder.
    func applyRecommendedFees(_ fees: RecommendedFees, appState: AppState) {
        recommendedFees = fees
        feeRateSatVb = Double(fees.halfHourFee)
        refreshMaxAmountText(appState: appState)
    }

    func refreshMaxAmountText(appState: AppState) {
        guard isMaxAmount else { return }
        amountInputText = SendAmountCalculator.formatSatsForUnit(
            calculateMaxSendableSats(appState: appState),
            unit: amountUnit,
            btcPrice: appState.accountingBTCPrice
        )
    }

    func resetFlow() {
        recipientTask?.cancel()
        recipientTask = nil
        invalidateLNURLQuote()
        step = .recipient
        inputText = ""
        destination = nil
        classification = .empty
        amountInputText = ""
        amountUnit = .usd
        isMaxAmount = false
        sendsAllFromOnchainWallet = false
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
}
