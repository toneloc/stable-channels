import Foundation

protocol TxConfirmationProvider: Sendable {
    func blockHeight(for txid: String) async throws -> UInt32?
}
