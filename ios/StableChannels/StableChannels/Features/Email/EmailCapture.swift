import SwiftUI

/// Form state for the optional email: validates the input, saves, changes or removes the address.
@MainActor
@Observable
final class EmailSettingsModel {
    enum Status: Equatable {
        case idle
        case saving
        case saved
        case failed(EmailSubmissionError)
    }

    var input = "" {
        didSet { if status != .saving { status = .idle } }
    }

    private(set) var status: Status = .idle
    private(set) var savedEmail: EmailAddress?
    /// True while Settings shows the field to replace an address that is already saved.
    private(set) var isChanging = false
    private var validationRequested = false
    private let service: EmailSubmitting
    private let store: SavedEmailStoring
    private let onSaved: () -> Void

    init(service: EmailSubmitting, store: SavedEmailStoring, onSaved: @escaping () -> Void = {}) {
        self.service = service
        self.store = store
        self.onSaved = onSaved
        savedEmail = store.savedEmail
    }

    var isInputValid: Bool {
        EmailAddress(input) != nil
    }

    var canSave: Bool {
        status != .saving && isInputValid
    }

    var isSaving: Bool {
        status == .saving
    }

    /// Shown only once the user tried to save or left the field, never while they are still typing.
    var showsInvalidHint: Bool {
        validationRequested && !isInputValid && !input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var failure: EmailSubmissionError? {
        if case let .failed(error) = status { return error }
        return nil
    }

    func fieldLostFocus() {
        validationRequested = true
    }

    func startChanging() {
        input = savedEmail?.value ?? ""
        isChanging = true
    }

    func save() async {
        guard status != .saving else { return }
        guard let email = EmailAddress(input) else {
            validationRequested = true
            return
        }
        status = .saving
        do {
            try await service.submit(email)
            store.savedEmail = email
            savedEmail = email
            isChanging = false
            status = .saved
            onSaved()
        } catch {
            status = .failed(error as? EmailSubmissionError ?? .unreachable)
        }
    }

    func remove() async {
        guard status != .saving else { return }
        status = .saving
        do {
            try await service.submit(nil)
            store.savedEmail = nil
            savedEmail = nil
            isChanging = false
            input = ""
            status = .idle
        } catch {
            status = .failed(error as? EmailSubmissionError ?? .unreachable)
        }
    }
}

/// Email input shared by the Home prompt and the Settings page.
struct EmailField: View {
    @Bindable var model: EmailSettingsModel
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Image(systemName: "envelope")
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)

                TextField(
                    String(localized: "placeholder_email", defaultValue: "you@example.com"),
                    text: $model.input
                )
                .keyboardType(.emailAddress)
                .textContentType(.emailAddress)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.done)
                .focused($focused)
                .onSubmit { Task { await model.save() } }

                if model.isInputValid {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.trendPositive)
                        .transition(.scale.combined(with: .opacity))
                        .accessibilityHidden(true)
                }
            }
            .padding(.horizontal, 16)
            .frame(height: Constants.emailFieldHeight)
            .background(
                Color(.secondarySystemGroupedBackground),
                in: RoundedRectangle(cornerRadius: Constants.emailFieldCornerRadius)
            )
            .overlay {
                if focused {
                    RoundedRectangle(cornerRadius: Constants.emailFieldCornerRadius)
                        .stroke(Color.stablePrimary, lineWidth: Constants.emailFieldFocusBorderWidth)
                }
            }
            .animation(.easeOut(duration: 0.15), value: model.isInputValid)
            .onChange(of: focused) {
                if !focused { model.fieldLostFocus() }
            }

            if model.showsInvalidHint {
                Text(String(localized: "email_invalid", defaultValue: "Enter a valid email address."))
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }
}

/// One red line explaining why the last save or removal failed.
struct EmailErrorLine: View {
    let error: EmailSubmissionError

    var body: some View {
        Text(message)
            .font(.caption)
            .foregroundStyle(.red)
    }

    private var message: String {
        switch error {
        case .walletNotReady:
            String(
                localized: "email_error_wallet_not_ready",
                defaultValue: "Your wallet is still starting. Try again in a moment."
            )
        case .unreachable:
            String(
                localized: "email_error_unreachable",
                defaultValue: "Couldn't reach Stable Channels. Check your connection and try again."
            )
        case .rejected:
            String(localized: "email_error_rejected", defaultValue: "Couldn't save your email. Try again.")
        }
    }
}
