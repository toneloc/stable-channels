import SwiftUI
import UIKit

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
                        // Auth triggered on .active via onChange — skip here to avoid racing
                        if !hasTriggeredAuth {
                            hasTriggeredAuth = true
                            if scenePhase == .active {
                                Task { await runAuth() }
                            }
                        }
                    }
            }
        }
        .onChange(of: scenePhase) { _, newPhase in
            // Lock only on .background. .inactive fires during app switcher and Face ID prompts.
            if newPhase == .background {
                appState.isUnlocked = false
                authFailed = false
                hasTriggeredAuth = false
                authInProgress = false
                return
            }
            if newPhase == .active {
                Task { @MainActor in
                    // 200ms delay lets .inactive from app switcher/Face ID fully settle first
                    try? await Task.sleep(nanoseconds: 200_000_000)
                    if !appState.isUnlocked && biometricEnabled && scenePhase == .active {
                        await runAuth()
                    }
                }
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
        .modifier(PrivacyOverlayModifier())
    }

    private var waitingView: some View {
        VStack(spacing: 20) {
            Image(systemName: "faceid")
                .font(.system(size: 60))
                .foregroundStyle(.green.opacity(0.5))

            Text(String(localized: "label_authenticate", defaultValue: "Authenticate"))
                .font(.headline)
                .foregroundStyle(.white)
        }
    }

    private var failedView: some View {
        VStack(spacing: 20) {
            Image(systemName: "faceid")
                .font(.system(size: 60))
                .foregroundStyle(.green)

            Text(String(localized: "label_auth_failed", defaultValue: "Authentication Failed"))
                .font(.headline)
                .foregroundStyle(.white)

            if let error = appState.authError {
                Text(error)
                    .font(.subheadline)
                    .foregroundStyle(.white.opacity(0.7))
                    .multilineTextAlignment(.center)
                    .padding(.horizontal)
            }

            Button(String(localized: "button_try_again", defaultValue: "Try Again")) {
                Task { await runAuth() }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

            Button(String(localized: "button_cancel", defaultValue: "Cancel")) { }
                .foregroundStyle(.white.opacity(0.6))
        }
    }

    private func runAuth() async {
        guard !appState.isUnlocked else { return }
        guard !authInProgress else { return }

        // Dismiss any active keyboard to avoid blocking system auth dialogs
        UIApplication.shared.sendAction(
            Selector(("resignFirstResponder")),
            to: nil,
            from: nil,
            for: nil
        )

        authInProgress = true
        authFailed = false

        let success = await appState.authenticate()

        if success {
            appState.isUnlocked = true
        }

        authInProgress = false
        if !success {
            authFailed = true
        } else {
            appState.authError = nil
        }
    }
}

// MARK: - Views

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
                Text(String(localized: "app_name", defaultValue: "Stable Channels"))
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(.primary)
                Text(String(localized: "app_subtitle", defaultValue: "Self-custodial bitcoin trading"))
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
                Text(String(localized: "status_syncing_wallet", defaultValue: "Syncing wallet..."))
                    .foregroundStyle(.secondary)
                Text(String(localized: "status_syncing_moment", defaultValue: "This may take a moment"))
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
            Text(String(localized: "error_title", defaultValue: "Error"))
                .font(.title2.bold())
            Text(message)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal)

            Button(String(localized: "try_again", defaultValue: "Try Again")) {
                appState.phase = .loading
                Task { await appState.start() }
            }
            .buttonStyle(.bordered)
            .padding(.top, 8)
        }
    }
}

// MARK: - Privacy Overlay

struct PrivacyOverlayModifier: ViewModifier {
    @Environment(\.scenePhase) private var scenePhase

    func body(content: Content) -> some View {
        content
            .overlay {
                if scenePhase == .background {
                    Color.black
                        .ignoresSafeArea()
                        .zIndex(999)
                }
            }
    }
}
