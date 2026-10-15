import SwiftUI
import UIKit

/// Standalone review and confirmation sheet for onchain sends.
struct OnChainReviewSheet: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss

    let address: String
    /// The send's single source of truth, owned by the presenting view.
    @Bindable var model: SendFlowModel
    var onDone: () -> Void

    var body: some View {
        Group {
            if model.step == .success {
                SendSuccessStepView(model: model, onDismiss: onDone)
                    .background(Color(.systemGroupedBackground))
            } else {
                reviewContent
            }
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .task {
            // The model outlives the sheet, so each presentation starts without a previous attempt's error.
            model.errorMessage = nil
            model.recommendedFees = await appState.feeRateService.recommendedFees()
        }
    }

    /// Review step; after a successful send the sheet shows the shared send receipt instead.
    @ViewBuilder
    private var reviewContent: some View {
        let isSpliceOut = model.isSpliceOut(appState: appState)
        let isInsufficientBalance = model.isInsufficientBalance(appState: appState)
        NavigationStack {
            VStack(spacing: 0) {
                ScrollView(showsIndicators: false) {
                    VStack(spacing: 12) {
                        SendConfirmAssetCard(
                            routeDescription: isSpliceOut ? "Onchain • Splice-Out" : "Onchain • Standard"
                        )

                        SendConfirmAddressCard(
                            headerTitle: String(localized: "header_address", defaultValue: "Recipient Address"),
                            representation: .onchain(AddressVisualChunker.chunkAddress(address)),
                            rawAddress: address
                        )

                        SendConfirmReceivesCard(
                            amountSats: model.sendAmountSats(appState: appState),
                            btcPrice: appState.accountingBTCPrice
                        )

                        // Tier fees assume a regular on-chain spend; a splice-out is priced by the
                        // splice fee in the total card below, as on the send review step.
                        if !isSpliceOut {
                            NetworkFeeSelectorView(
                                selectedTier: $model.selectedFeeTier,
                                baseFeeRateSatVb: model.baseFeeRateSatVb,
                                isSendAll: model.sendsAllFromOnchainWallet,
                                btcPrice: appState.accountingBTCPrice,
                                showExplanation: false,
                                recommendedFees: model.recommendedFees
                            )
                        }

                        SendConfirmFeeTotalCard(
                            feeLabel: isSpliceOut ? String(
                                localized: "label_splice_fee",
                                defaultValue: "Splice Fee"
                            ) :
                                String(
                                    localized: "label_total_fees",
                                    defaultValue: "Network Fee"
                                ),
                            estimatedFeeSats: model.estimatedFeeSats(appState: appState),
                            rateSatVb: model.effectiveFeeRateSatVb,
                            totalDebitSats: model.totalDebitSats(appState: appState),
                            btcPrice: appState.accountingBTCPrice
                        )

                        if let error = model.errorMessage {
                            SendErrorBanner(message: error)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 12)
                    .padding(.bottom, 8)
                }
                .scrollBounceBehavior(.basedOnSize)

                VStack(spacing: 4) {
                    SlideToSendButton(
                        title: String(localized: "button_slide_to_send", defaultValue: "Slide to Send"),
                        isSending: model.isSending,
                        resetToken: model.resetToken
                    ) {
                        Task { await executeSend() }
                    }
                    .disabled(!model.isFeeRateReady || isInsufficientBalance)
                    .opacity(model.isFeeRateReady && !isInsufficientBalance ? 1.0 : 0.5)
                }
                .padding(.horizontal, 16)
                .padding(.top, 8)
                .padding(.bottom, 16)
            }
            .background(Color(.systemGroupedBackground))
            .navigationTitle(String(localized: "title_confirm_transaction", defaultValue: "Confirm transaction"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_cancel", defaultValue: "Cancel")) {
                        dismiss()
                    }
                    .disabled(model.isSending)
                }
            }
        }
    }

    private func executeSend() async {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )

        let sendAll = model.sendsAllFromOnchainWallet
        let authReason = sendAll ? "Confirm onchain withdrawal" : "Confirm onchain send"
        await model.performSend(appState: appState, authReason: authReason) { sats in
            let result: SendPaymentResult
            if sendAll {
                result = try await SendPaymentExecutor.sendAllOnchain(
                    address: address,
                    price: appState.accountingBTCPrice,
                    feeRateSatVb: model.effectiveFeeRateSatVb,
                    appState: appState
                )
            } else {
                result = try await SendPaymentExecutor.sendOnchain(
                    address: address,
                    effectiveSats: sats,
                    price: appState.accountingBTCPrice,
                    feeRateSatVb: model.effectiveFeeRateSatVb,
                    appState: appState
                )
            }
            // A splice-out has no txid until negotiation completes, so its receipt stays pending.
            model.showSuccess(
                sentSats: sendAll ? sats : result.sentAmountSats,
                txid: result.txid,
                isPending: result.txid == nil
            )
        }
    }
}
