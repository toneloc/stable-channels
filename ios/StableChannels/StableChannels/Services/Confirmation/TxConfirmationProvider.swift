import Foundation

protocol TxConfirmationProvider: Sendable {
    func blockHeight(for txid: String) async throws -> UInt32?
}

/// Whether the block explorer still knows a transaction, in its mempool or in a block.
protocol TxPresenceProviding: Sendable {
    /// true: an explorer knows it. false: every explorer that answered says it does not exist.
    /// nil: no explorer answered.
    func isKnown(txid: String) async -> Bool?
}
