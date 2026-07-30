import SwiftUI

struct AboutSettingsView: View {
    var body: some View {
        List {
            Section {
                HStack {
                    Text(String(localized: "label_version", defaultValue: "Version"))
                    Spacer()
                    Text(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—")
                        .foregroundStyle(.secondary)
                }
                HStack {
                    Text(String(localized: "label_build", defaultValue: "Build"))
                    Spacer()
                    Text(Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "—")
                        .foregroundStyle(.secondary)
                }
            } header: {
                Text(String(localized: "section_app_info", defaultValue: "App Info"))
            }

            Section {
                HStack {
                    Text(String(localized: "label_network", defaultValue: "Network"))
                    Spacer()
                    Text(Constants.defaultNetwork)
                        .foregroundStyle(.secondary)
                }
            } header: {
                Text(String(localized: "section_network", defaultValue: "Network"))
            }

            Section {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(String(localized: "label_custody", defaultValue: "Custody"))
                            Spacer()
                            Text(String(localized: "custody_self", defaultValue: "Self-custodial"))
                                .foregroundStyle(.green)
                                .fontWeight(.medium)
                        }
                    }
                }

                Text(String(
                    localized: "info_self_custody",
                    defaultValue: "Stable Channels is a self-custodial wallet. You control your private keys. Third parties do not custody, access, or freeze your funds."
                ))
                .font(.caption)
                .foregroundStyle(.secondary)
            } header: {
                Text(String(localized: "section_custody", defaultValue: "Custody Model"))
            }
        }
        .navigationTitle(String(localized: "title_about", defaultValue: "About"))
        .navigationBarTitleDisplayMode(.inline)
    }
}
