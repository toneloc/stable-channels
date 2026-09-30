import Foundation

// MARK: - Bitcoin

struct Bitcoin: Codable, Equatable {
    var sats: UInt64

    static let zero = Bitcoin(sats: 0)

    static func fromSats(_ sats: UInt64) -> Bitcoin {
        Bitcoin(sats: sats)
    }

    static func fromBTC(_ btc: Double) -> Bitcoin {
        guard btc > 0, btc.isFinite else { return Bitcoin(sats: 0) }
        let sats = btc * Double(Constants.satsInBTC)
        guard !sats.isNaN, !sats.isInfinite, sats >= 0 else { return Bitcoin(sats: 0) }
        let clamped = min(sats.rounded(), Double(UInt64.max))
        return Bitcoin(sats: UInt64(clamped))
    }

    func toBTC() -> Double {
        Double(sats) / Double(Constants.satsInBTC)
    }

    static func fromUSD(_ usd: USD, price: Double) -> Bitcoin {
        guard price > 0, price.isFinite, usd.amount > 0, usd.amount.isFinite else {
            return Bitcoin(sats: 0)
        }
        let btc = usd.amount / price
        return Bitcoin.fromBTC(btc)
    }

    var formatted: String {
        let btcValue = toBTC()
        return String(format: "%.8f BTC", btcValue)
    }
}

// MARK: - USD

struct USD: Codable, Equatable {
    var amount: Double

    static let zero = USD(amount: 0.0)

    static func fromBitcoin(_ btc: Bitcoin, price: Double) -> USD {
        USD(amount: btc.toBTC() * price)
    }

    func toMsats(price: Double) -> UInt64 {
        guard price > 0 else { return 0 }
        let btcValue = amount / price
        let sats = btcValue * Double(Constants.satsInBTC)
        let millisats = sats * 1000.0
        guard !millisats.isNaN, !millisats.isInfinite, millisats >= 0 else { return 0 }
        let rounded = abs(millisats).rounded(.down)
        guard rounded < Double(UInt64.max) else { return UInt64.max }
        return UInt64(rounded)
    }

    var formatted: String {
        String(format: "$%.2f", amount)
    }
}

// MARK: - StableChannel

struct StableChannel: Codable {
    var channelId: String // ldk-node ChannelId is a String in Swift bindings
    var userChannelId: String // ldk-node UserChannelId is a String in Swift bindings
    var isStableReceiver: Bool
    var counterparty: String // hex-encoded pubkey (ldk-node PublicKey = String)
    var expectedUSD: USD
    var expectedBTC: Bitcoin
    var stableReceiverBTC: Bitcoin
    var stableProviderBTC: Bitcoin
    var stableReceiverUSD: USD
    var stableProviderUSD: USD
    var riskLevel: Int32
    var timestamp: Int64
    var formattedDatetime: String
    var paymentMade: Bool
    var scDir: String
    var latestPrice: Double
    var prices: String
    var onchainBTC: Bitcoin
    var onchainUSD: USD
    var note: String?
    var nativeChannelBTC: Bitcoin
    var backingSats: UInt64
    var nativeSats: UInt64
    var lastStabilityPayment: Int64

    static let `default` = StableChannel(
        channelId: "",
        userChannelId: "",
        isStableReceiver: true,
        counterparty: Constants.defaultLSPPubkey,
        expectedUSD: .zero,
        expectedBTC: .zero,
        stableReceiverBTC: .zero,
        stableProviderBTC: .zero,
        stableReceiverUSD: .zero,
        stableProviderUSD: .zero,
        riskLevel: 0,
        timestamp: Int64(Date().timeIntervalSince1970),
        formattedDatetime: "",
        paymentMade: false,
        scDir: ".data",
        latestPrice: 0.0,
        prices: "",
        onchainBTC: .zero,
        onchainUSD: .zero,
        note: nil,
        nativeChannelBTC: .zero,
        backingSats: 0,
        nativeSats: 0,
        lastStabilityPayment: 0
    )
}

// MARK: - Domain Errors

enum StabilitySpendError: LocalizedError, Equatable {
    case surplusSettling(owedUSD: Double)

    var errorDescription: String? {
        switch self {
        case .surplusSettling(let owedUSD):
            let formatted = owedUSD.formatted(.currency(code: "USD"))
            return "A stability payment of \(formatted) to the LSP is still settling -- retry this payment shortly."
        }
    }
}

enum SpliceOperationError: LocalizedError, Equatable {
    case inProgress
    case databaseUnavailable
    case persistenceFailed(underlyingDescription: String?)

    var errorDescription: String? {
        switch self {
        case .inProgress:
            return "A splice is already in progress — try again shortly"
        case .databaseUnavailable:
            return "Payment history is unavailable — splice not started"
        case .persistenceFailed:
            return "Could not save pending splice — splice not started"
        }
    }
}
