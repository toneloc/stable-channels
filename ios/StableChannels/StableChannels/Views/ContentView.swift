import SwiftUI

struct ContentView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.scenePhase) private var scenePhase
    @State private var authFailed = false
    @State private var authInProgress = false
    @State private var hasTriggeredAuth = false

    private var biometricEnabled: Bool {
        UserDefaults.standard.bool(forKey: "biometricAuthEnabled")
    }

    var body: some View {
        ZStack {
            switch appState.phase {
            case .loading:
                LoadingView()
            case .onboarding:
                SyncingView()
            case .syncing:
                SyncingView()
            case .wallet:
                MainTabView()
            case .error(let message):
                ErrorDisplayView(message: message)
            }

            // Auth overlay: shown only when locked and biometric is enabled
            if !appState.isUnlocked && biometricEnabled {
                Color.black
                    .ignoresSafeArea()
                    .overlay {
                        if authFailed {
                            failedView
                        } else {
                            waitingView
                        }
                    }
                    .onAppear {
                        // Trigger auth once when overlay first appears
                        if !hasTriggeredAuth {
                            hasTriggeredAuth = true
                            Task { await runAuth() }
                        }
                    }
            }
        }
        .onChange(of: scenePhase) { _, newPhase in
            // Re-auth when returning to foreground, unless already failed or in progress
            if newPhase == .active && !appState.isUnlocked && biometricEnabled && !authInProgress && !authFailed {
                Task { await runAuth() }
            }
        }
        .onChange(of: appState.isUnlocked) { _, unlocked in
            if unlocked {
                authInProgress = false
            }
        }
        .onChange(of: biometricEnabled) { _, enabled in
            if !enabled {
                authInProgress = false
                authFailed = false
                hasTriggeredAuth = false
            }
        }
    }

    private var waitingView: some View {
        VStack(spacing: 20) {
            Image(systemName: "faceid")
                .font(.system(size: 60))
                .foregroundStyle(.green.opacity(0.5))

            Text("Authenticate")
                .font(.headline)
                .foregroundStyle(.white)
        }
    }

    private var failedView: some View {
        VStack(spacing: 20) {
            Image(systemName: "faceid")
                .font(.system(size: 60))
                .foregroundStyle(.green)

            Text("Authentication Failed")
                .font(.headline)
                .foregroundStyle(.white)

            Button("Try Again") {
                Task { await runAuth() }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

            Button("Cancel") { }
                .foregroundStyle(.white.opacity(0.6))
        }
    }

    private func runAuth() async {
        guard !authInProgress else { return }
        guard !appState.isUnlocked else { return }

        authInProgress = true
        authFailed = false

        let success = await appState.authenticate()

        if success {
            appState.isUnlocked = true
        }

        authInProgress = false
        if !success {
            authFailed = true
        }
    }
}

// MARK: - Loading / Syncing / Error

struct LoadingView: View {
    @State private var pulse = false

    var body: some View {
        VStack(spacing: 24) {
            Image("SplashIcon")
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(width: 90, height: 90)
                .clipShape(Circle())
                .scaleEffect(pulse ? 1.06 : 0.94)
                .animation(.easeInOut(duration: 1.2).repeatForever(autoreverses: true), value: pulse)
                .onAppear { pulse = true }

            VStack(spacing: 4) {
                Text("Stable Channels")
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(.primary)
                Text("Self-custodial bitcoin trading")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

struct SyncingView: View {
    @State private var pulse = false

    var body: some View {
        VStack(spacing: 24) {
            Image("SplashIcon")
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(width: 90, height: 90)
                .clipShape(Circle())
                .scaleEffect(pulse ? 1.06 : 0.94)
                .animation(.easeInOut(duration: 1.2).repeatForever(autoreverses: true), value: pulse)
                .onAppear { pulse = true }

            VStack(spacing: 12) {
                ProgressView()
                Text("Syncing wallet...")
                    .foregroundStyle(.secondary)
                Text("This may take a moment")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
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
