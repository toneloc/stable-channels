import SwiftUI

struct ContentView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        switch appState.phase {
        case .loading:
            LoadingView()
        case .onboarding:
            OnboardingView()
        case .syncing:
            SyncingView()
        case .wallet:
            MainTabView()
        case .error(let message):
            ErrorDisplayView(message: message)
        }
    }
}

// MARK: - Loading / Syncing / Error

struct LoadingView: View {
    var body: some View {
        VStack(spacing: 16) {
            ProgressView()
            Text("Starting...")
                .foregroundStyle(.secondary)
        }
    }
}

struct SyncingView: View {
    var body: some View {
        VStack(spacing: 16) {
            ProgressView()
            Text("Syncing wallet...")
                .foregroundStyle(.secondary)
            Text("This may take a moment")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
    }
}

struct ErrorDisplayView: View {
    let message: String
    @Environment(AppState.self) private var appState

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "exclamationmark.triangle")
                .font(.largeTitle)
                .foregroundStyle(.red)
            Text("Error")
                .font(.title2.bold())
            Text(message)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal)

            Button("Try Again") {
                appState.phase = .loading
                Task { await appState.start() }
            }
            .buttonStyle(.bordered)
            .padding(.top, 8)
        }
    }
}

// MARK: - Onboarding

struct OnboardingView: View {
    @Environment(AppState.self) private var appState
    @State private var showRestore = false
    @State private var mnemonic = ""
    @State private var isCreating = false
    @State private var errorMessage: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 32) {
                Spacer()

                Text("Stable Channels")
                    .font(.largeTitle.bold())
                Text("Wallet v1.0")
                    .font(.title3)
                    .foregroundStyle(.secondary)

                Spacer()

                if showRestore {
                    restoreSection
                } else {
                    buttonSection
                }

                if let error = errorMessage {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .padding(.horizontal)
                }

                Spacer()
            }
        }
    }

    private var buttonSection: some View {
        VStack(spacing: 16) {
            Button {
                Task { await createWallet(restore: false) }
            } label: {
                if isCreating {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                } else {
                    Text("Create New Wallet")
                        .frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(isCreating)

            Button {
                showRestore = true
            } label: {
                Text("Restore from Seed")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
            .controlSize(.large)
        }
        .padding(.horizontal, 32)
    }

    private var restoreSection: some View {
        VStack(spacing: 16) {
            Text("Enter your 12-word seed phrase")
                .font(.headline)

            TextField("word1 word2 word3 ...", text: $mnemonic, axis: .vertical)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(3...5)
                .font(.system(.body, design: .monospaced))
                .padding()
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))

            HStack(spacing: 12) {
                Button("Back") {
                    showRestore = false
                    mnemonic = ""
                    errorMessage = nil
                }
                .buttonStyle(.bordered)
                .controlSize(.large)

                Button {
                    Task { await createWallet(restore: true) }
                } label: {
                    if isCreating {
                        ProgressView()
                            .frame(maxWidth: .infinity)
                    } else {
                        Text("Restore")
                            .frame(maxWidth: .infinity)
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(isCreating || mnemonic.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(.horizontal, 32)
    }

    private func createWallet(restore: Bool) async {
        isCreating = true
        errorMessage = nil
        defer { isCreating = false }

        let mnemonicInput = restore ? mnemonic.trimmingCharacters(in: .whitespacesAndNewlines) : ""
        if restore {
            let wordCount = mnemonicInput.split(separator: " ").count
            guard wordCount == 12 || wordCount == 24 else {
                errorMessage = "Seed phrase must be 12 or 24 words"
                return
            }
        }

        do {
            try await appState.nodeService.start(
                network: .bitcoin,
                esploraURL: Constants.defaultChainURL,
                mnemonic: mnemonicInput
            )
            await MainActor.run {
                appState.phase = .wallet
                appState.refreshBalances()
            }
        } catch {
            await MainActor.run {
                errorMessage = error.localizedDescription
            }
        }
    }
}
