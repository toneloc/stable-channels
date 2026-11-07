import Foundation
import SCForceClose
import SQLite3

/// The app's own force close (#456). ldk-node does not broadcast the close of an anchor channel with
/// a peer in `trustedPeersNoReserve`, and the app lists the LSP there, so the close would wait for
/// the LSP to broadcast its own.
///
/// The app force closes only when the LSP is offline or a normal close is stuck. After `start`, the
/// channel is followed until a close of it confirms: the app builds its close with a fee bump and
/// broadcasts both to the explorers that don't have a close of the channel, again every `retrySeconds`.
/// The LSP's close can still replace ours in the mempool when it comes back, as it can pay a higher
/// fee from its wallet. After `maxBuildFailures` failed builds the app stops, as building
/// stops the node each time; the LSP's close still frees the funds.
actor OwnForceClose {
    struct State: Codable, Equatable {
        let channelId: String
        /// The close's row in payment history.
        let paymentId: String
        let fundingTxid: String
        let fundingVout: UInt32
        var nextAt: TimeInterval
        var closeTxid: String?
        var closeHex: String?
        var feeBumpTxid: String?
        var feeBumpHex: String?
        var broadcastAt: TimeInterval = 0
        /// The unconfirmed close last seen spending the funding output: ours or the LSP's.
        var spendTxid: String?
        var buildFailures = 0
        /// When an explorer first showed our close as `spendTxid`.
        var seenAt: TimeInterval?
        /// A close seen confirmed while an explorer was silent; a later step must see it too.
        var confirmedTxid: String?

        /// Our close is in a mempool, unconfirmed `slowSeconds` after an explorer first showed it.
        func isSlow(now: TimeInterval) -> Bool {
            guard let seenAt, closeTxid != nil, spendTxid == closeTxid else { return false }
            return now - seenAt >= OwnForceClose.slowSeconds
        }
    }

    static let checkSeconds: TimeInterval = 60
    static let retrySeconds: TimeInterval = 10 * 60
    static let slowSeconds: TimeInterval = 60 * 60
    static let maxBuildFailures = 3
    private static let stateKey = "own_force_close"
    private static let errorPrefix = "error: "

    private let session: URLSession
    private let chainURLs: [String]
    private let defaults: UserDefaults
    private let now: @Sendable () -> TimeInterval
    private let log: @Sendable (String, [String: Any]) -> Void

    private(set) var state: State?
    private var running = false

    init(
        session: URLSession = .shared,
        chainURLs: [String],
        defaults: UserDefaults = .standard,
        now: @escaping @Sendable () -> TimeInterval = { Date().timeIntervalSince1970 },
        log: @escaping @Sendable (String, [String: Any]) -> Void = { AuditService.log($0, data: $1) }
    ) {
        self.session = session
        self.chainURLs = chainURLs
        self.defaults = defaults
        self.now = now
        self.log = log
        state = defaults.data(forKey: Self.stateKey)
            .flatMap { try? JSONDecoder().decode(State.self, from: $0) }
    }

    var isDue: Bool { state.map { now() >= $0.nextAt } ?? false }

    func start(channelId: String, paymentId: String, fundingTxid: String, fundingVout: UInt32) {
        let at = now()
        save(State(
            channelId: channelId,
            paymentId: paymentId,
            fundingTxid: fundingTxid,
            fundingVout: fundingVout,
            nextAt: at
        ))
    }

    /// Stops following the close. A restore from seed calls this: the restored node lacks the channel
    /// data to sweep what our close pays us, while the recovery sweeps the LSP's close (#364). A step
    /// in progress saves nothing after this.
    func clear() {
        save(nil)
    }

    /// One step. `build` builds the channel's close, with the node stopped.
    /// `onConfirmed` gets the close's payment row and the close that confirmed: ours or the LSP's.
    /// Returns a message for the user, or nil when there is nothing new to tell.
    func step(
        build: @Sendable (String) async throws -> ForceCloseTransactions,
        onConfirmed: @Sendable (_ paymentId: String, _ closeTxid: String) async -> Void = { _, _ in }
    ) async -> String? {
        guard !running, var state else { return nil }
        running = true
        defer { running = false }
        let spend = await fundingSpend(state)
        if let spend, spend.confirmed {
            // One explorer's word, with another silent, ends the follow-up only once repeated.
            if !spend.allAnswered, state.confirmedTxid != spend.txid {
                var next = state
                next.confirmedTxid = spend.txid
                next.nextAt = now() + Self.checkSeconds
                save(next, ifStill: state)
                return nil
            }
            log("OWN_FORCE_CLOSE_DONE", ["channel_id": state.channelId, "close_txid": spend.txid ?? ""])
            save(nil, ifStill: state)
            if let txid = spend.txid { await onConfirmed(state.paymentId, txid) }
            return nil
        }
        if state.closeHex == nil {
            let close: ForceCloseTransactions
            do {
                close = try await build(state.channelId)
            } catch {
                return buildFailed(state, error)
            }
            var built = state
            built.closeTxid = close.closeTxid
            built.closeHex = close.closeTxHex
            built.feeBumpTxid = close.anchorChildTxid
            built.feeBumpHex = close.anchorChildHex
            save(built, ifStill: state)
            guard self.state == built else { return nil }
            state = built
        }
        if let spend, !spend.unspentAt.isEmpty {
            return await send(state, to: spend.unspentAt, spendTxid: spend.txid)
        }
        var next = spend.map { seen(state, $0.txid) } ?? state
        next.nextAt = now() + Self.checkSeconds
        save(next, ifStill: state)
        return nil
    }

    private func buildFailed(_ current: State, _ error: Error) -> String {
        let failures = current.buildFailures + 1
        log("OWN_FORCE_CLOSE_FAILED", [
            "channel_id": current.channelId,
            "error": "\(error)",
            "failures": failures
        ])
        if failures >= Self.maxBuildFailures {
            save(nil, ifStill: current)
            return "Close could not be created. Funds arrive once the provider closes the channel."
        }
        var retry = current
        retry.buildFailures = failures
        retry.nextAt = now() + Self.retrySeconds
        save(retry, ifStill: current)
        return "Close not created yet. Retrying in 10 minutes."
    }

    /// `state` with `txid` as the unconfirmed close seen spending the funding output, if any. Our close is
    /// timed from when an explorer first shows it, for `State.isSlow`.
    private func seen(_ state: State, _ txid: String?) -> State {
        var next = state
        next.spendTxid = txid
        next.seenAt = txid != nil && txid == state.closeTxid ? state.seenAt ?? now() : nil
        next.confirmedTxid = nil
        return next
    }

    /// Broadcasts our close to `urls`, the explorers that see the funding output unspent, again every
    /// `retrySeconds`. `spendTxid` is a close another explorer already has.
    private func send(_ current: State, to urls: [String], spendTxid: String?) async -> String? {
        var state = seen(current, spendTxid)
        if now() < state.broadcastAt + Self.retrySeconds {
            state.nextAt = now() + Self.checkSeconds
            save(state, ifStill: current)
            return nil
        }
        // The fee bump spends the close, so it goes second.
        let closeResults = await Self.broadcast(session: session, chainURLs: urls, hex: state.closeHex ?? "")
        let bumpResults = await Self.broadcast(session: session, chainURLs: urls, hex: state.feeBumpHex ?? "")
        log("OWN_FORCE_CLOSE_BROADCAST", [
            "channel_id": state.channelId,
            "close_txid": state.closeTxid ?? "",
            "close_broadcast": "\(closeResults)",
            "fee_bump_txid": state.feeBumpTxid ?? "",
            "fee_bump_broadcast": "\(bumpResults)"
        ])
        state.broadcastAt = now()
        state.nextAt = now() + Self.checkSeconds
        save(state, ifStill: current)
        // A close of the channel is already in a mempool: nothing new to tell.
        if spendTxid != nil { return nil }
        return closeResults.values.contains { !$0.hasPrefix(Self.errorPrefix) }
            ? "Close sent."
            : "Close not accepted yet. Retrying in 10 minutes."
    }

    /// One explorer's answer: the tx spending the funding output, if any.
    private struct Outspend: Equatable {
        let txid: String?
        let confirmed: Bool
    }

    /// What the explorers that answer see. `txid` is a close one of them has. Our close goes to
    /// `unspentAt`, those that see the funding output unspent, which does no harm if it is spent. A close
    /// counts as confirmed only when all of them see it confirmed. `allAnswered` is whether every explorer
    /// answered.
    private struct Spend {
        let txid: String?
        let confirmed: Bool
        let unspentAt: [String]
        let allAnswered: Bool
    }

    private func fundingSpend(_ state: State) async -> Spend? {
        var answers: [(base: String, outspend: Outspend)] = []
        for base in chainURLs {
            if let answer = await outspend(base, state) { answers.append((base, answer)) }
        }
        guard let first = answers.first?.outspend else { return nil }
        return Spend(
            txid: answers.lazy.compactMap(\.outspend.txid).first,
            confirmed: answers.allSatisfy { $0.outspend.confirmed && $0.outspend.txid == first.txid },
            unspentAt: answers.filter { $0.outspend.txid == nil }.map(\.base),
            allAnswered: answers.count == chainURLs.count
        )
    }

    /// One explorer's answer, or nil when it gives none. Esplora reports an output of a tx it does not
    /// have as unspent, so an unspent output counts only when it has the funding tx: the close of a
    /// channel whose funding never reached the chain can't be sent.
    private func outspend(_ base: String, _ state: State) async -> Outspend? {
        guard let url = URL(string: Self.trimSlash(base) + "/tx/\(state.fundingTxid)/outspend/\(state.fundingVout)")
        else { return nil }
        var request = URLRequest(url: url)
        request.timeoutInterval = 10
        guard let (data, response) = try? await session.data(for: request),
              let http = response as? HTTPURLResponse, (200 ..< 300).contains(http.statusCode),
              let outspend = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return nil }
        guard outspend["spent"] as? Bool == true else {
            return await hasTx(base, state.fundingTxid) ? Outspend(txid: nil, confirmed: false) : nil
        }
        let txid = (outspend["txid"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        let confirmed = (outspend["status"] as? [String: Any])?["confirmed"] as? Bool == true
        return Outspend(txid: txid, confirmed: confirmed)
    }

    /// Whether the explorer has `txid`, in the mempool or on chain.
    private func hasTx(_ base: String, _ txid: String) async -> Bool {
        guard let url = URL(string: Self.trimSlash(base) + "/tx/\(txid)/hex") else { return false }
        var request = URLRequest(url: url)
        request.timeoutInterval = 10
        guard let (_, response) = try? await session.data(for: request),
              let http = response as? HTTPURLResponse else { return false }
        return (200 ..< 300).contains(http.statusCode)
    }

    /// Saves `next` unless a `start` during the step's awaits replaced `current`.
    private func save(_ next: State?, ifStill current: State) {
        if state == current { save(next) }
    }

    private func save(_ state: State?) {
        self.state = state
        if let state, let data = try? JSONEncoder().encode(state) {
            defaults.set(data, forKey: Self.stateKey)
        } else {
            defaults.removeObject(forKey: Self.stateKey)
        }
    }

    private static func trimSlash(_ base: String) -> String {
        base.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }

    /// Builds the signed close of `channelId` and its fee bump from the node's stored data. The node
    /// must be stopped and the wallet dir lock held, so no other SQLite writes the data while it is
    /// read. `seed` is the 64-byte wallet seed.
    static func build(dataDir: URL, channelId: String, seed: Data) throws -> ForceCloseTransactions {
        try buildForceClose(
            channelId: channelId,
            seed: seed,
            storedValues: readMonitors(dataDir: dataDir)
        )
    }

    /// ldk-node's channel monitors and their pending updates, read in one query.
    static func readMonitors(dataDir: URL) throws -> [StoredValue] {
        let path = dataDir.appendingPathComponent("ldk_node_data.sqlite").path
        var db: OpaquePointer?
        defer { sqlite3_close(db) }
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            throw ForceCloseError.ReadFailed(reason: String(cString: sqlite3_errmsg(db)))
        }
        var statement: OpaquePointer?
        defer { sqlite3_finalize(statement) }
        let query = "SELECT primary_namespace, secondary_namespace, key, value FROM ldk_node_data "
            + "WHERE primary_namespace IN ('monitors', 'monitor_updates')"
        guard sqlite3_prepare_v2(db, query, -1, &statement, nil) == SQLITE_OK else {
            throw ForceCloseError.ReadFailed(reason: String(cString: sqlite3_errmsg(db)))
        }
        var values: [StoredValue] = []
        while true {
            let result = sqlite3_step(statement)
            if result == SQLITE_DONE { return values }
            guard result == SQLITE_ROW else {
                throw ForceCloseError.ReadFailed(reason: String(cString: sqlite3_errmsg(db)))
            }
            let blob = sqlite3_column_blob(statement, 3)
            values.append(StoredValue(
                primaryNamespace: String(cString: sqlite3_column_text(statement, 0)),
                secondaryNamespace: String(cString: sqlite3_column_text(statement, 1)),
                key: String(cString: sqlite3_column_text(statement, 2)),
                value: blob.map { Data(bytes: $0, count: Int(sqlite3_column_bytes(statement, 3))) } ?? Data()
            ))
        }
    }

    /// Sends `hex` to every explorer. Returns each explorer's answer: the txid or an error.
    static func broadcast(session: URLSession, chainURLs: [String], hex: String) async -> [String: String] {
        var results: [String: String] = [:]
        for base in chainURLs {
            guard let url = URL(string: trimSlash(base) + "/tx") else { continue }
            var request = URLRequest(url: url)
            request.httpMethod = "POST"
            request.setValue("text/plain", forHTTPHeaderField: "Content-Type")
            request.httpBody = Data(hex.utf8)
            request.timeoutInterval = 10
            do {
                let (data, response) = try await session.data(for: request)
                let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
                let status = (response as? HTTPURLResponse)?.statusCode ?? 0
                results[base] = (200 ..< 300).contains(status) ? text : "\(errorPrefix)\(status) \(text.prefix(200))"
            } catch {
                results[base] = "\(errorPrefix)\(error.localizedDescription)"
            }
        }
        return results
    }
}
