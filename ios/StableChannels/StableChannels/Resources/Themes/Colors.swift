import SwiftUI

extension Color {
    static let stablePrimary = Color(red: 0/255, green: 163/255, blue: 224/255)
    static let trendPositive = Color(red: 0.06, green: 0.73, blue: 0.51)
    static let trendNegative = Color(red: 0.94, green: 0.27, blue: 0.27)
}

extension ShapeStyle where Self == Color {
    static var trendPositive: Color { Color.trendPositive }
    static var trendNegative: Color { Color.trendNegative }
}
