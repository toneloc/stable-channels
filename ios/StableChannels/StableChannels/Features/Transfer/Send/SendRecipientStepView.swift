import SwiftUI

/// Step 1: Destination input, native toolbar QR/Photo scanning, and subtle protocol recognition.
struct SendRecipientStepView: View {
    @Bindable var model: SendFlowModel
    @Environment(AppState.self) private var appState
    @FocusState private var isInputFocused: Bool

    var body: some View {
        VStack(spacing: 16) {
            recipientCard

            if let error = model.errorMessage {
                errorBanner(error)
            } else {
                SendDestinationBadgeView(classification: model.classification)
            }

            availableBalanceFooter

            Spacer(minLength: 20)

            continueButton
        }
        .padding(.horizontal, 16)
        .padding(.top, 16)
        .onAppear {
            isInputFocused = true
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 120_000_000)
                isInputFocused = true
            }
        }
        .qrInputToolbar(text: $model.inputText, sanitize: QRCodeExtractor.sanitizePaymentInput)
    }

    private var recipientCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(String(localized: "header_recipient", defaultValue: "To"))
                .font(.subheadline.weight(.medium))
                .foregroundStyle(.secondary)

            HStack(alignment: .top, spacing: 8) {
                TextField(
                    String(
                        localized: "placeholder_send_destination",
                        defaultValue: "Address, invoice, or name@domain.com"
                    ),
                    text: $model.inputText,
                    axis: .vertical
                )
                .font(.system(.body, design: .monospaced))
                .lineLimit(3...5)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .focused($isInputFocused)

                if model.inputText.isEmpty {
                    Button {
                        if let clipboard = UIPasteboard.general.string {
                            UISelectionFeedbackGenerator().selectionChanged()
                            model.inputText = QRCodeExtractor.sanitizePaymentInput(clipboard)
                        }
                    } label: {
                        Image(systemName: "doc.on.clipboard")
                            .font(.body)
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .padding(.top, 2)
                    .accessibilityLabel(Text(String(localized: "button_paste", defaultValue: "Paste")))
                } else {
                    Button {
                        UISelectionFeedbackGenerator().selectionChanged()
                        model.inputText = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.body)
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .padding(.top, 2)
                }
            }
        }
        .padding(16)
        .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 14))
        .contentShape(Rectangle())
        .onTapGesture {
            isInputFocused = true
        }
    }

    @ViewBuilder
    private var availableBalanceFooter: some View {
        if appState.btcPrice > 0 {
            let usd = Double(appState.totalBalanceSats) / Double(Constants.satsInBTC) * appState.btcPrice
            HStack(spacing: 4) {
                Text(String(localized: "available_balance", defaultValue: "Available: "))
                Text(verbatim: usd.usdFormatted)
            }
            .font(.footnote)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 4)
        }
    }

    private var continueButton: some View {
        Button {
            isInputFocused = false
            Task { await model.proceedFromRecipient(appState: appState) }
        } label: {
            Text(String(localized: "button_continue", defaultValue: "Continue"))
                .font(.headline)
                .frame(maxWidth: .infinity)
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .tint(Color.blue)
        .disabled(model.destination == nil || model.isFetchingLNURL)
        .padding(.bottom, 16)
    }

    private func errorBanner(_ message: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.circle.fill")
                .foregroundStyle(.red)
            Text(message)
                .font(.footnote)
                .foregroundStyle(.red)
            Spacer()
        }
        .padding(12)
        .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 12))
    }
}
