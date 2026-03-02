import Foundation

// MARK: - Date Formatting

extension Date {
    var relativeString: String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: self, relativeTo: Date())
    }

    var shortString: String {
        let formatter = DateFormatter()
        formatter.dateStyle = .short
        formatter.timeStyle = .short
        return formatter.string(from: self)
    }
}

// MARK: - Number Formatting

extension UInt64 {
    var satsFormatted: String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        return "\(formatter.string(from: NSNumber(value: self)) ?? "0") sats"
    }

    var btcFormatted: String {
        let btc = Double(self) / Double(Constants.satsInBTC)
        return String(format: "%.8f BTC", btc)
    }

    /// Format as BTC with spaced digit groups: "0.00 009 000"
    var btcSpacedFormatted: String {
        let btc = Double(self) / Double(Constants.satsInBTC)
        let raw = String(format: "%.8f", btc)
        guard let dotIndex = raw.firstIndex(of: ".") else { return raw }
        let whole = raw[raw.startIndex..<dotIndex]
        let decimals = raw[raw.index(after: dotIndex)...]
        // Group decimal digits in threes: "00 009 000"
        var grouped = ""
        for (i, ch) in decimals.enumerated() {
            if i > 0 && i % 3 == 0 { grouped += "\u{2009}" }  // thin space
            grouped += String(ch)
        }
        return "\(whole).\(grouped)"
    }
}

extension Double {
    var usdFormatted: String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .currency
        formatter.currencyCode = "USD"
        formatter.maximumFractionDigits = 2
        return formatter.string(from: NSNumber(value: self)) ?? "$0.00"
    }
}
