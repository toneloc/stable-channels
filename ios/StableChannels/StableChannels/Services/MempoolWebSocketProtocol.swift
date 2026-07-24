import Foundation

protocol MempoolWebSocketProtocol: AnyObject {
    var isConnected: Bool { get }
    var onTransactionDetected: ((_ target: String, _ isTxid: Bool, _ txid: String, _ amountSats: Int64) -> Void)? {
        get set
    }
    var onBlockHeader: ((_ height: UInt32) -> Void)? { get set }
    func connect()
    func disconnect()
    func trackAddress(_ address: String)
    func untrackAddress(_ address: String)
    func trackTx(_ txid: String)
    func untrackTx(_ txid: String)
}
