import Foundation

/// Currency unit options for amount entry in the Send workflow.
enum SendAmountUnit: String, CaseIterable, Identifiable, Sendable {
    case usd = "USD"
    case sats = "Sats"
    case btc = "BTC"

    var id: String { rawValue }

    var title: String { rawValue }

    var menuTitle: String {
        switch self {
        case .usd: return "US Dollar (USD)"
        case .sats: return "Satoshis (sats)"
        case .btc: return "Bitcoin (BTC)"
        }
    }

    var symbolOrSuffix: String {
        switch self {
        case .usd: return "$"
        case .sats: return "sats"
        case .btc: return "BTC"
        }
    }

    var placeholder: String {
        switch self {
        case .usd: return "0.00"
        case .sats: return "0"
        case .btc: return "0.0"
        }
    }

    var maxDecimals: Int {
        switch self {
        case .usd: return 2
        case .sats: return 0
        case .btc: return 8
        }
    }

    func secondaryConversionText(sats: UInt64, btcPrice: Double) -> String {
        let usd = (Double(sats) / Double(Constants.satsInBTC)) * btcPrice
        switch self {
        case .usd:
            return "≈ \(sats.btcSpacedFormatted) BTC (\(sats) sats)"
        case .sats:
            return "≈ \(usd.usdFormatted) USD (\(sats.btcSpacedFormatted) BTC)"
        case .btc:
            return "≈ \(usd.usdFormatted) USD (\(sats) sats)"
        }
    }

    func allowedRangeText(params: LNURLPayParams, btcPrice: Double) -> String {
        switch self {
        case .usd:
            let minUSD = (Double(params.minSats) / Double(Constants.satsInBTC)) * btcPrice
            let maxUSD = (Double(params.maxSats) / Double(Constants.satsInBTC)) * btcPrice
            return "Allowed: \(minUSD.usdFormatted) – \(maxUSD.usdFormatted) (\(params.minSats)–\(params.maxSats) sats)"
        case .sats:
            return "Allowed range: \(params.minSats) – \(params.maxSats) sats"
        case .btc:
            return "Allowed: \(params.minSats.btcFormatted) – \(params.maxSats.btcFormatted)"
        }
    }
}

/// Pure domain calculations for amount inputs, unit conversions, and balance percentages.
/// Zero UI framework dependencies (Functional Core).
///
/// All text <-> sats conversions go through `Decimal`, never `Double`: a binary double cannot
/// represent most decimal inputs exactly ("0.0003" * 1e8 is 29999.999999999996), so flooring a
/// double product silently drops a sat on ordinary amounts. Every conversion here rounds *down*
/// to the unit's precision, so a displayed or confirmed amount is never larger than what the
/// user typed and never larger than the sats actually sent.
enum SendAmountCalculator {
    private static let satsPerBTC = Decimal(Constants.satsInBTC)
    /// 21,000,000 BTC in sats — anything above this is not a real amount.
    private static let maxSats = Decimal(2_100_000_000_000_000)
    private static let posixLocale = Locale(identifier: "en_US_POSIX")

    /// Parses user text as a non-negative decimal. Rejects signs, exponents and anything that is
    /// not digits with at most one decimal point.
    static func parseDecimal(_ text: String) -> Decimal? {
        var trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        guard trimmed.allSatisfy({ ($0.isASCII && $0.isNumber) || $0 == "." }) else { return nil }
        guard trimmed.filter({ $0 == "." }).count <= 1 else { return nil }
        if trimmed.hasPrefix(".") { trimmed = "0" + trimmed }
        if trimmed.hasSuffix(".") { trimmed.removeLast() }
        guard !trimmed.isEmpty, let value = Decimal(string: trimmed, locale: posixLocale), value.isFinite,
              value >= 0 else {
            return nil
        }
        return value
    }

    /// Rounds `value` down (toward zero; inputs are non-negative) to `scale` decimal places.
    static func roundedDown(_ value: Decimal, scale: Int) -> Decimal {
        var input = value
        var result = Decimal()
        NSDecimalRound(&result, &input, scale, .down)
        return result
    }

    /// Converts a whole-sat `Decimal` to `UInt64`, or nil when it is not a sane sat amount.
    private static func satsValue(_ whole: Decimal) -> UInt64? {
        guard whole >= 1, whole <= maxSats else { return nil }
        let number = NSDecimalNumber(decimal: whole)
        let sats = number.uint64Value
        guard Decimal(sats) == whole else { return nil }
        return sats
    }

    /// Sats represented by a BTC amount, floored to a whole sat.
    static func sats(fromBTC btc: Decimal) -> UInt64? {
        satsValue(roundedDown(btc * satsPerBTC, scale: 0))
    }

    /// Sats represented by a USD amount at `btcPrice`, floored to a whole sat.
    static func sats(fromUSD usd: Decimal, btcPrice: Double) -> UInt64? {
        guard btcPrice.isFinite, btcPrice > 0 else { return nil }
        let price = Decimal(btcPrice)
        guard price > 0 else { return nil }
        return satsValue(roundedDown(usd / price * satsPerBTC, scale: 0))
    }

    /// USD cents represented by `sats` at `btcPrice`, floored to a whole cent.
    static func cents(fromSats sats: UInt64, btcPrice: Double) -> UInt64? {
        guard btcPrice.isFinite, btcPrice > 0 else { return nil }
        let usd = Decimal(sats) * Decimal(btcPrice) / satsPerBTC
        let cents = roundedDown(usd * 100, scale: 0)
        guard cents >= 0, cents <= maxSats else { return nil }
        return NSDecimalNumber(decimal: cents).uint64Value
    }

    /// Exact fixed-point BTC string for `sats` ("0.00030000"), optionally with trailing zeros
    /// (and a bare trailing point) removed ("0.0003").
    static func btcString(sats: UInt64, trimTrailingZeros: Bool) -> String {
        let whole = sats / Constants.satsInBTC
        let fraction = sats % Constants.satsInBTC
        var text = "\(whole)." + String(format: "%08llu", fraction)
        if trimTrailingZeros {
            while text.hasSuffix("0") {
                text.removeLast()
            }
            if text.hasSuffix(".") { text.removeLast() }
        }
        return text
    }

    /// Exact fixed-point USD string for a cent amount ("12.34").
    static func usdString(cents: UInt64) -> String {
        "\(cents / 100)." + String(format: "%02llu", cents % 100)
    }

    /// Normalizes user text input based on the active currency unit. Excess precision is
    /// truncated, never rounded up, so normalization can only lower an amount.
    static func normalizeInput(_ text: String, unit: SendAmountUnit) -> String {
        guard !text.isEmpty else { return "" }
        guard let value = parseDecimal(text) else { return "" }
        switch unit {
        case .usd:
            let cents = roundedDown(value * 100, scale: 0)
            guard cents <= maxSats else { return "" }
            return usdString(cents: NSDecimalNumber(decimal: cents).uint64Value)
        case .sats:
            let whole = roundedDown(value, scale: 0)
            guard whole <= maxSats else { return "" }
            return "\(NSDecimalNumber(decimal: whole).uint64Value)"
        case .btc:
            let whole = roundedDown(value * satsPerBTC, scale: 0)
            guard whole <= maxSats else { return "" }
            return btcString(sats: NSDecimalNumber(decimal: whole).uint64Value, trimTrailingZeros: true)
        }
    }

    /// Convenience wrapper for normalizing amount input.
    static func normalizeAmountInput(text: String, unit: SendAmountUnit) -> String {
        normalizeInput(text, unit: unit)
    }

    /// Computes effective satoshi amount from user input or invoice preset amount.
    static func computeEffectiveSats(
        destination: SendDestination? = nil,
        inputText: String,
        unit: SendAmountUnit,
        btcPrice: Double
    ) -> UInt64 {
        if let dest = destination {
            switch dest {
            case .bolt11(_, _, let msat):
                if let msat, msat > 0 {
                    return msat / 1000
                }
            case .onchain(_, let amountSats):
                let trimmed = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
                if trimmed.isEmpty, let amountSats, amountSats > 0 {
                    return amountSats
                }
            default:
                break
            }
        }
        guard let value = parseDecimal(inputText), value > 0 else { return 0 }
        switch unit {
        case .sats:
            return satsValue(roundedDown(value, scale: 0)) ?? 0
        case .usd:
            return sats(fromUSD: value, btcPrice: btcPrice) ?? 0
        case .btc:
            return sats(fromBTC: value) ?? 0
        }
    }

    /// Convenience wrapper for computing effective satoshis from text without destination.
    static func computeEffectiveSats(
        text: String,
        unit: SendAmountUnit,
        btcPrice: Double
    ) -> UInt64 {
        computeEffectiveSats(destination: nil, inputText: text, unit: unit, btcPrice: btcPrice)
    }

    /// Formats satoshis into the requested currency unit, truncated to the unit's precision.
    /// BTC output is exact, so `formatSatsForUnit` and `computeEffectiveSats` round-trip
    /// without drift.
    static func formatSatsForUnit(_ sats: UInt64, unit: SendAmountUnit, btcPrice: Double) -> String {
        guard sats > 0 else { return "" }
        switch unit {
        case .usd:
            guard let cents = cents(fromSats: sats, btcPrice: btcPrice) else { return "" }
            return usdString(cents: cents)
        case .sats:
            return "\(sats)"
        case .btc:
            return btcString(sats: sats, trimTrailingZeros: false)
        }
    }

    /// Converts an amount between units preserving the underlying value.
    static func switchUnit(
        from: SendAmountUnit,
        to: SendAmountUnit,
        text: String,
        btcPrice: Double
    ) -> String {
        let sats = computeEffectiveSats(destination: nil, inputText: text, unit: from, btcPrice: btcPrice)
        guard sats > 0 else { return "" }
        return formatSatsForUnit(sats, unit: to, btcPrice: btcPrice)
    }

    /// Calculates percentage-based send amount (e.g. 25%, 50%, 100%).
    static func calculatePercentageAmount(
        percent: Int,
        totalBalanceSats: UInt64,
        unit: SendAmountUnit,
        btcPrice: Double
    ) -> String {
        guard totalBalanceSats > 0, btcPrice > 0, percent >= 0 else { return "" }
        let targetSats = (totalBalanceSats * UInt64(percent)) / 100
        return formatSatsForUnit(targetSats, unit: unit, btcPrice: btcPrice)
    }

    /// Convenience wrapper for applying percentage allocation.
    static func applyPercentage(
        _ percent: Int,
        totalBalanceSats: UInt64,
        btcPrice: Double,
        unit: SendAmountUnit
    ) -> String {
        calculatePercentageAmount(
            percent: percent,
            totalBalanceSats: totalBalanceSats,
            unit: unit,
            btcPrice: btcPrice
        )
    }
}
