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
                            avatarData: model.lnurlParams?.avatarImageData
                        )
                    }

                    let sats = model.computeEffectiveSats(btcPrice: appState.accountingBTCPrice)
                    SendConfirmReceivesCard(
                        amountSats: sats,
                        btcPrice: appState.accountingBTCPrice
                    )

                    if case .onchain = model.destination, !isSpliceOut {
                        NetworkFeeSelectorView(
                            selectedTier: $model.selectedFeeTier,
                            baseFeeRateSatVb: model.feeRateSatVb ?? 10,
                            isSendAll: false,
                            btcPrice: appState.accountingBTCPrice,
                            showExplanation: false
                        )
                    }

                    SendConfirmFeeTotalCard(
                        feeLabel: feeCardLabel,
                        estimatedFeeSats: model.estimatedFeeSats(appState: appState),
                        rateSatVb: isLightning || isSpliceOut ? nil : model.effectiveFeeRateSatVb,
                        totalDebitSats: sats + model.estimatedFeeSats(appState: appState),
                        btcPrice: appState.accountingBTCPrice
                    )

                    if isInsufficientBalance {
                        errorBanner(String(
                            localized: "error_insufficient_balance_total",
                            defaultValue: "Insufficient balance. Total debit exceeds available funds."
                        ))
                    } else if let error = model.errorMessage {
                        errorBanner(error)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 10)
                .padding(.bottom, 6)
            }
            .scrollBounceBehavior(.basedOnSize)

            VStack(spacing: 4) {
                SlideToSendButton(
                    title: String(localized: "button_slide_to_send", defaultValue: "Slide to Send"),
                    isSending: model.isSending,
                    resetToken: model.resetToken
                ) {
                    Task { await model.executeSend(appState: appState) }
                }
                .disabled(isInsufficientBalance || !model.isFeeRateReady)
                .opacity(isInsufficientBalance || !model.isFeeRateReady ? 0.5 : 1.0)
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
            let rec = await appState.feeRateService.recommendedFees()
            model.recommendedFees = rec
            model.feeRateSatVb = Double(rec.halfHourFee)
        }
    }

    private var isLightning: Bool {
        guard let dest = model.destination else { return false }
        switch dest {
        case .bolt11, .bolt12, .lightningAddress, .lnurlPay: return true
        case .onchain: return false
        }
    }

    private var feeCardLabel: String {
        if isSpliceOut {
            return String(localized: "label_routing_fee", defaultValue: "Routing Fee")
        }
        return isLightning
            ? String(localized: "label_routing_fee", defaultValue: "Routing Fee")
            : String(localized: "label_total_fees", defaultValue: "Network Fee")
    }

    private var isSpliceOut: Bool {
        appState.hasReadyChannel && !appState.isSweeping
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
        case .bolt11: return String(localized: "header_invoice", defaultValue: "Lightning (BOLT11) Invoice")
        case .bolt12: return String(localized: "header_offer", defaultValue: "Lightning (BOLT12) Offer")
        case .lightningAddress, .lnurlPay: return String(localized: "header_recipient", defaultValue: "Recipient")
        case .onchain, .none: return String(localized: "header_address", defaultValue: "Recipient Address")
        }
    }

    private func errorBanner(_ message: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.red)
            Text(message).font(.footnote).foregroundStyle(.red)
            Spacer()
        }
        .padding(12)
        .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 12))
    }
}
