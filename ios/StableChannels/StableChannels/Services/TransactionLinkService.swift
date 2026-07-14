import Observation

/// Service responsible for managing transaction link states (receive address,
/// stored txids).
@MainActor
@Observable
final class TransactionLinkService {
    private let txidLinks = TxidLinkStore()

    var onchainReceiveAddress: String? {
        didSet {
            if onchainReceiveAddress != nil {
                setReceiveTxid(nil)
            }
        }
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
