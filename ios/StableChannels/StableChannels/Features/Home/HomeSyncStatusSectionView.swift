import SwiftUI

struct HomeSyncSpinnerView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        if appState.isSyncing {
            HStack(spacing: 6) {
                ProgressView()
                    .controlSize(.small)
                Text(String(localized: "home_syncing", defaultValue: "Syncing..."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

struct HomeSyncStatusSectionView: View {
    @Environment(AppState.self) private var appState
    let onOpenPaymentDetail: () -> Void

    var body: some View {
        if !appState.statusMessage.isEmpty {
            Button(action: onOpenPaymentDetail) {
                Text(appState.statusMessage)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.ultraThinMaterial, in: Capsule())
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            .buttonStyle(.plain)
        }
    }
}
