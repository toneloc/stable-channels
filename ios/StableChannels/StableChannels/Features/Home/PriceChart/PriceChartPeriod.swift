import Foundation

/// Periods available for price chart display with timeframe calculations and date formats.
enum ChartPeriod: String, CaseIterable, Sendable {
    case day = "1D"
    case week = "1W"
    case month = "1M"
    case threeMonth = "3M"
    case sixMonth = "6M"
    case ytd = "YTD"
    case year = "1Y"
    case twoYear = "2Y"
    case fiveYear = "5Y"
    case tenYear = "10Y"
    case all = "ALL"

    var days: UInt32 {
        switch self {
        case .day: return 1
        case .week: return 7
        case .month: return 30
        case .threeMonth: return 90
        case .sixMonth: return 180
        case .ytd:
            let now = Date()
            let jan1 = Calendar.current.date(from: Calendar.current.dateComponents([.year], from: now)) ?? now
            return UInt32(max(1, Int(now.timeIntervalSince(jan1) / 86400) + 1))
        case .year: return 365
        case .twoYear: return 730
        case .fiveYear: return 1825
        case .tenYear: return 3650
        case .all: return 99999
        }
    }

    var usesHourly: Bool {
        switch self {
        case .day, .week, .month: return true
        default: return false
        }
    }

    var dateFormat: Date.FormatStyle {
        switch self {
        case .day:
            return .dateTime.hour().minute()
        case .week, .month, .threeMonth:
            return .dateTime.month(.abbreviated).day()
        case .sixMonth, .ytd, .year:
            return .dateTime.month(.abbreviated).year(.twoDigits)
        default:
            return .dateTime.month(.abbreviated).year()
        }
    }

    var xAxisFormat: Date.FormatStyle {
        switch self {
        case .day:
            return .dateTime.hour()
        case .week, .month:
            return .dateTime.month(.abbreviated).day()
        case .threeMonth, .sixMonth, .ytd:
            return .dateTime.month(.abbreviated)
        default:
            return .dateTime.year()
        }
    }
}
