import SwiftUI

struct AppearanceSettingsView: View {
    @Binding var themeSelection: String

    private let primaryColor = Color(red: 0/255, green: 163/255, blue: 224/255)

    var body: some View {
        List {
            Section {
                Picker(String(localized: "label_theme", defaultValue: "Theme"), selection: $themeSelection) {
                    HStack {
                        Image(systemName: "circle.lefthalf.filled")
                        Text(String(localized: "theme_system", defaultValue: "System"))
                    }
                    .tag("system")

                    HStack {
                        Image(systemName: "sun.max.fill")
                        Text(String(localized: "theme_light", defaultValue: "Light"))
                    }
                    .tag("light")

                    HStack {
                        Image(systemName: "moon.fill")
                        Text(String(localized: "theme_dark", defaultValue: "Dark"))
                    }
                    .tag("dark")
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } header: {
                Text(String(localized: "section_appearance", defaultValue: "Appearance"))
            } footer: {
                Text(String(
                    localized: "info_theme_setting",
                    defaultValue: "System uses your device's appearance setting."
                ))
            }
        }
        .navigationTitle(String(localized: "title_appearance", defaultValue: "Appearance"))
        .navigationBarTitleDisplayMode(.inline)
    }
}
