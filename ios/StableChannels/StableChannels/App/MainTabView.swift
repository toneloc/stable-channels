import SwiftUI

struct MainTabView: View {
    @State private var selectedTab = Tab.home
    @State private var coordinator = PaymentDetailCoordinator()
    @Environment(AppState.self) private var appState

    enum Tab {
        case home
        case history
        case settings
    }

    var body: some View {
        TabView(selection: $selectedTab) {
            HomeView()
                .tabItem {
                    Label(String(localized: "tab_home", defaultValue: "Home"), systemImage: "house.fill")
                }
                .tag(Tab.home)

            HistoryView()
                .tabItem {
                    Label(String(localized: "tab_history", defaultValue: "History"), systemImage: "clock.fill")
                }
                .tag(Tab.history)

            SettingsView()
                .tabItem {
                    Label(String(localized: "tab_settings", defaultValue: "Settings"), systemImage: "gearshape.fill")
                }
                .tag(Tab.settings)
        }
        .environment(coordinator)
        .onChange(of: coordinator.paymentId) { _, newValue in
            if newValue != nil, selectedTab != .history {
                selectedTab = .history
            }
        }
        .sheet(item: Binding(
            get: { coordinator.paymentId.map(PaymentIdentifier.init) },
            set: { coordinator.paymentId = $0?.id }
        )) { id in
            PaymentDetailView(
                paymentId: id.id,
                displayPrice: historyDisplayPrice
            )
        }
    }

    private var historyDisplayPrice: Double {
        appState.btcPrice > 0 ? appState.btcPrice : appState.stableChannel.latestPrice
    }
}

private struct PaymentIdentifier: Identifiable, Equatable {
    let id: Int64
}
