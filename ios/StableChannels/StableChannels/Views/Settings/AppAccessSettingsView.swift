import SwiftUI

struct AppAccessSettingsView: View {
    @AppStorage("biometricAuthEnabled") private var biometricAuthEnabled = true
    @AppStorage("transactionAuthEnabled") private var transactionAuthEnabled = true
    @State private var showAuthForToggle = false
    @State private var authTargetKey: String?

    enum AuthTarget: String {
        case appUnlock = "biometricAuthEnabled"
        case transaction = "transactionAuthEnabled"
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
                            authTargetKey = AuthTarget.appUnlock.rawValue
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
                            authTargetKey = AuthTarget.transaction.rawValue
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
                authTarget: authTargetKey.flatMap { AuthTarget(rawValue: $0) },
                onAuthenticated: { confirmed, target in
                    if confirmed {
                        switch target {
                        case .appUnlock:
                            biometricAuthEnabled = false
                        case .transaction:
                            transactionAuthEnabled = false
                        case .none:
                            break
                        }
                    }
                }
            )
        }
    }
}

struct ToggleAuthSheet: View {
    @Binding var isPresented: Bool
    var authTarget: AppAccessSettingsView.AuthTarget?
    var onAuthenticated: (Bool, AppAccessSettingsView.AuthTarget?) -> Void

    private var titleText: String {
        switch authTarget {
        case .appUnlock:
            return "Authenticate to disable App Unlock"
        case .transaction:
            return "Authenticate to disable Payment Confirmation"
        case .none:
            return "Authenticate"
        }
    }

    private var subtitleText: String {
        switch authTarget {
        case .appUnlock:
            return "Verify your identity to turn off App Unlock"
        case .transaction:
            return "Verify your identity to turn off Payment Confirmation"
        case .none:
            return "Verify your identity to continue"
        }
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
                        do {
                            let success = try await BiometricService.authenticate(
                                reason: titleText
                            )
                            onAuthenticated(success, authTarget)
                        } catch {
                            onAuthenticated(false, nil)
                        }
                        isPresented = false
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
