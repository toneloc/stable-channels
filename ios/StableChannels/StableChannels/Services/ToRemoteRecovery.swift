import Foundation

/// Finds and moves to the wallet our side of a channel the LSP force-closed after the app lost its
/// channel data (#364, #348), which LDK no longer tracks. It does in the app what the force close
/// recovery script does by hand. It finds the close from a funding txid saved before the loss: what
/// spent the funding output is the close, and its outputs are matched against our keys (a few
/// requests). A restore on a new device has no saved funding txid; getting it from the LSP is a
/// follow-up. Scanning the scripts of all 1000 keys instead would take 1000 requests, more than the
/// public explorers allow an hour.
///
/// The search repeats on a growing delay until the LSP's close shows up. A found output is swept
/// once its close confirms, and stays counted on the Home card until the sweep confirms.
actor ToRemoteRecovery {
    struct Output: Codable, Equatable {
        let txid: String
        let vout: UInt32
        let valueSats: UInt64
        let keyIndex: Int
    }

    struct State: Codable, Equatable {
        var fundingTxid: String?
        /// Nil when not saved: the funding output is then found by its 2-of-2 witness.
        var fundingVout: UInt32?
        /// The search stops after this time when nothing new is found.
        var until: TimeInterval = 0
        var attempts = 0
        var nextAt: TimeInterval = 0
        /// Bumped by `start`, so a search that began earlier doesn't undo it.
        var generation = 0
        var found: [Output] = []
        /// Kept across attempts so a failing sweep doesn't use up wallet addresses.
        var destination: String?
        /// The last funding txid fully checked, so it is not looked up again.
        var checkedFundingTxid: String?

        var active: Bool { fundingTxid != nil || !found.isEmpty }
    }

    /// How long to keep looking for the LSP's close.
    static let searchSeconds: TimeInterval = 14 * 24 * 60 * 60
    static let settleIntervalSeconds: TimeInterval = 60
    private static let stateKey = "to_remote_recovery"

    /// 1 minute, as the LSP closes within seconds of a reconnect, then 10 minutes doubling up to a
    /// day for an LSP that is slow or offline.
    static func backoffSeconds(_ attempts: Int) -> TimeInterval {
        attempts <= 1 ? 60 : min(24 * 60 * 60, 10 * 60 * pow(2, Double(min(attempts - 2, 10))))
    }

    private let session: URLSession
    private let chainURLs: [String]
    private let defaults: UserDefaults
    private let now: @Sendable () -> TimeInterval
    private let log: @Sendable (String, [String: Any]) -> Void

    private(set) var state: State
    private var running = false
    private var lastSettleAt: TimeInterval = 0
    private var cachedKeys: (seed: Data, keys: ToRemoteKeys)?

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
            .flatMap { try? JSONDecoder().decode(State.self, from: $0) } ?? State()
    }

    /// Sats found on chain and not yet confirmed in the wallet.
    var pendingSats: UInt64 { state.found.reduce(0) { $0 + $1.valueSats } }

    /// Whether a funding txid that LDK no longer knows still needs looking up.
    func needsCheck(_ fundingTxid: String) -> Bool {
        state.found.isEmpty && fundingTxid != state.fundingTxid && fundingTxid != state.checkedFundingTxid
    }

    /// Called on each balance refresh. A saved funding txid with no channel left in LDK means the
    /// channel data was lost (#348), so that funding txid gets looked up. Returns whether recovery
    /// may run now: never while LDK handles a close itself, so the two never race.
    func onRefresh(savedFundingTxid: String?, ldkHasChannels: Bool, ldkHandlesClose: Bool) -> Bool {
        if ldkHandlesClose { return false }
        if let savedFundingTxid, !ldkHasChannels, needsCheck(savedFundingTxid) {
            start(fundingTxid: savedFundingTxid, fundingVout: nil)
        }
        return state.active
    }

    /// Starts looking for the close of the channel funded by `fundingTxid`, which LDK doesn't track.
    /// With `repeats` false it looks once, for a close that already happened.
    func start(fundingTxid: String, fundingVout: UInt32?, repeats: Bool = true) {
        let until = repeats ? now() + Self.searchSeconds : now()
        update {
            $0.fundingTxid = fundingTxid
            $0.fundingVout = fundingVout
            $0.until = max($0.until, until)
            $0.attempts = 0
            $0.nextAt = 0
            $0.generation += 1
        }
        log("TO_REMOTE_RECOVERY_START", ["funding_txid": fundingTxid, "repeats": repeats])
    }

    /// After a restore from seed: starts over, as found outputs and the sweep address may belong to
    /// the wallet that was replaced, then looks for the close of `fundingTxid` when it is known.
    func startAfterRestore(fundingTxid: String?, repeats: Bool) {
        update { $0 = State(generation: $0.generation + 1) }
        if let fundingTxid { start(fundingTxid: fundingTxid, fundingVout: nil, repeats: repeats) }
    }

    /// Does whatever is due. Cheap to call often; returns at once while another run is going.
    /// `seed` is the 64-byte wallet seed. `onSwept` gets each broadcast sweep's txid, the sats it
    /// pays the wallet and the address.
    func run(
        seed: Data,
        destination: @Sendable () async -> String?,
        feeRateSatVb: @Sendable () async -> UInt64?,
        onSwept: @Sendable (String, UInt64, String) async -> Void = { _, _, _ in }
    ) async {
        guard state.active, !running, let keys = keys(for: seed) else { return }
        running = true
        defer { running = false }
        if !state.found.isEmpty {
            await settle(keys, destination: destination, feeRateSatVb: feeRateSatVb, onSwept: onSwept)
        }
        if state.fundingTxid != nil {
            await search(keys)
        }
    }

    private func keys(for seed: Data) -> ToRemoteKeys? {
        if let cachedKeys, cachedKeys.seed == seed { return cachedKeys.keys }
        guard let keys = try? ToRemoteKeys(seed: seed) else { return nil }
        cachedKeys = (seed, keys)
        return keys
    }

    private func search(_ keys: ToRemoteKeys) async {
        let s = state
        guard let fundingTxid = s.fundingTxid, now() >= s.nextAt else { return }
        let result = await findByFunding(fundingTxid, vout: s.fundingVout, keys: keys)
        let time = now()
        let ends = result?.spent == true || time >= s.until
        let added = (result?.outputs ?? []).filter { output in
            !s.found.contains { $0.txid == output.txid && $0.vout == output.vout }
        }
        update { current in
            guard current.generation == s.generation else { return }
            current.found += added
            if ends {
                current.fundingTxid = nil
                current.checkedFundingTxid = fundingTxid
            }
            current.attempts += 1
            current.nextAt = time + Self.backoffSeconds(current.attempts)
        }
        if !added.isEmpty {
            log("TO_REMOTE_FOUND", [
                "outpoints": added.map { "\($0.txid):\($0.vout)" }.joined(separator: ","),
                "sats": added.reduce(0) { $0 + $1.valueSats }
            ])
        }
        if !state.active { log("TO_REMOTE_RECOVERY_END", ["found": false]) }
    }

    /// The outputs paying our keys in whatever spent the funding output, and whether the funding
    /// output is spent. With no saved vout, the funding output is the one spent with a 2-of-2
    /// multisig witness, as every channel close is. Nil when the explorer can't be reached. Outputs
    /// already swept and confirmed are left out.
    private func findByFunding(
        _ txid: String,
        vout: UInt32?,
        keys: ToRemoteKeys
    ) async -> (outputs: [Output], spent: Bool)? {
        guard let spends = await getJSON("/tx/\(txid)/outspends") as? [Any] else { return nil }
        let indices = vout.map { [Int($0)] } ?? Array(spends.indices)
        var fundingSpent = false
        var found: [Output] = []
        for index in indices {
            guard index < spends.count,
                  let spend = spends[index] as? [String: Any],
                  spend["spent"] as? Bool == true,
                  let closeTxid = spend["txid"] as? String else { continue }
            guard let close = await getJSON("/tx/\(closeTxid)") as? [String: Any] else { return nil }
            if vout != nil || Self.spendsWithTwoOfTwo(close, txid: txid, index: index) { fundingSpent = true }
            let outputs = close["vout"] as? [[String: Any]] ?? []
            let matched = outputs.enumerated().compactMap { n, out -> Output? in
                guard let script = (out["scriptpubkey"] as? String).flatMap({ Data(hex: $0) }),
                      let key = keys.match(script),
                      let value = (out["value"] as? NSNumber)?.uint64Value else { return nil }
                return Output(txid: closeTxid, vout: UInt32(n), valueSats: value, keyIndex: key.index)
            }
            if matched.isEmpty { continue }
            guard let closeSpends = await getJSON("/tx/\(closeTxid)/outspends") as? [Any] else { return nil }
            found += matched.filter {
                let spend = Int($0.vout) < closeSpends.count ? closeSpends[Int($0.vout)] : nil
                return !Self.isSpentAndConfirmed(spend)
            }
        }
        return (found, fundingSpent)
    }

    /// Whether `tx` spends txid:index with a 2-of-2 multisig witness script.
    private static func spendsWithTwoOfTwo(_ tx: [String: Any], txid: String, index: Int) -> Bool {
        let inputs = tx["vin"] as? [[String: Any]] ?? []
        return inputs.contains { input in
            guard input["txid"] as? String == txid,
                  (input["vout"] as? NSNumber)?.intValue == index,
                  let script = (input["witness"] as? [String])?.last else { return false }
            return isTwoOfTwo(script)
        }
    }

    private static func isTwoOfTwo(_ script: String) -> Bool {
        let bytes = Array(script.utf8)
        return bytes.count == 142 && script.hasPrefix("5221") && script.hasSuffix("52ae")
            && String(decoding: bytes[70 ..< 72], as: UTF8.self) == "21"
    }

    private func settle(
        _ keys: ToRemoteKeys,
        destination: @Sendable () async -> String?,
        feeRateSatVb: @Sendable () async -> UInt64?,
        onSwept: @Sendable (String, UInt64, String) async -> Void
    ) async {
        guard now() - lastSettleAt >= Self.settleIntervalSeconds else { return }
        lastSettleAt = now()
        var done: [Output] = []
        for output in state.found {
            guard let spend = await getJSON("/tx/\(output.txid)/outspend/\(output.vout)") as? [String: Any] else {
                continue
            }
            if spend["spent"] as? Bool == true {
                if Self.isSpentAndConfirmed(spend) { done.append(output) }
                continue
            }
            // The output is locked until one block after the close confirms (1 OP_CSV).
            guard let status = await getJSON("/tx/\(output.txid)/status") as? [String: Any],
                  status["confirmed"] as? Bool == true else { continue }
            var address = state.destination
            if address == nil, let fresh = await destination() {
                address = fresh
                update { $0.destination = fresh }
            }
            guard let address,
                  let script = Bech32.segwitScriptPubKey(address, expectedHrp: "bc"),
                  let estimate = await feeRateSatVb() else { continue }
            // 1 sat/vB is the floor the sweep pays.
            let feeRate = max(estimate, 1)
            let key = keys.keys[output.keyIndex]
            func sweep(at rate: UInt64) throws -> ToRemoteSweep.Signed? {
                try ToRemoteSweep.build(
                    key: key,
                    txid: output.txid,
                    vout: output.vout,
                    valueSats: output.valueSats,
                    destinationScript: script,
                    feeRateSatVb: rate
                )
            }
            let tx: ToRemoteSweep.Signed?
            let atMinimum: ToRemoteSweep.Signed?
            do {
                tx = try sweep(at: feeRate)
                atMinimum = tx == nil ? try sweep(at: 1) : tx
            } catch {
                continue
            }
            guard let tx else {
                // Waits for lower fees; dropped only when even 1 sat/vB would leave dust.
                if atMinimum == nil {
                    log("TO_REMOTE_DUST", ["outpoint": "\(output.txid):\(output.vout)", "sats": output.valueSats])
                    done.append(output)
                }
                continue
            }
            let result = await broadcast(tx.hex)
            log("TO_REMOTE_SWEEP_BROADCAST", [
                "outpoint": "\(output.txid):\(output.vout)",
                "result": result,
                "sats": output.valueSats,
                "fee_rate": feeRate
            ])
            if !result.hasPrefix("error:") {
                await onSwept(tx.txid, tx.amountSats, address)
            }
        }
        guard !done.isEmpty else { return }
        update {
            $0.found.removeAll { done.contains($0) }
            if $0.found.isEmpty { $0.destination = nil }
        }
        if !state.active { log("TO_REMOTE_RECOVERY_END", ["found": true]) }
    }

    private static func isSpentAndConfirmed(_ spend: Any?) -> Bool {
        guard let spend = spend as? [String: Any], spend["spent"] as? Bool == true else { return false }
        return (spend["status"] as? [String: Any])?["confirmed"] as? Bool == true
    }

    private func update(_ change: (inout State) -> Void) {
        change(&state)
        if let data = try? JSONEncoder().encode(state) {
            defaults.set(data, forKey: Self.stateKey)
        }
    }

    private nonisolated func url(_ base: String, _ path: String) -> URL? {
        URL(string: base.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + path)
    }

    private nonisolated func getJSON(_ path: String) async -> Any? {
        for base in chainURLs {
            guard let url = url(base, path) else { continue }
            var request = URLRequest(url: url)
            request.timeoutInterval = 10
            guard let (data, response) = try? await session.data(for: request),
                  let http = response as? HTTPURLResponse, (200 ..< 300).contains(http.statusCode) else { continue }
            if let json = try? JSONSerialization.jsonObject(with: data) { return json }
        }
        return nil
    }

    /// Returns the txid, or the last error when no explorer accepted the transaction.
    private nonisolated func broadcast(_ hex: String) async -> String {
        var error = "unreachable"
        for base in chainURLs {
            guard let url = url(base, "/tx") else { continue }
            var request = URLRequest(url: url)
            request.httpMethod = "POST"
            request.setValue("text/plain", forHTTPHeaderField: "Content-Type")
            request.httpBody = Data(hex.utf8)
            request.timeoutInterval = 10
            do {
                let (data, response) = try await session.data(for: request)
                let text = String(decoding: data, as: UTF8.self)
                if let http = response as? HTTPURLResponse, (200 ..< 300).contains(http.statusCode) {
                    return text.trimmingCharacters(in: .whitespacesAndNewlines)
                }
                error = "\((response as? HTTPURLResponse)?.statusCode ?? 0) \(text.prefix(200))"
            } catch let failure {
                error = failure.localizedDescription
            }
        }
        return "error: \(error)"
    }
}
