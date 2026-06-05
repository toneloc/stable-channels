import Foundation

/// Persists the most recent close and onchain-receive txids across app
/// launches, with a 7-day expiry so a txid from a long-finished session
/// does not linger on the UI forever.
///
/// One value type holds both slots and the shared UserDefaults read/write
/// logic. Callers observe via the `lastCloseTxid` / `lastReceiveTxid`
/// computed properties and call `setClose(_:)` / `setReceive(_:)` to
/// update. All access is MainActor-isolated because the values drive UI.
@MainActor
@Observable
final class TxidLinkStore {
    private(set) var lastCloseTxid: String?
    private(set) var lastReceiveTxid: String?

    private let defaults: UserDefaults?
    private static let expirySeconds: TimeInterval = 7 * 86400

    private enum Key {
        static let close = "last_close_txid"
        static let closeAt = "last_close_txid_at"
        static let receive = "last_receive_txid"
        static let receiveAt = "last_receive_txid_at"
    }

    init(defaults: UserDefaults? = UserDefaults(suiteName: Constants.appGroupIdentifier)) {
        self.defaults = defaults
        let now = Date().timeIntervalSince1970
        self.lastCloseTxid = Self.restore(key: Key.close, atKey: Key.closeAt, now: now, defaults: defaults)
        self.lastReceiveTxid = Self.restore(key: Key.receive, atKey: Key.receiveAt, now: now, defaults: defaults)
    }

    func setClose(_ txid: String?) {
        lastCloseTxid = txid
        Self.persist(txid: txid, valueKey: Key.close, atKey: Key.closeAt, defaults: defaults)
    }

    func setReceive(_ txid: String?) {
        lastReceiveTxid = txid
        Self.persist(txid: txid, valueKey: Key.receive, atKey: Key.receiveAt, defaults: defaults)
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
