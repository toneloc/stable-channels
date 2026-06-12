import SwiftUI

struct MainTabView: View {
    @State private var selectedTab = Tab.home

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
    }
}
