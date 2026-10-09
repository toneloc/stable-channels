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
                        title: String(localized: "title_resolving_lnurl", defaultValue: "Resolving Lightning Address"),
                        subtitle: String(
                            localized: "subtitle_resolving_lnurl",
                            defaultValue: "Fetching invoice parameters from payee server..."
                        ),
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
            return String(localized: "title_send", defaultValue: "Send")
        case .amount:
            return String(localized: "title_send_amount", defaultValue: "Amount")
        case .confirm:
            return String(localized: "title_send_confirm", defaultValue: "Review")
        case .success:
            return model.isPendingSettlement
                ? String(localized: "title_send_pending", defaultValue: "Payment Pending")
                : String(localized: "title_send_success", defaultValue: "Payment Sent")
        }
    }

    @ViewBuilder
    private var leadingToolbarButton: some View {
        if model.isFetchingLNURL {
            EmptyView()
        } else {
            switch model.step {
            case .recipient:
                Button(String(localized: "button_done", defaultValue: "Done")) {
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
                        Text(String(localized: "button_back", defaultValue: "Back"))
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
                        Text(String(localized: "button_back", defaultValue: "Back"))
                    }
                }
            case .success:
                EmptyView()
            }
        }
    }
}
