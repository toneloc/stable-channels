import SwiftUI

struct AppearanceSettingsView: View {
    @Binding var themeSelection: String

    private let primaryColor = Color(red: 0/255, green: 163/255, blue: 224/255)

    var body: some View {
        List {
            Section {
                Picker(String(localized: .labelTheme), selection: $themeSelection) {
                    HStack {
                        Image(systemName: "circle.lefthalf.filled")
                        Text(String(localized: .themeSystem))
                    }
                    .tag("system")

                    HStack {
                        Image(systemName: "sun.max.fill")
                        Text(String(localized: .themeLight))
                    }
                    .tag("light")

                    HStack {
                        Image(systemName: "moon.fill")
                        Text(String(localized: .themeDark))
                    }
                    .tag("dark")
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } header: {
                Text(String(localized: .sectionAppearance))
            } footer: {
                Text(String(localized: .infoThemeSetting))
            }
        }
        .navigationTitle(String(localized: .titleAppearance))
        .navigationBarTitleDisplayMode(.inline)
    }
}
