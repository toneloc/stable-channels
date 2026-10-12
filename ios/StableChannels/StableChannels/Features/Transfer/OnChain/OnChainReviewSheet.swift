import SwiftUI
import UIKit

/// Standalone review and confirmation sheet for onchain sends.
struct OnChainReviewSheet: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss

    let address: String
    /// The send's single source of truth, owned by the presenting view.
    @Bindable var model: SendFlowModel
    var onSent: (_ txid: String?, _ isSplice: Bool) -> Void

    @State private var isSending = false
    @State private var reviewErrorMessage: String?
    @State private var resetToken = 0

    var body: some View {
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

                        if let error = reviewErrorMessage {
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
                        isSending: isSending,
                        resetToken: resetToken
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
                    .disabled(isSending)
                }
            }
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .task {
            model.recommendedFees = await appState.feeRateService.recommendedFees()
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

        guard model.isFeeRateReady else {
            reviewErrorMessage = SendFlowModel.feeRateNotReadyMessage
            resetToken += 1
            return
        }

        guard !model.isInsufficientBalance(appState: appState) else {
            reviewErrorMessage = "Amount plus estimated fee exceeds your balance."
            resetToken += 1
            return
        }

        let transactionAuth = UserDefaults.standard.bool(forKey: "transactionAuthEnabled")
        if transactionAuth {
            let authReason = sendAll ? "Confirm onchain withdrawal" : "Confirm onchain send"
            let authPassed = await appState.authenticate(reason: authReason)
            guard authPassed else {
                reviewErrorMessage = appState.authError ?? SendFlowModel.authenticationRequiredMessage
                resetToken += 1
                return
            }
        }

        isSending = true
        reviewErrorMessage = nil
        defer {
            isSending = false
            resetToken += 1
        }

        let sats = model.sendAmountSats(appState: appState)
        if !sendAll, sats == 0 {
            reviewErrorMessage = String(
                localized: "error_price_unavailable",
                defaultValue: "The BTC price is unavailable or stale. Refresh and try again."
            )
            resetToken += 1
            return
        }

        do {
            if sendAll {
                let result = try await SendPaymentExecutor.sendAllOnchain(
                    address: address,
                    price: appState.accountingBTCPrice,
                    feeRateSatVb: model.effectiveFeeRateSatVb,
                    appState: appState
                )
                onSent(result.txid, false)
                dismiss()
            } else {
                let result = try await SendPaymentExecutor.sendOnchain(
                    address: address,
                    effectiveSats: sats,
                    price: appState.accountingBTCPrice,
                    feeRateSatVb: model.effectiveFeeRateSatVb,
                    appState: appState
                )
                if result.txid != nil {
                    onSent(result.txid, false)
                } else {
                    onSent(nil, true)
                }
                dismiss()
            }
        } catch {
            reviewErrorMessage = error.localizedDescription
            resetToken += 1
        }
    }
}
