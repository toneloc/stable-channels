import Foundation

enum TradeAction: String, Codable {
    case buyBTC = "buy"
    case sellBTC = "sell"

    var displayName: String {
        switch self {
        case .buyBTC: return "Buy BTC"
        case .sellBTC: return "Sell BTC"
        }
    }
}

struct PendingTrade {
    let action: TradeAction
    let amountUSD: Double
    let btcPrice: Double
    let feeUSD: Double
    let btcAmount: Double
    let netAmountUSD: Double
}

/// Trade payment sent but not yet confirmed by LDK.
/// Stored in a map keyed by PaymentId so we can finalize on PaymentSuccessful
/// or revert on PaymentFailed.
struct PendingTradePayment {
    let newExpectedUSD: Double
    let price: Double
    let tradeDbId: Int64
    let action: String  // "buy" or "sell"
}

/// Outgoing splice awaiting confirmation.
struct PendingSplice {
    let direction: String  // "in" or "out"
    let amountSats: UInt64
    let address: String?  // For splice_out
}

// MARK: - Database Records

struct ChannelRecord: Codable {
    let channelId: String
    let userChannelId: String
    let expectedUSD: Double
    let note: String?
    let backingSats: UInt64
    let receiverSats: UInt64
    let latestPrice: Double
}

struct TradeRecord: Codable, Identifiable {
    let id: Int64
    let channelId: String
    let action: String
    let amountUSD: Double
    let amountBTC: Double
    let btcPrice: Double
    let feeUSD: Double
    let paymentId: String?
    let status: String
    let createdAt: Int64

    var date: Date {
        Date(timeIntervalSince1970: TimeInterval(createdAt))
    }

    var tradeAction: TradeAction? {
        TradeAction(rawValue: action)
    }
}

struct PaymentRecord: Codable, Identifiable {
    let id: Int64
    let paymentId: String?
    let paymentType: String
    let direction: String
    let amountMsat: UInt64
    let amountUSD: Double?
    let btcPrice: Double?
    let counterparty: String?
    let status: String
    let createdAt: Int64
    let feeMsat: UInt64
    let txid: String?
    let address: String?
    let confirmations: UInt32

    var date: Date {
        Date(timeIntervalSince1970: TimeInterval(createdAt))
    }

    var amountSats: UInt64 {
        amountMsat / 1000
    }

    var isIncoming: Bool {
        direction == "received"
    }
}

struct PriceRecord: Codable, Identifiable {
    let id: Int64
    let price: Double
    let source: String?
    let timestamp: Int64

    var date: Date {
        Date(timeIntervalSince1970: TimeInterval(timestamp))
    }
}

struct DailyPriceRecord: Codable, Identifiable {
    var id: String { date }
    let date: String
    let open: Double
    let high: Double
    let low: Double
    let close: Double
    let volume: Double?
}

struct OnchainTxRecord: Codable, Identifiable {
    let id: Int64
    let txid: String
    let direction: String
    let amountSats: UInt64
    let address: String?
    let btcPrice: Double?
    let status: String
    let confirmations: UInt32
    let createdAt: Int64

    var date: Date {
        Date(timeIntervalSince1970: TimeInterval(createdAt))
    }
}
