import Foundation

enum WebSocketEvent {
    case receive(target: String, txid: String, amountSats: Int64)
    case removed(target: String, txid: String)
    case trackedOutspend(trackedTxid: String, spendingTxid: String)
}

@MainActor
protocol MempoolWebSocketProtocol: AnyObject {
    var isConnected: Bool { get }
    var onTransactionDetected: ((WebSocketEvent) -> Void)? {
        get set
    }
    var onBlockHeader: ((MempoolWSBlock) -> Void)? { get set }
    func connect()
    func disconnect()
    func trackAddress(_ address: String)
    func untrackAddress(_ address: String)
    func trackTx(_ txid: String)
    func untrackTx(_ txid: String)
}
