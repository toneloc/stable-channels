import SwiftUI
import UIKit

/// Standalone review and confirmation sheet for onchain sends.
struct OnChainReviewSheet: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss

    let address: String
    let sendAll: Bool
    let amountSats: UInt64?
    let feeRateSatVb: Double?
    @Binding var selectedFeeTier: NetworkFeeSpeedTier
    var onSent: (_ txid: String?, _ isSplice: Bool) -> Void

    @State private var isSending = false
    @State private var reviewErrorMessage: String?
    @State private var resetToken = 0
    @State private var recommendedFees: RecommendedFees?

    private var effectiveFeeRateSatVb: Double? {
        if let rec = recommendedFees {
            return selectedFeeTier.effectiveRate(baseRate: rec.halfHourFee, recommendedFees: rec)
        }
        guard let base = feeRateSatVb else { return nil }
        return selectedFeeTier.effectiveRate(baseRate: base)
    }

    private var isFeeRateReady: Bool {
        recommendedFees != nil || feeRateSatVb != nil || selectedFeeTier == .standard
    }

    private var hasReadyChannel: Bool {
        appState.nodeService.channels.contains { $0.isChannelReady }
    }

    private var isSpliceOut: Bool {
        hasReadyChannel && !sendAll && !appState.isSweeping
    }

    private var estimatedFeeSats: UInt64 {
        if isSpliceOut {
            return 0
        }
        let rate = effectiveFeeRateSatVb ?? (feeRateSatVb ?? 10)
        return PaymentFeeEstimator.estimateOnchainFee(
            feeRateSatVb: rate,
            isSendAll: sendAll
        )
    }

    private var netReceivesSats: UInt64 {
        if sendAll {
            let fee = estimatedFeeSats
            let bal = appState.spendableOnchainSats
            return bal > fee ? bal - fee : 0
        }
        return amountSats ?? 0
    }

    private var totalDebitSats: UInt64 {
        sendAll ? appState.spendableOnchainSats : (netReceivesSats + estimatedFeeSats)
    }

    var body: some View {
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
                            amountSats: netReceivesSats,
                            btcPrice: appState.accountingBTCPrice
                        )

                        if !isSpliceOut {
                            NetworkFeeSelectorView(
                                selectedTier: $selectedFeeTier,
                                baseFeeRateSatVb: feeRateSatVb ?? 10,
                                isSendAll: sendAll,
                                btcPrice: appState.accountingBTCPrice,
                                showExplanation: false
                            )
                        }

                        SendConfirmFeeTotalCard(
                            feeLabel: isSpliceOut ? String(
                                localized: "label_routing_fee",
                                defaultValue: "Routing Fee"
                            ) :
                                String(
                                    localized: "label_total_fees",
                                    defaultValue: "Network Fee"
                                ),
                            estimatedFeeSats: estimatedFeeSats,
                            rateSatVb: isSpliceOut ? nil : effectiveFeeRateSatVb,
                            totalDebitSats: totalDebitSats,
                            btcPrice: appState.accountingBTCPrice
                        )

                        if let error = reviewErrorMessage {
                            HStack(spacing: 8) {
                                Image(systemName: "exclamationmark.circle.fill")
                                    .foregroundStyle(.red)
                                Text(error)
                                    .font(.footnote)
                                    .foregroundStyle(.red)
                                Spacer()
                            }
                            .padding(12)
                            .background(
                                Color(uiColor: .secondarySystemGroupedBackground),
                                in: RoundedRectangle(cornerRadius: 12)
                            )
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
                    .disabled(!isFeeRateReady)
                    .opacity(isFeeRateReady ? 1.0 : 0.5)
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
            let rec = await appState.feeRateService.recommendedFees()
            recommendedFees = rec
        }
    }

    private func executeSend() async {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )

        guard isFeeRateReady else {
            reviewErrorMessage = "Waiting for network fee rate. Please wait a moment."
            resetToken += 1
            return
        }

        let transactionAuth = UserDefaults.standard.bool(forKey: "transactionAuthEnabled")
        if transactionAuth {
            let authReason = sendAll ? "Confirm onchain withdrawal" : "Confirm onchain send"
            let authPassed = await appState.authenticate(reason: authReason)
            guard authPassed else {
                reviewErrorMessage = appState.authError ?? "Authentication required to send."
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

        let conversionPrice = sendAll ? nil : appState.accountingBTCPrice
        let sats: UInt64
        if sendAll {
            sats = 0
        } else if conversionPrice != nil, let converted = amountSats {
            sats = converted
        } else {
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
                    price: appState.btcPrice,
                    feeRateSatVb: effectiveFeeRateSatVb,
                    appState: appState
                )
                onSent(result.txid, false)
                dismiss()
            } else {
                let result = try await SendPaymentExecutor.sendOnchain(
                    address: address,
                    effectiveSats: sats,
                    price: conversionPrice ?? 0,
                    feeRateSatVb: effectiveFeeRateSatVb,
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
