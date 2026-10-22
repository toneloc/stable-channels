import SwiftUI

/// Primary coordinator container for the multi-step send workflow.
struct SendView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss

    @State private var model: SendFlowModel

    init(initialInput: String? = nil, model: SendFlowModel? = nil) {
        let initialModel = model ?? SendFlowModel()
        if let initialInput, !initialInput.isEmpty {
            initialModel.inputText = initialInput
        }
        _model = State(initialValue: initialModel)
    }

    var body: some View {
        NavigationStack {
            ZStack {
                Color(uiColor: .systemGroupedBackground)
                    .ignoresSafeArea()

                if model.isFetchingLNURL {
                    SendLoadingView(
                        title: String(localized: .titleResolvingLnurl),
                        subtitle: String(localized: .subtitleResolvingLnurl),
                        curve: .roseCurve,
                        tint: .orange
                    )
                    .transition(.opacity)
                } else {
                    stepContent
                        .animation(.easeInOut(duration: 0.25), value: model.step)
                }
            }
            .navigationTitle(navigationTitle)
            .navigationBarTitleDisplayMode(.inline)
            .navigationBarBackButtonHidden(true)
            .onChange(of: appState.nodeService.activeNetwork, initial: true) { _, network in
                model.expectedNetwork = network
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    leadingToolbarButton
                }
            }
        }
    }

    @ViewBuilder
    private var stepContent: some View {
        switch model.step {
        case .recipient:
            SendRecipientStepView(model: model)
        case .amount:
            SendAmountStepView(model: model)
        case .confirm:
            SendConfirmStepView(model: model)
        case .success:
            SendSuccessStepView(model: model) {
                dismiss()
            }
        }
    }

    private var navigationTitle: String {
        if model.isFetchingLNURL {
            return ""
        }
        switch model.step {
        case .recipient:
            return String(localized: .titleSend)
        case .amount:
            return String(localized: .titleSendAmount)
        case .confirm:
            return String(localized: .titleSendConfirm)
        case .success:
            return model.isPendingSettlement
                ? String(localized: .titleSendPending)
                : String(localized: .titleSendSuccess)
        }
    }

    @ViewBuilder
    private var leadingToolbarButton: some View {
        if model.isFetchingLNURL {
            Button(String(localized: .buttonCancel)) {
                model.cancelProceedFromRecipient()
            }
        } else {
            switch model.step {
            case .recipient:
                Button(String(localized: .buttonDone)) {
                    dismiss()
                }
            case .amount:
                Button {
                    withAnimation(.easeInOut(duration: 0.2)) {
                        model.step = .recipient
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.left")
                        Text(String(localized: .buttonBack))
                    }
                }
            case .confirm:
                Button {
                    withAnimation(.easeInOut(duration: 0.2)) {
                        if case .bolt11(_, _, let msat) = model.destination, let msat, msat > 0 {
                            model.step = .recipient
                        } else {
                            model.step = .amount
                        }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.left")
                        Text(String(localized: .buttonBack))
                    }
                }
                // Leaving mid-send would surface its result on the wrong step.
                .disabled(model.isSending)
            case .success:
                EmptyView()
            }
        }
    }
}

#Preview("Send View - Recipient Step") {
    SendView()
        .environment(AppState())
}

#Preview("Send View - Amount Step") {
    let model = SendFlowModel()
    model.step = .amount
    model.destination = .lightningAddress(
        handle: "prabal",
        domain: "0xprabal.com",
        url: URL(string: "https://0xprabal.com/.well-known/lnurlp/prabal")!
    )
    return SendView(model: model)
        .environment(AppState())
}

#Preview("Send View - Confirm Step") {
    let model = SendFlowModel()
    model.step = .confirm
    model.amountInputText = "21.00"
    model.destination = .lightningAddress(
        handle: "prabal",
        domain: "0xprabal.com",
        url: URL(string: "https://0xprabal.com/.well-known/lnurlp/prabal")!
    )
    return SendView(model: model)
        .environment(AppState())
}

#Preview("Send View - Success Step") {
    let model = SendFlowModel()
    model.step = .success
    model.destination = .lightningAddress(
        handle: "prabal",
        domain: "0xprabal.com",
        url: URL(string: "https://0xprabal.com/.well-known/lnurlp/prabal")!
    )
    model.sentAmountSats = 35_000
    model.successPaymentId = "483dc85d1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d"
    return SendView(model: model)
        .environment(AppState())
}
