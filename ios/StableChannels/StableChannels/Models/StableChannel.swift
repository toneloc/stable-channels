import Foundation

// MARK: - Bitcoin

struct Bitcoin: Codable, Equatable {
    var sats: UInt64

    static let zero = Bitcoin(sats: 0)

    static func fromSats(_ sats: UInt64) -> Bitcoin {
        Bitcoin(sats: sats)
    }

    static func fromBTC(_ btc: Double) -> Bitcoin {
        let sats = UInt64((btc * Double(Constants.satsInBTC)).rounded())
        return Bitcoin(sats: sats)
    }

    func toBTC() -> Double {
        Double(sats) / Double(Constants.satsInBTC)
    }

    static func fromUSD(_ usd: USD, price: Double) -> Bitcoin {
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
        let btcValue = amount / price
        let sats = btcValue * Double(Constants.satsInBTC)
        let millisats = sats * 1000.0
        return UInt64(abs(millisats).rounded(.down))
    }

    var formatted: String {
        String(format: "$%.2f", amount)
    }
}

// MARK: - StableChannel

struct StableChannel: Codable {
    var channelId: String  // ldk-node ChannelId is a String in Swift bindings
    var userChannelId: String  // ldk-node UserChannelId is a String in Swift bindings
    var isStableReceiver: Bool
    var counterparty: String  // hex-encoded pubkey (ldk-node PublicKey = String)
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
        lastStabilityPayment: 0
    )
}
