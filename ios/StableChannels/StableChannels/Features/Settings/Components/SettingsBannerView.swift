import SwiftUI

/// A reusable settings banner — icon badge, title, body text, rounded-rect background with colored border.
/// Used for custody disclaimer, backup warnings, and support banners.
struct SettingsBanner: View {
    let icon: String
    let iconColor: Color
    let title: String
    let bodyText: String
    let borderColor: Color
    let maxWidth: Bool

    init(
        icon: String,
        iconColor: Color,
        title: String,
        bodyText: String,
        borderColor: Color,
        maxWidth: Bool = true
    ) {
        self.icon = icon
        self.iconColor = iconColor
        self.title = title
        self.bodyText = bodyText
        self.borderColor = borderColor
        self.maxWidth = maxWidth
    }

    var body: some View {
        HStack(spacing: 14) {
            SettingsIconBadge(systemImage: icon, color: iconColor)

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.subheadline)
                    .fontWeight(.bold)
                    .foregroundStyle(.primary)

                Text(bodyText)
                    .font(.caption)
                    .foregroundStyle(Color(uiColor: .label).opacity(0.7))
            }
            Spacer()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(maxWidth: maxWidth ? .infinity : nil, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(.ultraThinMaterial)
                .overlay(
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .strokeBorder(borderColor, lineWidth: 1)
                )
        )
    }
}
