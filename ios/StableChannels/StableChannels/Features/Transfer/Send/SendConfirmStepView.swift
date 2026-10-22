import SwiftUI

/// Step 3: Transaction review with visual address chunking and Slide to Send confirmation.
struct SendConfirmStepView: View {
    @Bindable var model: SendFlowModel
    @Environment(AppState.self) private var appState

    var body: some View {
        VStack(spacing: 0) {
            ScrollView(showsIndicators: false) {
                VStack(spacing: 10) {
                    SendConfirmAssetCard(routeDescription: sourceRouteDescription)

                    if let dest = model.destination {
                        SendConfirmAddressCard(
                            headerTitle: addressHeaderTitle,
                            representation: AddressVisualChunker.formatDestination(dest),
                            rawAddress: dest.rawDestination,
                            avatar: model.payeeAvatar
                        )
                    }

                    let sats = model.sendAmountSats(appState: appState)
                    SendConfirmReceivesCard(
                        amountSats: sats,
                        btcPrice: appState.accountingBTCPrice
                    )

                    if isLNURL {
                        SendConfirmLNURLInvoiceCard(
                            isLoading: model.isFetchingLNURLQuote,
                            errorMessage: model.lnurlQuoteError
                        ) {
                            model.startLNURLQuoteFetch(appState: appState)
                        }
                    }

                    if case .onchain = model.destination, !isSpliceOut {
                        NetworkFeeSelectorView(
                            selectedTier: $model.selectedFeeTier,
                            baseFeeRateSatVb: model.baseFeeRateSatVb,
                            isSendAll: model.sendsEntireOnchainBalance(appState: appState),
                            btcPrice: appState.accountingBTCPrice,
                            showExplanation: false,
                            recommendedFees: model.recommendedFees
                        )
                    }

                    SendConfirmFeeTotalCard(
                        feeLabel: feeCardLabel,
                        estimatedFeeSats: model.estimatedFeeSats(appState: appState),
                        rateSatVb: isLightning ? nil : model.effectiveFeeRateSatVb,
                        totalDebitSats: PaymentFeeEstimator.saturatingAdd(
                            sats,
                            model.estimatedFeeSats(appState: appState)
                        ),
                        btcPrice: appState.accountingBTCPrice
                    )

                    if isInsufficientBalance {
                        SendErrorBanner(message: String(localized: .errorInsufficientBalanceTotal))
                    } else if let error = model.errorMessage {
                        SendErrorBanner(message: error)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 10)
                .padding(.bottom, 6)
            }
            .scrollBounceBehavior(.basedOnSize)

            VStack(spacing: 4) {
                SlideToSendButton(
                    title: String(localized: .buttonSlideToSend),
                    isSending: model.isSending,
                    resetToken: model.resetToken
                ) {
                    Task { await model.executeSend(appState: appState) }
                }
                .disabled(isSlideDisabled)
                .opacity(isSlideDisabled ? 0.5 : 1.0)

                if !model.isFeeRateReady {
                    Text(String(localized: .labelLoadingNetworkFee))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 6)
            .padding(.bottom, 16)
        }
        .onAppear {
            UIApplication.shared.sendAction(
                #selector(UIResponder.resignFirstResponder),
                to: nil,
                from: nil,
                for: nil
            )
        }
        .task {
            model.loadLNURLQuoteIfNeeded(appState: appState)
            await model.applyRecommendedFees(appState.feeRateService.recommendedFees(), appState: appState)
        }
    }

    private var isLightning: Bool {
        guard let dest = model.destination else { return false }
        switch dest {
        case .bolt11, .bolt12, .lightningAddress, .lnurlPay: return true
        case .onchain: return false
        }
    }

    private var isLNURL: Bool {
        switch model.destination {
        case .lightningAddress, .lnurlPay: true
        default: false
        }
    }

    /// The slide waits for funds, the network fee rate, and for an LNURL send the invoice to pay.
    private var isSlideDisabled: Bool {
        isInsufficientBalance || !model.isFeeRateReady || !model.isLNURLQuoteReady
    }

    private var isSpliceOut: Bool {
        guard case .onchain = model.destination else { return false }
        return SendChannelSpendPolicy.isSpliceOut(
            channels: appState.nodeService.channels,
            isSweeping: appState.isSweeping
        )
    }

    private var feeCardLabel: String {
        if isSpliceOut {
            return String(localized: .labelEstimatedSpliceFee)
        }
        if isLightning {
            return String(localized: .labelMaxRoutingFee)
        }
        // Sending everything: the node sets the exact fee when it builds the transaction.
        return model.sendsEntireOnchainBalance(appState: appState)
            ? String(localized: .labelEstimatedNetworkFee)
            : String(localized: .labelTotalFees)
    }

    private var isInsufficientBalance: Bool {
        model.isInsufficientBalance(appState: appState)
    }

    private var sourceRouteDescription: String {
        switch model.destination {
        case .bolt11:
            return "Lightning (BOLT11) • Instant"
        case .bolt12:
            return "Lightning (BOLT12) • Instant"
        case .lightningAddress, .lnurlPay:
            return "Lightning • Instant"
        case .onchain:
            return isSpliceOut ? "Onchain • Splice-Out" : "Onchain • Standard"
        case .none:
            return "Standard"
        }
    }

    private var addressHeaderTitle: String {
        switch model.destination {
        case .bolt11: return String(localized: .headerInvoice)
        case .bolt12: return String(localized: .headerOffer)
        case .lightningAddress, .lnurlPay: return String(localized: .headerRecipient)
        case .onchain, .none: return String(localized: .headerAddress)
        }
    }
}
