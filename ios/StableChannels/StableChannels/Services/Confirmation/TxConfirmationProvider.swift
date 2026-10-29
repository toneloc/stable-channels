import Foundation

/// The explorer's confirmation status for one transaction.
struct TxStatus: Equatable, Sendable {
    /// Height of the confirming block, or nil while the transaction is unconfirmed.
    let blockHeight: UInt32?
}

/// Whether a transaction pays an address, plus the status embedded in the same `/tx` response.
struct ReceiveTxCheck: Equatable, Sendable {
    let paysToAddress: Bool
    /// Nil when the response carried no usable `status` object.
    let status: TxStatus?
}

/// Lookups rethrow cancellation only; every other failure is `.unavailable`.
protocol TxConfirmationProvider: Sendable {
    func status(for txid: String) async throws -> EsploraLookup<TxStatus>
    func receiveCheck(for txid: String, paysTo address: String) async throws -> EsploraLookup<ReceiveTxCheck>
}
