import Foundation
import Observation

/// Service responsible for managing transaction link states (receive address,
/// stored txids).
@MainActor
@Observable
final class TransactionLinkService {
    private let txidLinks = TxidLinkStore()

    var onchainReceiveAddress: String? {
        get { txidLinks.onchainReceiveAddress }
        set { txidLinks.setReceiveAddress(newValue) }
    }

    var lastCloseTxid: String? { txidLinks.lastCloseTxid }
    var lastReceiveTxid: String? { txidLinks.lastReceiveTxid }
    var lastReceiveTxidAddress: String? { txidLinks.lastReceiveTxidAddress }

    func setCloseTxid(_ txid: String?) {
        txidLinks.setClose(txid)
    }

    /// `address`: the receive address the txid pays, when known.
    func setReceiveTxid(_ txid: String?, address: String? = nil) {
        txidLinks.setReceive(txid, address: address)
    }

    func clearReceiveTxid() {
        setReceiveTxid(nil)
    }

    /// Drops the stored receive txid only if it is `txid` (e.g. it was found not to pay the address).
    func clearReceiveTxid(ifMatching txid: String) {
        guard lastReceiveTxid == txid else { return }
        clearReceiveTxid()
    }

    func clearCloseTxid() {
        setCloseTxid(nil)
    }
}
