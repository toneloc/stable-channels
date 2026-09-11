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

    func setCloseTxid(_ txid: String?) {
        txidLinks.setClose(txid)
    }

    func setReceiveTxid(_ txid: String?) {
        txidLinks.setReceive(txid)
    }

    func clearReceiveTxid() {
        setReceiveTxid(nil)
    }

    func clearCloseTxid() {
        setCloseTxid(nil)
    }
}
