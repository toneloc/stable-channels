import SwiftUI

struct EmailSettingsView: View {
    @State private var model: EmailSettingsModel
    @State private var confirmingRemove = false

    init(service: EmailSubmitting, store: EmailPromptStoring & SavedEmailStoring = EmailPromptStore.shared) {
        _model = State(initialValue: EmailSettingsModel(service: service, store: store, onSaved: store.markResponded))
    }

    var body: some View {
        List {
            if let saved = model.savedEmail, !model.isChanging {
                savedSections(saved)
            } else {
                entrySections
            }
        }
        .navigationTitle(String(localized: "title_email", defaultValue: "Email"))
        .navigationBarTitleDisplayMode(.inline)
        .alert(
            String(localized: "email_remove_title", defaultValue: "Remove your email?"),
            isPresented: $confirmingRemove
        ) {
            Button(String(localized: "button_remove", defaultValue: "Remove"), role: .destructive) {
                Task { await model.remove() }
            }
            Button(String(localized: "button_cancel", defaultValue: "Cancel"), role: .cancel) {}
        } message: {
            Text(String(
                localized: "email_remove_message",
                defaultValue: "We'll delete it from the Stable Channels server."
            ))
        }
    }

    @ViewBuilder
    private var entrySections: some View {
        Section {
            EmailField(model: model)
                .listRowInsets(EdgeInsets())
                .listRowBackground(Color.clear)
        } footer: {
            footerText
        }

        Section {
            Button {
                Task { await model.save() }
            } label: {
                HStack {
                    Text(String(localized: "button_save_email", defaultValue: "Save email"))
                    Spacer()
                    if model.isSaving { ProgressView() }
                }
            }
            .tint(Color.stablePrimary)
            .disabled(!model.canSave)
        } footer: {
            errorLine
        }
    }

    @ViewBuilder
    private func savedSections(_ saved: EmailAddress) -> some View {
        Section {
            HStack {
                Text(saved.value)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                Label(String(localized: "email_status_saved", defaultValue: "Saved"), systemImage: "checkmark")
                    .font(.subheadline)
                    .foregroundStyle(.trendPositive)
            }
        } header: {
            Text(String(localized: "email_section_yours", defaultValue: "Your email"))
        }

        Section {
            Button(String(localized: "button_change_email", defaultValue: "Change email")) {
                model.startChanging()
            }
            .tint(Color.stablePrimary)

            Button(role: .destructive) {
                confirmingRemove = true
            } label: {
                HStack {
                    Text(String(localized: "button_remove_email", defaultValue: "Remove email"))
                    Spacer()
                    if model.isSaving { ProgressView() }
                }
            }
            .disabled(model.isSaving)
        } footer: {
            VStack(alignment: .leading, spacing: 8) {
                errorLine
                footerText
            }
        }
    }

    @ViewBuilder
    private var errorLine: some View {
        if let failure = model.failure {
            EmailErrorLine(error: failure)
        }
    }

    private var footerText: some View {
        Text(String(
            localized: "email_settings_footer",
            defaultValue: "The Stable Channels team may write to ask how the wallet is working for you or to tell you about important updates. No spam, and we will never ask for your recovery phrase."
        ))
    }
}
