import SwiftUI

/// Latest payments on Home, with a "View all" link to the History tab.
struct RecentActivityView: View {
    @Environment(AppState.self) private var appState
    @Environment(PaymentDetailCoordinator.self) private var paymentCoordinator

    /// Changes whenever a sheet that may record a payment closes, so the list reloads.
    let reloadToken: Bool
    let maxRows: Int
    let onSelect: (PaymentRecord) -> Void

    @State private var payments: [PaymentRecord] = []

    private static let loadCount = 4
    private static let fourRowsMinSpace: CGFloat = 252
    private static let threeRowsMinSpace: CGFloat = 200
    private static let twoRowsMinSpace: CGFloat = 148

    /// One row when the card sits near the bottom of the screen, up to four when there is room.
    static func rowCount(spaceBelow: CGFloat) -> Int {
        if spaceBelow >= fourRowsMinSpace { return loadCount }
        if spaceBelow >= threeRowsMinSpace { return 3 }
        return spaceBelow >= twoRowsMinSpace ? 2 : 1
    }

    private var shown: [PaymentRecord] {
        Array(payments.prefix(maxRows))
    }

    private var displayPrice: Double {
        appState.btcPrice > 0 ? appState.btcPrice : appState.stableChannel.latestPrice
    }

    var body: some View {
        // A Group with no children would never fire onAppear, so the list would never load.
        VStack(spacing: 0) {
            if !shown.isEmpty {
                VStack(spacing: 0) {
                    HStack {
                        Text(String(localized: "home_recent_activity", defaultValue: "Recent activity"))
                            .font(.caption.bold())
                        Spacer()
                        Button {
                            paymentCoordinator.showPaymentsRequested = true
                        } label: {
                            Text(String(localized: "home_view_all", defaultValue: "View all"))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.horizontal, 4)
                    .padding(.bottom, 2)

                    ForEach(shown, id: \.id) { payment in
                        Button { onSelect(payment) } label: {
                            PaymentRowView(payment: payment, displayPrice: displayPrice, compact: true)
                                .padding(.horizontal, 4)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                    .padding(.bottom, 4)
                }
            }
        }
        .onAppear { load() }
        .onChange(of: appState.confirmationUpdateEpoch) { _, _ in load() }
        .onChange(of: appState.paymentFlash) { _, isFlashing in
            if isFlashing { load() }
        }
        .onChange(of: appState.lightningBalanceSats) { _, _ in load() }
        .onChange(of: appState.onchainBalanceSats) { _, _ in load() }
        .onChange(of: reloadToken) { _, _ in load() }
    }

    private func load() {
        payments = (try? appState.databaseService?.paymentRepo.getRecentPayments(limit: Self.loadCount)) ?? []
    }
}
