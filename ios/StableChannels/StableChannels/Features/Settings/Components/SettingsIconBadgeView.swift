import SwiftUI
import os.log

/// A circular icon badge used in settings banners and cards.
/// - Parameters:
/// - systemImage: SF Symbol name
/// - color: Fill and shadow color
/// - size: Diameter of the circle (default 44)
/// - iconSize: Font size of the symbol (default 20)
struct SettingsIconBadge: View {
    let systemImage: String
    let color: Color
    let size: CGFloat
    let iconSize: CGFloat

    init(
        systemImage: String,
        color: Color,
        size: CGFloat = 44,
        iconSize: CGFloat = 20
    ) {
        self.systemImage = systemImage
        self.color = color
        self.size = size
        self.iconSize = iconSize
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(color)
                .frame(width: size, height: size)
                .shadow(color: color.opacity(0.3), radius: 8, x: 0, y: 4)
            Image(systemName: systemImage)
                .font(.system(size: iconSize, weight: .semibold))
                .foregroundStyle(.white)
        }
    }
}
