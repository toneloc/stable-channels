import SwiftUI

struct AboutSettingsView: View {
    var body: some View {
        List {
            Section {
                HStack {
                    Text(String(localized: .labelVersion))
                    Spacer()
                    Text(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—")
                        .foregroundStyle(.secondary)
                }
                HStack {
                    Text(String(localized: .labelBuild))
                    Spacer()
                    Text(Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "—")
                        .foregroundStyle(.secondary)
                }
            } header: {
                Text(String(localized: .sectionAppInfo))
            }

            Section {
                HStack {
                    Text(String(localized: .labelNetwork))
                    Spacer()
                    Text(Constants.defaultNetwork)
                        .foregroundStyle(.secondary)
                }
            } header: {
                Text(String(localized: .sectionNetwork))
            }

            Section {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(String(localized: .labelCustody))
                            Spacer()
                            Text(String(localized: .custodySelf))
                                .foregroundStyle(.green)
                                .fontWeight(.medium)
                        }
                    }
                }

                Text(String(localized: .infoSelfCustody))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } header: {
                Text(String(localized: .sectionCustody))
            }
        }
        .navigationTitle(String(localized: .titleAbout))
        .navigationBarTitleDisplayMode(.inline)
    }
}
