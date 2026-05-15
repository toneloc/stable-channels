import SwiftUI

struct AppAccessSettingsView: View {
    @AppStorage("biometricAuthEnabled") private var biometricAuthEnabled = false
    @AppStorage("transactionAuthEnabled") private var transactionAuthEnabled = false
    @State private var showAuthForToggle = false
    @State private var pendingTarget: AuthTarget?

    enum AuthTarget: String, CaseIterable {
        case appUnlock
        case transaction

        var userDefaultsKey: String {
            switch self {
            case .appUnlock: return "biometricAuthEnabled"
            case .transaction: return "transactionAuthEnabled"
            }
        }

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

    var body: some View {
        List {
            Section("Wallet Security") {
                Toggle(isOn: Binding(
                    get: { biometricAuthEnabled },
                    set: { newValue in
                        if newValue {
                            biometricAuthEnabled = true
                        } else {
                            pendingTarget = .appUnlock
                            showAuthForToggle = true
                        }
                    }
                )) {
                    Label {
                        Text("App Unlock")
                    } icon: {
                        Image(systemName: "faceid")
                            .foregroundStyle(.green)
                    }
                }
                .disabled(!BiometricService.canUseBiometrics)

                Toggle(isOn: Binding(
                    get: { transactionAuthEnabled },
                    set: { newValue in
                        if newValue {
                            transactionAuthEnabled = true
                        } else {
                            pendingTarget = .transaction
                            showAuthForToggle = true
                        }
                    }
                )) {
                    Label {
                        Text("Payment Confirmation")
                    } icon: {
                        Image(systemName: "faceid")
                            .foregroundStyle(.green)
                    }
                }
                .disabled(!BiometricService.canUseBiometrics)
            }
        }
        .navigationTitle("App Access")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $showAuthForToggle) {
            ToggleAuthSheet(
                isPresented: $showAuthForToggle,
                authTarget: pendingTarget,
                onAuthenticated: { confirmed in
                    guard confirmed, let target = pendingTarget else { return }
                    UserDefaults.standard.set(false, forKey: target.userDefaultsKey)
                    pendingTarget = nil
                }
            )
        }
    }
}

struct ToggleAuthSheet: View {
    @Binding var isPresented: Bool
    var authTarget: AppAccessSettingsView.AuthTarget?
    var onAuthenticated: (Bool) -> Void

    private var titleText: String {
        authTarget?.title ?? "Authenticate"
    }

    private var subtitleText: String {
        authTarget?.subtitle ?? "Verify your identity to continue"
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                Image(systemName: "faceid")
                    .font(.system(size: 60))
                    .foregroundStyle(.green)

                Text(titleText)
                    .font(.headline)

                Text(subtitleText)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)

                Button("Continue") {
                    Task {
                        isPresented = false
                        do {
                            let success = try await BiometricService.authenticate(reason: titleText)
                            onAuthenticated(success)
                        } catch {
                            onAuthenticated(false)
                        }
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                Button("Cancel") {
                    isPresented = false
                }
                .foregroundStyle(.secondary)
            }
            .padding()
            .navigationTitle("Security")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { isPresented = false }
                }
            }
        }
        .presentationDetents([.medium])
    }
}
