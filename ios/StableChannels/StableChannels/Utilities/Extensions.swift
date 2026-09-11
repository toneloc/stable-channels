import Foundation
import UIKit

// MARK: - Thread-Safe Formatter Cache

enum AppFormatters {
    private static let relativeDate: RelativeDateTimeFormatter = {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter
    }()

    private static let shortDate: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .short
        formatter.timeStyle = .short
        return formatter
    }()

    private static let sats: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.locale = Locale(identifier: "en_US")
        formatter.numberStyle = .decimal
        return formatter
    }()

    private static let usd: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.locale = Locale(identifier: "en_US")
        formatter.numberStyle = .currency
        formatter.currencyCode = "USD"
        formatter.maximumFractionDigits = 2
        return formatter
    }()

    static func formatRelativeDate(_ date: Date, relativeTo: Date = Date()) -> String {
        relativeDate.localizedString(for: date, relativeTo: relativeTo)
    }

    static func formatShortDate(_ date: Date) -> String {
        shortDate.string(from: date)
    }

    static func formatSats(_ satsValue: UInt64) -> String {
        sats.string(from: NSNumber(value: satsValue)) ?? "0"
    }

    static func formatUSD(_ amount: Double) -> String {
        usd.string(from: NSNumber(value: amount)) ?? "$0.00"
    }
}

// MARK: - Date Formatting

extension Date {
    var relativeString: String {
        AppFormatters.formatRelativeDate(self)
    }

    var shortString: String {
        AppFormatters.formatShortDate(self)
    }
}

// MARK: - Number Formatting

extension UInt64 {
    var satsFormatted: String {
        let formatted = AppFormatters.formatSats(self)
        return "\(formatted) sats"
    }

    var btcFormatted: String {
        let btc = Double(self) / Double(Constants.satsInBTC)
        return String(format: "%.8f BTC", btc)
    }

    /// Format as BTC with spaced digit groups: "0.00 190 079"
    /// Two decimal digits, then groups of three separated by thin spaces.
    /// Preallocates buffer capacity to minimize intermediate heap reallocations.
    var btcSpacedFormatted: String {
        let btc = Double(self) / Double(Constants.satsInBTC)
        let raw = String(format: "%.8f", btc)
        guard let dotIndex = raw.firstIndex(of: ".") else { return raw }
        let whole = raw[raw.startIndex..<dotIndex]
        let decimals = raw[raw.index(after: dotIndex)...]
        guard decimals.count >= 8 else { return "\(whole).\(decimals)" }

        let d0 = decimals.startIndex
        let d2 = decimals.index(d0, offsetBy: 2)
        let d5 = decimals.index(d0, offsetBy: 5)
        let d8 = decimals.index(d0, offsetBy: 8)

        var result = String()
        result.reserveCapacity(raw.count + 4)
        result.append(contentsOf: whole)
        result.append(".")
        result.append(contentsOf: decimals[d0..<d2])
        result.append("\u{2009}")
        result.append(contentsOf: decimals[d2..<d5])
        result.append("\u{2009}")
        result.append(contentsOf: decimals[d5..<d8])
        return result
    }
}

extension Double {
    var usdFormatted: String {
        AppFormatters.formatUSD(self)
    }
}

extension UIFont {
    static func rounded(ofSize size: CGFloat, weight: UIFont.Weight) -> UIFont {
        let base = UIFont.systemFont(ofSize: size, weight: weight)
        if let descriptor = base.fontDescriptor.withDesign(.rounded) {
            return UIFont(descriptor: descriptor, size: size)
        }
        return base
    }
}
