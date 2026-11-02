import SwiftUI

/// Home prompt for the optional email. Save or "Not now" answers it; swiping it away lets it come back later.
struct EmailPromptSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State private var model: EmailSettingsModel
    @State private var contentHeight: CGFloat = 0
    @State private var bounceCheckmark = false
    private let promptStore: EmailPromptStoring

    init(service: EmailSubmitting, store: EmailPromptStoring & SavedEmailStoring) {
        promptStore = store
        _model = State(initialValue: EmailSettingsModel(service: service, store: store, onSaved: store.markResponded))
    }

    private var isSaved: Bool {
        model.status == .saved
    }

    var body: some View {
        ScrollView {
            // The form stays in the layout under the success state so the sheet keeps its height.
            form
                .opacity(isSaved ? 0 : 1)
                .scaleEffect(isSaved ? 0.96 : 1)
                .allowsHitTesting(!isSaved)
                .accessibilityHidden(isSaved)
                .overlay {
                    if isSaved {
                        savedState
                            .transition(.opacity.combined(with: .scale(scale: 0.96)))
                    }
                }
                .animation(.easeInOut(duration: 0.3), value: isSaved)
                .padding(.horizontal, 24)
                .padding(.top, 32)
                .padding(.bottom, 8)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { contentHeight = $0 }
        }
        .scrollDismissesKeyboard(.interactively)
        .scrollBounceBehavior(.basedOnSize)
        // A height taller than the screen is clamped to the full sheet, and the content scrolls.
        .presentationDetents(contentHeight > 0 ? [.height(contentHeight)] : [.large])
        .presentationDragIndicator(.visible)
        .presentationBackground(Color(.systemGroupedBackground))
        .onChange(of: isSaved) {
            guard isSaved else { return }
            bounceCheckmark = true
            Task {
                try? await Task.sleep(for: .seconds(Constants.emailPromptSavedDismissDelaySeconds))
                dismiss()
            }
        }
    }

    private var form: some View {
        VStack(spacing: 0) {
            Image(systemName: "envelope.fill")
                .font(.title2)
                .foregroundStyle(Color.stablePrimary)
                .frame(width: Constants.emailBadgeSize, height: Constants.emailBadgeSize)
                .background(Color.stablePrimary.opacity(Constants.emailBadgeFillOpacity), in: Circle())
                .accessibilityHidden(true)
                .padding(.bottom, 16)

            Text(String(localized: "email_prompt_title", defaultValue: "Stay in touch"))
                .font(.title2.bold())
                .multilineTextAlignment(.center)
                .padding(.bottom, 8)

            Text(String(
                localized: "email_prompt_body",
                defaultValue: "Share your email and the Stable Channels team can ask how the wallet is working for you and tell you about important updates."
            ))
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.bottom, 24)

            EmailField(model: model)
                .padding(.bottom, 16)

            if let failure = model.failure {
                EmailErrorLine(error: failure)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.bottom, 10)
            }

            saveButton
                .padding(.bottom, 4)

            Button {
                promptStore.markResponded()
                dismiss()
            } label: {
                Text(String(localized: "button_not_now", defaultValue: "Not now"))
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: Constants.emailSecondaryButtonMinHeight)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.bottom, 8)

            Label {
                Text(String(
                    localized: "email_prompt_footnote",
                    defaultValue: "Optional. Tied to this wallet, never to your recovery phrase. Change or remove it anytime in Settings."
                ))
            } icon: {
                Image(systemName: "lock.fill")
                    .imageScale(.small)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var saveButton: some View {
        Button {
            Task { await model.save() }
        } label: {
            ZStack {
                if model.isSaving {
                    ProgressView()
                        .tint(Color.revealButtonLabel)
                } else {
                    Text(String(localized: "button_save_email", defaultValue: "Save email"))
                        .font(.headline)
                }
            }
            .foregroundStyle(Color.revealButtonLabel)
            .frame(maxWidth: .infinity, minHeight: Constants.emailPrimaryButtonMinHeight)
            .background(Color.revealButtonFill, in: Capsule())
            .opacity(model.isInputValid ? 1 : 0.4)
        }
        .disabled(!model.canSave)
    }

    private var savedState: some View {
        VStack(spacing: 12) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: Constants.emailBadgeSize))
                .foregroundStyle(.trendPositive)
                .symbolEffect(.bounce, value: bounceCheckmark)
                .accessibilityHidden(true)

            Text(String(localized: "email_saved_title", defaultValue: "Thanks!"))
                .font(.title2.bold())

            Text(String(
                localized: "email_saved_body",
                defaultValue: "We'll write to you at \(model.savedEmail?.value ?? "")."
            ))
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
