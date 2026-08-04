import SwiftUI

struct AppAccessSettingsView: View {
    enum AuthTarget: String, Identifiable {
        case appUnlock = "biometricAuthEnabled"
        case transaction = "transactionAuthEnabled"

        var id: String { rawValue }

        var enableReason: String {
            switch self {
            case .appUnlock: return "Verify to enable App Unlock"
            case .transaction: return "Verify to enable Payment Confirmation"
            }
        }

        var disableTitle: String {
            switch self {
            case .appUnlock: return "Authenticate to disable App Unlock"
            case .transaction: return "Authenticate to disable Payment Confirmation"
            }
        }

        var disableSubtitle: String {
            switch self {
            case .appUnlock: return "Verify your identity to turn off App Unlock"
            case .transaction: return "Verify your identity to turn off Payment Confirmation"
            }
        }
    }

    // MARK: - Dependencies (Dependency Inversion)

    /// Capability checker — only exposes hardware detection, not auth methods.
    private let capability: BiometricCapabilityChecking

    /// Auth orchestration for toggle enable/disable (Single Responsibility).
    @State private var coordinator: BiometricToggleCoordinator

    // MARK: - State

    @State private var disableTarget: AuthTarget?

    init(
        capability: BiometricCapabilityChecking = BiometricService.shared,
        auth: BiometricAuthenticating = BiometricService.shared
    ) {
        self.capability = capability
        self._coordinator = State(initialValue: BiometricToggleCoordinator(auth: auth))
    }

    private func isEnabled(_ target: AuthTarget) -> Bool {
        UserDefaults.standard.bool(forKey: target.rawValue)
    }

    var body: some View {
        List {
            Section(String(localized: "section_wallet_security", defaultValue: "Wallet Security")) {
                Toggle(isOn: Binding(
                    get: { isEnabled(.appUnlock) },
                    set: { newValue in
                        if newValue {
                            Task { await coordinator.enableToggle(
                                AuthTarget.appUnlock.rawValue,
                                reason: AuthTarget.appUnlock.enableReason
                            ) }
                        } else {
                            disableTarget = .appUnlock
                        }
                    }
                )) {
                    Label { Text(String(localized: "label_app_unlock", defaultValue: "App Unlock")) }
                        icon: { Image(systemName: "faceid").foregroundStyle(.green) }
                }
                .disabled(capability.biometricType == .none || coordinator.isEnabling)

                Toggle(isOn: Binding(
                    get: { isEnabled(.transaction) },
                    set: { newValue in
                        if newValue {
                            Task { await coordinator.enableToggle(
                                AuthTarget.transaction.rawValue,
                                reason: AuthTarget.transaction.enableReason
                            ) }
                        } else {
                            disableTarget = .transaction
                        }
                    }
                )) {
                    Label {
                        Text(String(localized: "label_payment_confirmation", defaultValue: "Payment Confirmation"))
                    }
                    icon: { Image(systemName: "faceid").foregroundStyle(.green) }
                }
                .disabled(capability.biometricType == .none || coordinator.isEnabling)
            }
        }
        .navigationTitle(String(localized: "title_app_access", defaultValue: "App Access"))
        .navigationBarTitleDisplayMode(.inline)
        .sheet(item: $disableTarget) { target in
            ToggleAuthSheet(target: target, coordinator: coordinator)
        }
        .alert(
            "Face ID Access Required",
            isPresented: $coordinator.requiresSettingsRedirect
        ) {
            Button("Open Settings") {
                if let url = URL(string: UIApplication.openSettingsURLString) {
                    UIApplication.shared.open(url)
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "Face ID access is turned off for Stable Channels. Please enable it in Settings > Stable Channels > Face ID."
            )
        }
    }
}

// MARK: - Disable Auth Sheet

struct ToggleAuthSheet: View {
    let target: AppAccessSettingsView.AuthTarget
    let coordinator: BiometricToggleCoordinator
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                Image(systemName: "faceid")
                    .font(.system(size: 60))
                    .foregroundStyle(.green)

                Text(target.disableTitle)
                    .font(.headline)

                Text(target.disableSubtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                Button(String(localized: "button_continue", defaultValue: "Continue")) {
                    Task {
                        await coordinator.disableToggle(target.rawValue, reason: target.disableTitle)
                        dismiss()
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                Button(String(localized: "button_cancel", defaultValue: "Cancel")) {
                    dismiss()
                }
                .foregroundStyle(.secondary)
            }
            .padding()
            .navigationTitle(String(localized: "title_security", defaultValue: "Security"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "button_cancel", defaultValue: "Cancel")) { dismiss() }
                }
            }
        }
        .presentationDetents([.medium])
    }
}
