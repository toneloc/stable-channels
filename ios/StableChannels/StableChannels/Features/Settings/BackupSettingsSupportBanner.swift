import SwiftUI

struct BackupSettingsSupportBanner: View {
    var body: some View {
        HStack(spacing: 14) {
            supportIconBadge
            VStack(alignment: .leading, spacing: 2) {
                Text(String(localized: "recovery_banner_title", defaultValue: "Need Help?"))
                    .font(.subheadline)
                    .fontWeight(.bold)
                    .foregroundStyle(.primary)

                VStack(alignment: .leading, spacing: 2) {
                    Text(String(
                        localized: "recovery_banner_text_prefix",
                        defaultValue: "Please email"
                    ))
                        + Text(" ")
                        + Text("support@stablechannels.com")
                        .foregroundStyle(.blue)
                        .underline()
                        + Text(" ")
                        + Text(String(
                            localized: "recovery_banner_text_suffix",
                            defaultValue: "for wallet recovery assistance."
                        ))
                }
                .font(.caption)
                .foregroundStyle(Color(uiColor: .label).opacity(0.7))
            }
            Spacer()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(.ultraThinMaterial)
                .overlay(
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .strokeBorder(.orange, lineWidth: 1)
                )
        )
    }

    private var supportIconBadge: some View {
        ZStack {
            Circle()
                .fill(.orange)
                .frame(width: 44, height: 44)
                .shadow(color: .orange.opacity(0.3), radius: 8, x: 0, y: 4)
            Image(systemName: "questionmark.circle.fill")
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(.white)
        }
    }
}
