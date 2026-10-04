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
enum SendAmountCalculator {
    /// Normalizes user text input based on the active currency unit.
    static func normalizeInput(_ text: String, unit: SendAmountUnit) -> String {
        guard !text.isEmpty else { return "" }
        switch unit {
        case .usd:
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let val = Double(trimmed), val >= 0 else { return "" }
            return String(format: "%.2f", val)
        case .sats:
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            // Strip decimal part if present
            let integerPart = trimmed.split(separator: ".").first.map(String.init) ?? trimmed
            guard let sats = UInt64(integerPart) else { return "" }
            return "\(sats)"
        case .btc:
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let btc = Double(trimmed), btc >= 0 else { return "" }
            var formatted = String(format: "%.8f", btc)
            while formatted.hasSuffix("0") && formatted.contains(".") {
                formatted.removeLast()
            }
            if formatted.hasSuffix(".") { formatted.removeLast() }
            return formatted
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
        if let dest = destination, case .bolt11(_, _, let msat) = dest, let msat, msat > 0 {
            return msat / 1000
        }
        let trimmed = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let val = Double(trimmed), val > 0, !trimmed.starts(with: "-") else { return 0 }
        switch unit {
        case .sats:
            guard val.isFinite, val >= 1, val < Double(UInt64.max) else { return 0 }
            return UInt64(val)
        case .usd:
            guard btcPrice > 0, val.isFinite else { return 0 }
            let sats = (val / btcPrice) * Double(Constants.satsInBTC)
            guard sats.isFinite, sats >= 1, sats < Double(UInt64.max) else { return 0 }
            // Integer-floored conversion to prevent rounding overdraw
            return UInt64(floor(sats))
        case .btc:
            guard val.isFinite else { return 0 }
            let sats = val * Double(Constants.satsInBTC)
            guard sats.isFinite, sats >= 1, sats < Double(UInt64.max) else { return 0 }
            return UInt64(floor(sats))
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

    /// Formats satoshis into the requested currency unit with deterministic integer flooring.
    static func formatSatsForUnit(_ sats: UInt64, unit: SendAmountUnit, btcPrice: Double) -> String {
        guard sats > 0 else { return "" }
        switch unit {
        case .usd:
            guard btcPrice > 0 else { return "" }
            let rawUSD = (Double(sats) / Double(Constants.satsInBTC)) * btcPrice
            let flooredUSD = floor(rawUSD * 100.0) / 100.0
            return String(format: "%.2f", flooredUSD)
        case .sats:
            return "\(sats)"
        case .btc:
            let rawBTC = Double(sats) / Double(Constants.satsInBTC)
            let flooredBTC = floor(rawBTC * 100_000_000.0) / 100_000_000.0
            return String(format: "%.8f", flooredBTC)
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
        guard totalBalanceSats > 0, btcPrice > 0 else { return "" }
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
