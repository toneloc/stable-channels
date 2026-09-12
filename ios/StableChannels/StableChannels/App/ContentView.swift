import SwiftUI
import UIKit

struct ContentView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.scenePhase) private var scenePhase
    @State private var authFailed = false
    @State private var authInProgress = false
    @State private var hasTriggeredAuth = false

    @AppStorage("user_theme") private var themeSelection: String = "system"

    private var colorScheme: ColorScheme? {
        switch themeSelection {
        case "light": return .light
        case "dark": return .dark
        default: return nil
        }
    }

    private var biometricEnabled: Bool {
        UserDefaults.standard.bool(forKey: "biometricAuthEnabled")
    }

    var body: some View {
        ZStack {
            switch appState.phase {
            case .loading, .onboarding, .syncing:
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
        .preferredColorScheme(colorScheme)
        .onChange(of: scenePhase) { _, newPhase in
            // Lock only on .background. .inactive fires during app switcher and Face ID prompts.
            if newPhase == .background {
                appState.lock()
                authFailed = false
                hasTriggeredAuth = false
                authInProgress = false
                return
            }
            if newPhase == .active {
                Task { @MainActor in
                    // 200ms delay lets .inactive from app switcher/Face ID fully settle first
                    try? await Task.sleep(nanoseconds: 200_000_000)
                    if !appState.isUnlocked && biometricEnabled && !authFailed && scenePhase == .active {
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

            Button(String(localized: "try_again", defaultValue: "Try Again")) {
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

        authInProgress = false
        if !success {
            authFailed = true
        } else {
            appState.authError = nil
        }
    }
}

// MARK: - Views

struct SyncingView: View {
    var isSyncComplete: Bool = false
    var onBalanced: (() -> Void)?

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var startTime: Double = 0.0

    private let shimmerDuration: Double = 1.15
    private let crossfadeDuration: Double = 0.40

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60.0)) { timeline in
            let now = timeline.date.timeIntervalSinceReferenceDate
            let effectiveStart = startTime == 0.0 ? now : startTime
            let elapsed = max(0.0, now - effectiveStart)

            let rawProgress = reduceMotion
                ? (elapsed >= shimmerDuration ? 1.0 : 0.0)
                : max(0.0, min(1.0, (elapsed - shimmerDuration) / crossfadeDuration))

            // Smoothstep curve for seamless organic crossfade
            let smoothProgress = rawProgress * rawProgress * (3.0 - 2.0 * rawProgress)

            VStack(spacing: 22) {
                Spacer()

                UnifiedBalanceLaunchView(
                    isSyncComplete: isSyncComplete,
                    size: 115,
                    onBalanced: onBalanced
                )

                ZStack {
                    // Screen 1: Brand title & subtitle during initial shimmer
                    VStack(spacing: 6) {
                        Text(String(localized: "app_name", defaultValue: "Stable Channels"))
                            .font(.title3.weight(.bold))
                            .foregroundStyle(.primary)

                        Text(String(localized: "custody_subtitle", defaultValue: "Self-custodial bitcoin wallet"))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    .opacity(1.0 - smoothProgress)
                    .offset(y: reduceMotion ? 0 : -6.0 * smoothProgress)
                    .allowsHitTesting(smoothProgress < 0.5)

                    // Screen 2: Active syncing status during oscillation
                    VStack(spacing: 6) {
                        Text(String(localized: "status_syncing_wallet", defaultValue: "Wallet Syncing..."))
                            .font(.title3.weight(.bold))
                            .foregroundStyle(.primary)

                        Text(String(localized: "status_syncing_moment", defaultValue: "This may take a moment"))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    .opacity(smoothProgress)
                    .offset(y: reduceMotion ? 0 : 6.0 * (1.0 - smoothProgress))
                    .allowsHitTesting(smoothProgress >= 0.5)
                }
                .frame(minHeight: 52)

                Spacer()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onAppear {
            if startTime == 0.0 {
                startTime = Date().timeIntervalSinceReferenceDate
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

// MARK: - Previews

#Preview("App Launch Flow - Dark") {
    SyncingFlowPreviewContainer()
        .preferredColorScheme(.dark)
}

#Preview("App Launch Flow - Light") {
    SyncingFlowPreviewContainer()
        .preferredColorScheme(.light)
}

private struct SyncingFlowPreviewContainer: View {
    @State private var replayId = UUID()
    @State private var isSyncComplete = false

    var body: some View {
        ZStack {
            SyncingView(isSyncComplete: isSyncComplete)
                .id(replayId)

            VStack {
                Spacer()

                HStack(spacing: 16) {
                    Button {
                        isSyncComplete = false
                        replayId = UUID()
                    } label: {
                        Label(
                            String(localized: "preview_replay", defaultValue: "Replay Flow"),
                            systemImage: "arrow.counterclockwise"
                        )
                    }
                    .buttonStyle(.bordered)

                    Button {
                        isSyncComplete.toggle()
                    } label: {
                        Text(
                            isSyncComplete
                                ? String(localized: "preview_reset_sync", defaultValue: "Reset Sync")
                                : String(localized: "preview_complete_sync", defaultValue: "Complete Sync")
                        )
                    }
                    .buttonStyle(.borderedProminent)
                }
                .padding(.bottom, 32)
            }
        }
    }
}
