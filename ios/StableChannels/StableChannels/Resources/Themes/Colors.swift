import SwiftUI

extension Color {
    static let stablePrimary = Color(red: 0/255, green: 163/255, blue: 224/255)
    static let trendPositive = Color(red: 0.06, green: 0.73, blue: 0.51)
    static let trendNegative = Color(red: 0.94, green: 0.27, blue: 0.27)
    static let sendBlue = Color.blue
    static let deepSendBlue = Color(red: 0.08, green: 0.35, blue: 0.78)
    static let deepSendNavy = Color(red: 0.05, green: 0.22, blue: 0.55)
}

/// Recovery phrase reveal tones; they invert with the system appearance.
extension Color {
    /// Filled call to action: white in dark mode, black in light mode.
    static let revealButtonFill = Color.primary
    static let revealButtonLabel = Color(uiColor: .systemBackground)
    static let revealHairline = Color.primary.opacity(0.1)
    /// Covers the blurred words until tapped.
    static let revealVeil = Color(uiColor: .systemGroupedBackground).opacity(0.55)
}

extension ShapeStyle where Self == Color {
    static var trendPositive: Color { Color.trendPositive }
    static var trendNegative: Color { Color.trendNegative }
}
