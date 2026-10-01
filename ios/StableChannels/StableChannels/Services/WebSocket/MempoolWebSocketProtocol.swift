import Foundation

enum WebSocketEvent {
    case receive(target: String, txid: String, amountSats: Int64)
    case removed(target: String, txid: String)
    case trackedOutspend(trackedTxid: String, spendingTxid: String)
}

struct MempoolWSFees: Codable, Equatable, Sendable {
    let fastestFee: Double
    let halfHourFee: Double
    let hourFee: Double
    let economyFee: Double?
    let minimumFee: Double

    init(
        fastestFee: Double,
        halfHourFee: Double,
        hourFee: Double,
        economyFee: Double? = nil,
        minimumFee: Double
    ) {
        self.fastestFee = fastestFee
        self.halfHourFee = halfHourFee
        self.hourFee = hourFee
        self.economyFee = economyFee
        self.minimumFee = minimumFee
    }
}

@MainActor
protocol MempoolWebSocketProtocol: AnyObject {
    var isConnected: Bool { get }
    var onTransactionDetected: ((WebSocketEvent) -> Void)? {
        get set
    }
    var onBlockHeader: ((MempoolWSBlock) -> Void)? { get set }
    var onFeesUpdated: ((MempoolWSFees) -> Void)? { get set }
    var latestFees: MempoolWSFees? { get }
    func connect()
    func disconnect()
    func trackAddress(_ address: String)
    func untrackAddress(_ address: String)
    func trackTx(_ txid: String)
    func untrackTx(_ txid: String)
}
