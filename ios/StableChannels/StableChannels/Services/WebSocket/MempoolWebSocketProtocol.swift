import Foundation

enum WebSocketEvent {
    case receive(target: String, txid: String, amountSats: Int64)
    case removed(target: String, txid: String)
    case trackedOutspend(trackedTxid: String, spendingTxid: String)
}

struct MempoolWSFees: Codable, Equatable, Sendable {
    let fastestFee: UInt64
    let halfHourFee: UInt64
    let hourFee: UInt64
    let economyFee: UInt64?
    let minimumFee: UInt64

    init(
        fastestFee: UInt64,
        halfHourFee: UInt64,
        hourFee: UInt64,
        economyFee: UInt64? = nil,
        minimumFee: UInt64
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
