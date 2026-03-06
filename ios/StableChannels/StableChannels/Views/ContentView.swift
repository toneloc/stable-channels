import SwiftUI

struct ContentView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
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

