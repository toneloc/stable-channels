import SwiftUI
import LocalAuthentication

struct AppAccessSettingsView: View {
    enum AuthTarget: String, Identifiable {
        case appUnlock = "biometricAuthEnabled"
        case transaction = "transactionAuthEnabled"

        var id: String { rawValue }

        var title: String {
            switch self {
            case .appUnlock: return "Authenticate to disable App Unlock"
            case .transaction: return "Authenticate to disable Payment Confirmation"
            }
        }

        var subtitle: String {
            switch self {
            case .appUnlock: return "Verify your identity to turn off App Unlock"
            case .transaction: return "Verify your identity to turn off Payment Confirmation"
            }
        }
    }

    @State private var authTarget: AuthTarget?

    private func isEnabled(_ target: AuthTarget) -> Bool {
        UserDefaults.standard.bool(forKey: target.rawValue)
    }

    var body: some View {
        List {
            Section("Wallet Security") {
                Toggle(isOn: Binding(
                    get: { isEnabled(.appUnlock) },
                    set: { if $0 { enable(.appUnlock) } else { requestAuth(for: .appUnlock) } }
                )) {
                    Label { Text("App Unlock") }
                        icon: { Image(systemName: "faceid").foregroundStyle(.green) }
                }
                .disabled(!BiometricService.canUseBiometrics)

                Toggle(isOn: Binding(
                    get: { isEnabled(.transaction) },
                    set: { if $0 { enable(.transaction) } else { requestAuth(for: .transaction) } }
                )) {
                    Label { Text("Payment Confirmation") }
                        icon: { Image(systemName: "faceid").foregroundStyle(.green) }
                }
                .disabled(!BiometricService.canUseBiometrics)
            }
        }
        .navigationTitle("App Access")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(item: $authTarget) { target in
            ToggleAuthSheet(target: target)
        }
    }

    private func enable(_ target: AuthTarget) {
        UserDefaults.standard.set(true, forKey: target.rawValue)
    }

    private func requestAuth(for target: AuthTarget) {
        authTarget = target
    }
}

struct ToggleAuthSheet: View {
    let target: AppAccessSettingsView.AuthTarget
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                Image(systemName: "faceid")
                    .font(.system(size: 60))
                    .foregroundStyle(.green)

                Text(target.title)
                    .font(.headline)

                Text(target.subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                Button("Continue") {
                    Task { await performAuth() }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                Button("Cancel") {
                    dismiss()
                }
                .foregroundStyle(.secondary)
            }
            .padding()
            .navigationTitle("Security")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium])
    }

    private func performAuth() async {
        do {
            try await BiometricService.authenticate(reason: target.title)
            UserDefaults.standard.set(false, forKey: target.rawValue)
        } catch {
            let passcodeOk = await (try? BiometricService.authenticateWithPasscode(reason: target.title)) ?? false
            if passcodeOk {
                UserDefaults.standard.set(false, forKey: target.rawValue)
            }
        }
        dismiss()
    }
}
