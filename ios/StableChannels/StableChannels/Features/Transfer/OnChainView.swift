import SwiftUI
import UIKit

struct OnChainSendView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    /// Single source of truth for this send's rules, shared with the review sheet.
    @State private var model = SendFlowModel()
    @State private var address = ""
    @State private var errorMessage: String?
    @State private var showReviewSheet = false

    private var amountSats: UInt64 {
        model.sendAmountSats(appState: appState)
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    addressCard
                    OnChainAmountCard(model: model)
                    if model.isSpliceOut(appState: appState) {
                        infoCard(
                            icon: "arrow.up.arrow.down",
                            text: String(
                                localized: "info_splice_out_funds",
                                defaultValue: "Funds will be sent via splice-out from your Lightning channel."
                            )
                        )
                    }
                    if let error = errorMessage {
                        errorCard(error)
                    }
                    reviewButton
                    Spacer(minLength: 12)
                }
                .padding(20)
            }
            .scrollDismissesKeyboard(.interactively)
            .background(Color(.systemGroupedBackground))
            .navigationTitle(String(localized: "title_send_on_chain", defaultValue: "Send Onchain"))
            .navigationBarTitleDisplayMode(.inline)
            .qrInputToolbar(text: $address, sanitize: QRCodeExtractor.sanitizeAddress)
            .sheet(isPresented: $showReviewSheet, onDismiss: {
                // Leaving the receipt, by Done or by swiping it away, also leaves this screen.
                if model.step == .success { dismiss() }
            }) {
                OnChainReviewSheet(address: address, model: model) {
                    showReviewSheet = false
                }
            }
            .onChange(of: address, initial: true) { _, new in
                model.destination = .onchain(address: new, amountSats: nil)
            }
            .task {
                model.feeRateSatVb = await appState.feeRateService.currentRate()
            }
        }
    }

    private var addressCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Image(systemName: "wallet.bifold")
                    .foregroundStyle(.secondary)
                Text(String(localized: "header_destination_address", defaultValue: "Destination Address"))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                Spacer()
            }

            TextField(String(localized: "placeholder_address", defaultValue: "bc1..."), text: $address)
                .font(.system(.body, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(12)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
                .overlay(
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(Color.primary.opacity(0.08), lineWidth: 1)
                )
                .onChange(of: address) { _, new in
                    address = QRCodeExtractor.sanitizeAddress(new)
                }
        }
        .padding(16)
        .glassCard()
    }

    private func infoCard(icon: String, text: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: icon)
                .foregroundStyle(.secondary)
            Text(text)
                .font(.footnote)
                .foregroundStyle(.secondary)
            Spacer()
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassCard()
    }

    private func errorCard(_ message: String) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.title2)
                .foregroundStyle(.red)
            Text(message)
                .font(.subheadline)
                .foregroundStyle(.red)
            Spacer()
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 16)
                .fill(.red.opacity(0.12))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 16)
                .stroke(.red.opacity(0.3), lineWidth: 1)
        )
    }

    private var reviewButton: some View {
        Button {
            errorMessage = nil
            showReviewSheet = true
        } label: {
            Text(String(localized: "button_review_transfer", defaultValue: "Review Transfer"))
                .fontWeight(.semibold)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 14)
        }
        .buttonStyle(.borderedProminent)
        .tint(.blue)
        .disabled(address.isEmpty || !model.canReviewOnchainAmount(appState: appState))
        .animation(.easeInOut(duration: 0.2), value: address.isEmpty)
        .animation(.easeInOut(duration: 0.2), value: amountSats)
    }
}

private struct GlassCardModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(.ultraThinMaterial)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(Color.primary.opacity(0.08), lineWidth: 1)
            )
    }
}

private extension View {
    func glassCard() -> some View { modifier(GlassCardModifier()) }
}
