import Foundation

/// Finds and moves to the wallet our side of a channel the LSP force-closed after the app lost its
/// channel data (#364, #348), which LDK no longer tracks. It does in the app what the force close
/// recovery script does by hand, and finds the close itself in one of two ways:
/// - by a funding txid saved before the loss: what spent the funding output is the close, and its
///   outputs are matched against our keys (a few requests),
/// - after a restore from seed, by looking up the scripts of all 1000 keys.
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
        var scan = false
        /// When the scan began: an output confirmed well before is an older close.
        var scanFrom: TimeInterval = 0
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

        var active: Bool { fundingTxid != nil || scan || !found.isEmpty }
    }

    private struct Scan {
        let outputs: [Output]
        let newClose: Bool
        let complete: Bool
    }

    /// How long to keep looking for the LSP's close.
    static let searchSeconds: TimeInterval = 14 * 24 * 60 * 60
    static let settleIntervalSeconds: TimeInterval = 60
    static let scanParallel = 4
    /// Device and block clocks can differ; a close this close to the scan start counts as new.
    static let clockSlackSeconds: TimeInterval = 2 * 60 * 60
    private static let stateKey = "to_remote_recovery"

    /// 10 minutes, doubling up to a day: the LSP's close usually shows up within hours.
    static func backoffSeconds(_ attempts: Int) -> TimeInterval {
        min(24 * 60 * 60, 10 * 60 * pow(2, Double(min(max(attempts - 1, 0), 10))))
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
            start(fundingTxid: savedFundingTxid, fundingVout: nil, scan: false)
        }
        return state.active
    }

    /// Starts looking for a close LDK doesn't track: by `fundingTxid` when known, and by scanning
    /// all keys when `scan`. With `repeats` false it searches once, for a close that already
    /// happened.
    func start(fundingTxid: String?, fundingVout: UInt32?, scan: Bool, repeats: Bool = true) {
        let time = now()
        let until = repeats ? time + Self.searchSeconds : time
        update {
            if let fundingTxid {
                $0.fundingTxid = fundingTxid
                $0.fundingVout = fundingVout
            }
            if scan { $0.scanFrom = time }
            $0.scan = $0.scan || scan
            $0.until = max($0.until, until)
            $0.attempts = 0
            $0.nextAt = 0
            $0.generation += 1
        }
        log("TO_REMOTE_RECOVERY_START", ["funding_txid": fundingTxid ?? "", "scan": scan, "repeats": repeats])
    }

    /// After a restore from seed: starts over, as found outputs and the sweep address may belong to
    /// the wallet that was replaced, then scans all keys.
    func startAfterRestore(repeats: Bool) {
        update { $0 = State(generation: $0.generation + 1) }
        start(fundingTxid: nil, fundingVout: nil, scan: true, repeats: repeats)
    }

    /// Does whatever is due. Cheap to call often; returns at once while another run is going.
    /// `seed` is the 64-byte wallet seed. Closes of channels in `liveFundingTxids` are left to LDK.
    func run(
        seed: Data,
        destination: @Sendable () async -> String?,
        feeRateSatVb: @Sendable () async -> UInt64?,
        liveFundingTxids: Set<String> = []
    ) async {
        guard state.active, !running, let keys = keys(for: seed) else { return }
        running = true
        defer { running = false }
        if !state.found.isEmpty {
            await settle(keys, destination: destination, feeRateSatVb: feeRateSatVb)
        }
        if state.fundingTxid != nil || state.scan {
            await search(keys, liveFundingTxids: liveFundingTxids)
        }
    }

    private func keys(for seed: Data) -> ToRemoteKeys? {
        if let cachedKeys, cachedKeys.seed == seed { return cachedKeys.keys }
        guard let keys = try? ToRemoteKeys(seed: seed) else { return nil }
        cachedKeys = (seed, keys)
        return keys
    }

    private func search(_ keys: ToRemoteKeys, liveFundingTxids: Set<String>) async {
        let s = state
        guard now() >= s.nextAt else { return }
        var found: [Output] = []
        var fundingSpent = false
        if let txid = s.fundingTxid, let result = await findByFunding(txid, vout: s.fundingVout, keys: keys) {
            found += result.outputs
            fundingSpent = result.spent
        }
        let scan = s.scan ? await scan(keys, liveFundingTxids: liveFundingTxids, scanFrom: s.scanFrom) : nil
        found += scan?.outputs ?? []
        let time = now()
        let pastDeadline = time >= s.until
        let fundingEnds = s.fundingTxid != nil && (fundingSpent || pastDeadline)
        // An older close found on the keys doesn't end the wait for the new one. Past the deadline
        // the scan ends once a full scan found nothing new, or two weeks later anyway.
        let scanEnds = scan.map {
            $0.newClose || pastDeadline && ($0.complete || time >= s.until + Self.searchSeconds)
        } ?? false
        let added = found.filter { output in
            !s.found.contains { $0.txid == output.txid && $0.vout == output.vout }
        }
        update { current in
            guard current.generation == s.generation else { return }
            for output in added where !current.found.contains(where: {
                $0.txid == output.txid && $0.vout == output.vout
            }) {
                current.found.append(output)
            }
            if fundingEnds {
                current.fundingTxid = nil
                current.checkedFundingTxid = s.fundingTxid
            }
            if scanEnds { current.scan = false }
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

    /// Looks up the script of every key. An output whose close is unconfirmed or confirmed since
    /// `scanFrom` counts as the new close. Closes of channels LDK still lists are left out.
    private func scan(_ keys: ToRemoteKeys, liveFundingTxids: Set<String>, scanFrom: TimeInterval) async -> Scan {
        var results: [(output: Output, confirmedAt: TimeInterval?)] = []
        var complete = true
        for start in stride(from: 0, to: keys.keys.count, by: Self.scanParallel) {
            let batch = keys.keys[start ..< min(start + Self.scanParallel, keys.keys.count)]
            await withTaskGroup(of: [(output: Output, confirmedAt: TimeInterval?)]?.self) { group in
                for key in batch {
                    group.addTask { await self.utxos(of: key) }
                }
                for await outputs in group {
                    if let outputs { results += outputs } else { complete = false }
                }
            }
        }
        var kept: [(output: Output, confirmedAt: TimeInterval?)] = []
        for result in results {
            if !liveFundingTxids.isEmpty {
                guard let inputs = await (getJSON("/tx/\(result.output.txid)") as? [String: Any])?["vin"]
                    as? [[String: Any]]
                else {
                    complete = false
                    continue
                }
                if inputs.contains(where: { ($0["txid"] as? String).map(liveFundingTxids.contains) == true }) {
                    continue
                }
            }
            kept.append(result)
        }
        return Scan(
            outputs: kept.map(\.output),
            newClose: kept.contains { $0.confirmedAt.map { $0 >= scanFrom - Self.clockSlackSeconds } ?? true },
            complete: complete
        )
    }

    /// Nil when the lookup failed.
    private nonisolated func utxos(of key: ToRemoteKeys.Key) async -> [(output: Output, confirmedAt: TimeInterval?)]? {
        // Esplora's scripthash is the plain SHA-256 of the scriptPubKey.
        let hash = key.scriptPubKey.sha256Hex
        guard let utxos = await getJSON("/scripthash/\(hash)/utxo") as? [[String: Any]] else { return nil }
        return utxos.compactMap { utxo in
            guard let txid = utxo["txid"] as? String,
                  let vout = (utxo["vout"] as? NSNumber)?.uint32Value,
                  let value = (utxo["value"] as? NSNumber)?.uint64Value else { return nil }
            let status = utxo["status"] as? [String: Any]
            let confirmedAt = status?["confirmed"] as? Bool == true
                ? (status?["block_time"] as? NSNumber)?.doubleValue ?? 0
                : nil
            return (Output(txid: txid, vout: vout, valueSats: value, keyIndex: key.index), confirmedAt)
        }
    }

    private func settle(
        _ keys: ToRemoteKeys,
        destination: @Sendable () async -> String?,
        feeRateSatVb: @Sendable () async -> UInt64?
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
                  let feeRate = await feeRateSatVb() else { continue }
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
            await log("TO_REMOTE_SWEEP_BROADCAST", [
                "outpoint": "\(output.txid):\(output.vout)",
                "result": broadcast(tx.hex),
                "sats": output.valueSats,
                "fee_rate": feeRate
            ])
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
