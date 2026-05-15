import SwiftUI

struct ContentView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        ZStack {
            switch appState.phase {
            case .loading:
                LoadingView()
            case .onboarding:
                SyncingView() // Auto-create handles this; should not stay here
            case .syncing:
                SyncingView()
            case .wallet:
                MainTabView()
            case .error(let message):
                ErrorDisplayView(message: message)
            }

            // Tinted overlay — allows background work (price fetch, sync) while waiting for auth
            if !appState.isUnlocked {
                Color.black.opacity(0.7)
                    .ignoresSafeArea()
                    .onAppear {
                        let biometricEnabled = UserDefaults.standard.bool(forKey: "biometricAuthEnabled")
                        if biometricEnabled {
                            Task {
                                let success = await appState.authenticate()
                                if success {
                                    appState.isUnlocked = true
                                }
                            }
                        } else {
                            appState.isUnlocked = true
                        }
                    }
            }
        }
        .onChange(of: scenePhase) { _, newPhase in
            if newPhase == .active && !appState.isUnlocked {
                let biometricEnabled = UserDefaults.standard.bool(forKey: "biometricAuthEnabled")
                if biometricEnabled {
                    Task { await appState.authenticate() }
                } else {
                    appState.isUnlocked = true
                }
            }
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
