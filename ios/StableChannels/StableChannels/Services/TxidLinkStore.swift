import Foundation

/// Persists the most recent close and onchain-receive txids and receive
/// address across app launches, with a 7-day expiry for txids so a txid
/// from a long-finished session does not linger on the UI forever.
///
/// The receive address is persisted without expiry so the deposit resolver
/// can match incoming onchain funds after app restart. It is cleared when
/// the user generates a new address.
@MainActor
@Observable
final class TxidLinkStore {
    private(set) var lastCloseTxid: String?
    private(set) var lastReceiveTxid: String?
    private(set) var onchainReceiveAddress: String?

    private let defaults: UserDefaults?
    private static let expirySeconds: TimeInterval = 7 * 86400

    private enum Key {
        static let close = "last_close_txid"
        static let closeAt = "last_close_txid_at"
        static let receive = "last_receive_txid"
        static let receiveAt = "last_receive_txid_at"
        static let receiveAddr = "last_receive_address"
    }

    init(defaults: UserDefaults? = UserDefaults(suiteName: Constants.appGroupIdentifier)) {
        self.defaults = defaults
        let now = Date().timeIntervalSince1970
        self.lastCloseTxid = Self.restore(key: Key.close, atKey: Key.closeAt, now: now, defaults: defaults)
        self.lastReceiveTxid = Self.restore(key: Key.receive, atKey: Key.receiveAt, now: now, defaults: defaults)
        self.onchainReceiveAddress = defaults?.string(forKey: Key.receiveAddr)
    }

    func setClose(_ txid: String?) {
        lastCloseTxid = txid
        Self.persist(txid: txid, valueKey: Key.close, atKey: Key.closeAt, defaults: defaults)
    }

    func setReceive(_ txid: String?) {
        lastReceiveTxid = txid
        Self.persist(txid: txid, valueKey: Key.receive, atKey: Key.receiveAt, defaults: defaults)
    }

    func setReceiveAddress(_ address: String?) {
        onchainReceiveAddress = address
        if let address {
            defaults?.set(address, forKey: Key.receiveAddr)
        } else {
            defaults?.removeObject(forKey: Key.receiveAddr)
        }
    }

    func clearReceiveAddress() {
        setReceiveAddress(nil)
    }

    private static func restore(key: String, atKey: String, now: TimeInterval, defaults: UserDefaults?) -> String? {
        guard let stored = defaults?.string(forKey: key),
              let storedAt = defaults?.object(forKey: atKey) as? Int64,
              now - TimeInterval(storedAt) < expirySeconds
        else {
            defaults?.removeObject(forKey: key)
            defaults?.removeObject(forKey: atKey)
            return nil
        }
        return stored
    }

    private static func persist(txid: String?, valueKey: String, atKey: String, defaults: UserDefaults?) {
        defaults?.set(txid, forKey: valueKey)
        if txid != nil {
            defaults?.set(Int64(Date().timeIntervalSince1970), forKey: atKey)
        } else {
            defaults?.removeObject(forKey: atKey)
        }
    }
}
