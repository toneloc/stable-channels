@testable import StableChannels
import XCTest

private final class ToRemoteMockURLProtocol: URLProtocol {
    static var routes: [String: String] = [:]
    /// Paths the explorer says it doesn't have. Any other unknown path is an explorer error.
    static var missing: Set<String> = []
    static var posted: [String] = []
    /// Paths asked for; requests arrive on URLSession threads, hence the lock.
    static var requested: [String] = []
    static var postStatus = 200
    static let lock = NSLock()

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let url = request.url!
        var status = 200
        var body: String
        Self.lock.lock()
        defer { Self.lock.unlock() }
        Self.requested.append(url.path)
        if request.httpMethod == "POST" {
            let data = request.httpBody ?? request.httpBodyStream.map(Self.read) ?? Data()
            Self.posted.append(String(decoding: data, as: UTF8.self))
            status = Self.postStatus
            body = "sweep"
        } else if let route = Self.routes[url.path] {
            body = route
        } else {
            status = Self.missing.contains(url.path) ? 404 : 503
            body = ""
        }
        let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static func read(_ stream: InputStream) -> Data {
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 1024)
        while stream.hasBytesAvailable {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        return data
    }
}

/// Vectors come from LDK 0.2.7 `KeysManager::possible_v2_counterparty_closed_balance_spks` for this
/// mnemonic, with the seed path of ldk-node's builder, and from rust-bitcoin signing the same sweep
/// the way the force close recovery script does. Android checks the same vectors.
final class ToRemoteRecoveryTests: XCTestCase {
    private static let seed = ToRemoteKeys.bip39Seed(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    )
    private static let keys = try! ToRemoteKeys(seed: seed)

    private let funding = String(repeating: "aa", count: 32)
    private let close = "7b0abcb56f5196eaa8ec0db3ddad1e1a7980ca8c428530b95c39eb9380c89243"
    private let ours = "0020976a6b80c817104437bd4024cfd4b44fe495a0b8061bb61998e690b695546811"
    /// The 2-of-2 witness script a channel close spends the funding output with.
    private let multisig = "5221" + "02" + String(repeating: "1", count: 64) + "21" + "03"
        + String(repeating: "2", count: 64) + "52ae"
    private var fundingInput: String {
        "\"vin\":[{\"txid\":\"\(funding)\",\"vout\":0,\"witness\":[\"\",\"30\",\"30\",\"\(multisig)\"]}]"
    }

    private let lspOutput = "{\"scriptpubkey\":\"0014\(String(repeating: "11", count: 20))\",\"value\":5000}"
    private let address = "bc1qmf6gnapjkfg4pg675v7y2l94vtv7s4evevr40l"
    private var defaults: UserDefaults!
    private var clock: TimeInterval = 1_000_000

    override func setUp() {
        super.setUp()
        ToRemoteMockURLProtocol.routes = [:]
        ToRemoteMockURLProtocol.missing = []
        ToRemoteMockURLProtocol.posted = []
        ToRemoteMockURLProtocol.requested = []
        ToRemoteMockURLProtocol.postStatus = 200
        addressRequests = AddressCounter()
        defaults = UserDefaults(suiteName: "ToRemoteRecoveryTests-\(UUID().uuidString)")
    }

    private func recovery() -> ToRemoteRecovery {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [ToRemoteMockURLProtocol.self]
        let clock = clock
        return ToRemoteRecovery(
            session: URLSession(configuration: config),
            chainURLs: ["https://esplora.test/api"],
            defaults: defaults,
            now: { clock },
            log: { _, _ in }
        )
    }

    private final class AddressCounter: @unchecked Sendable {
        var count = 0
    }

    private var addressRequests = AddressCounter()

    private final class SweptLog: @unchecked Sendable {
        var entries: [String] = []
    }

    private var swept = SweptLog()

    private func runNow(_ recovery: ToRemoteRecovery, feeRate: UInt64? = 2) async {
        let address = address
        let counter = addressRequests
        let swept = swept
        await recovery.run(
            seed: Self.seed,
            destination: {
                counter.count += 1
                return address
            },
            feeRateSatVb: { feeRate },
            onSwept: { txid, sats, to in swept.entries.append("\(txid) \(sats) \(to)") }
        )
    }

    private func closeConfirmed() {
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)/outspend/3"] = "{\"spent\":false}"
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)/status"] = "{\"confirmed\":true}"
    }

    /// A new instance reads the saved state, as after an app restart, and sees the moved clock.
    private func later(by seconds: TimeInterval) -> ToRemoteRecovery {
        clock += seconds
        return recovery()
    }

    private func closePaysUs() {
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] =
            "[{\"spent\":true,\"txid\":\"\(close)\",\"vin\":0}]"
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)"] =
            "{\(fundingInput),\"vout\":[\(lspOutput),\(lspOutput),\(lspOutput),{\"scriptpubkey\":\"\(ours)\",\"value\":94467}]}"
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)/outspends"] =
            "[{\"spent\":false},{\"spent\":false},{\"spent\":false},{\"spent\":false}]"
    }

    func testAnchorScriptsMatchLdk() throws {
        let vectors = [
            0: "002089af21e96a1c0e6d5fe67b50345a9a2eabf53ae11ca44892be0cbf3f9035761c",
            1: "0020ebc7c7f178b97ab61b89d7e483287ea3393bea45b8770067dfb9c26f33631634",
            45: ours,
            999: "0020da1b41af0cf998322b743b0a1cf29e9a2326bef3cf32b8bb34cf5fc9e989f9a2"
        ]
        for (index, script) in vectors {
            XCTAssertEqual(Self.keys.match(try XCTUnwrap(Data(hex: script)))?.index, index)
        }
        XCTAssertEqual(Self.keys.keys.count, ToRemoteKeys.count)
        // LDK's P2WPKH kind for key 45, used only by channels without anchors.
        XCTAssertNil(Self.keys.match(try XCTUnwrap(Data(hex: "00144b332e0a32443b65d64b77ccd431acd354591e36"))))
    }

    func testSweepMatchesReference() throws {
        let key = try XCTUnwrap(Self.keys.match(try XCTUnwrap(Data(hex: ours))))
        let destination = try XCTUnwrap(Bech32.segwitScriptPubKey(address, expectedHrp: "bc"))
        XCTAssertEqual(destination, Data(hex: "0014da7489f432b25150a35ea33c457cb562d9e8572c"))
        let tx = try ToRemoteSweep.withFee(
            key: key,
            txid: close,
            vout: 3,
            valueSats: 94467,
            destinationScript: destination,
            feeSats: 400
        )
        XCTAssertEqual(
            tx.hex,
            "020000000001014392c88093eb395cb93085428cca80791a1eadddb30deca8ea96516fb5bc0a7b030000"
                + "00000100000001736f010000000000160014da7489f432b25150a35ea33c457cb562d9e8572c024730"
                + "4402203351178dc8fcb9b020100272739b6d587272c21440e2e8108c272e4945f2348e022019af35ed"
                + "69c1e357be72c7f623de90996f270c1067dc8f82e5c4a20097837e3701252103aa09948dbf86e2915c"
                + "3fa9265709d8ce05cdcc967b82b24179af2981998657cead51b200000000"
        )
        XCTAssertEqual(tx.vsize, 111)
        XCTAssertEqual(tx.txid, "aa1eb195d898f73cff841f257000b4dcdf496d340c7c53d8e7e357b40873e269")
        XCTAssertEqual(tx.amountSats, 94067)
    }

    func testFeeRateSweepPaysAtLeastTheRateAndSkipsDust() throws {
        let key = try XCTUnwrap(Self.keys.match(try XCTUnwrap(Data(hex: ours))))
        let destination = try XCTUnwrap(Data(hex: "0014da7489f432b25150a35ea33c457cb562d9e8572c"))
        let tx = try XCTUnwrap(ToRemoteSweep.build(
            key: key, txid: close, vout: 3, valueSats: 94467, destinationScript: destination, feeRateSatVb: 5
        ))
        XCTAssertTrue(tx.hex.contains("0100000001"))
        XCTAssertNil(try ToRemoteSweep.build(
            key: key, txid: close, vout: 3, valueSats: 800, destinationScript: destination, feeRateSatVb: 5
        ))
    }

    func testFindsTheCloseFromTheFundingTxidAndSweepsItOnceConfirmed() async {
        closePaysUs()
        var recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        let found = await recovery.state.found
        XCTAssertEqual(found, [ToRemoteRecovery.Output(txid: close, vout: 3, valueSats: 94467, keyIndex: 45)])
        let pending = await recovery.pendingSats
        XCTAssertEqual(pending, 94467)

        // Not swept while the close is unconfirmed.
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)/outspend/3"] = "{\"spent\":false}"
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)/status"] = "{\"confirmed\":false}"
        await runNow(recovery)
        XCTAssertTrue(ToRemoteMockURLProtocol.posted.isEmpty)

        ToRemoteMockURLProtocol.routes["/api/tx/\(close)/status"] = "{\"confirmed\":true}"
        clock += ToRemoteRecovery.settleIntervalSeconds
        recovery = self.recovery()
        await runNow(recovery)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.count, 1)
        let sweep = ToRemoteMockURLProtocol.posted.first ?? ""
        XCTAssertTrue(sweep
            .hasPrefix("02000000000101" + "4392c88093eb395cb93085428cca80791a1eadddb30deca8ea96516fb5bc0a7b03000000"))
        XCTAssertTrue(sweep.contains("160014da7489f432b25150a35ea33c457cb562d9e8572c"))

        // Counted until the sweep confirms, then done.
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)/outspend/3"] =
            "{\"spent\":true,\"txid\":\"sweep\",\"status\":{\"confirmed\":true}}"
        clock += ToRemoteRecovery.settleIntervalSeconds
        recovery = self.recovery()
        await runNow(recovery)
        let left = await recovery.pendingSats
        let active = await recovery.state.active
        let needsCheck = await recovery.needsCheck(funding)
        XCTAssertEqual(left, 0)
        XCTAssertFalse(active)
        XCTAssertFalse(needsCheck)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.count, 1)
    }

    func testACloseThatDoesNotPayUsEndsTheCheck() async {
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] =
            "[{\"spent\":true,\"txid\":\"\(close)\",\"vin\":0}]"
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)"] = "{\(fundingInput),\"vout\":[\(lspOutput)]}"
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        let state = await recovery.state
        XCTAssertFalse(state.active)
        XCTAssertEqual(state.checkedFundingTxid, funding)
    }

    func testUnspentFundingIsRetriedLater() async {
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] = "[{\"spent\":false}]"
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        let state = await recovery.state
        XCTAssertTrue(state.active)
        XCTAssertEqual(state.nextAt, clock + ToRemoteRecovery.backoffSeconds(1))
    }

    func testBackoffStartsAtAMinuteThenDoublesUpToADay() {
        XCTAssertEqual(ToRemoteRecovery.backoffSeconds(1), 60)
        XCTAssertEqual(ToRemoteRecovery.backoffSeconds(2), 10 * 60)
        XCTAssertEqual(ToRemoteRecovery.backoffSeconds(3), 20 * 60)
        XCTAssertEqual(ToRemoteRecovery.backoffSeconds(20), 24 * 60 * 60)
    }

    // MARK: - Case B scenarios

    /// Without a restore: LDK lost the channel but the funding txid is still saved.
    func testLostChannelDataIsCheckedOnRefresh() async {
        let recovery = recovery()
        var runs = await recovery.onRefresh(savedFundingTxid: funding, ldkHasChannels: true, ldkHandlesClose: false)
        XCTAssertFalse(runs)
        runs = await recovery.onRefresh(savedFundingTxid: funding, ldkHasChannels: false, ldkHandlesClose: true)
        XCTAssertFalse(runs)
        let active = await recovery.state.active
        XCTAssertFalse(active)

        runs = await recovery.onRefresh(savedFundingTxid: funding, ldkHasChannels: false, ldkHandlesClose: false)
        XCTAssertTrue(runs)
        let armed = await recovery.state.fundingTxid
        XCTAssertEqual(armed, funding)
        closePaysUs()
        await runNow(recovery)
        let pending = await recovery.pendingSats
        XCTAssertEqual(pending, 94467)

        // Never runs while LDK handles a close itself.
        runs = await recovery.onRefresh(savedFundingTxid: nil, ldkHasChannels: false, ldkHandlesClose: true)
        XCTAssertFalse(runs)
        runs = await recovery.onRefresh(savedFundingTxid: nil, ldkHasChannels: false, ldkHandlesClose: false)
        XCTAssertTrue(runs)
    }

    /// iOS saves no funding vout, so every spent output of the funding tx is checked.
    func testWithoutAVoutEverySpentOutputIsChecked() async {
        closePaysUs()
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] =
            "[{\"spent\":true,\"txid\":\"other\"},{\"spent\":true,\"txid\":\"\(close)\"}]"
        ToRemoteMockURLProtocol.routes["/api/tx/other"] = "{\"vout\":[\(lspOutput)]}"
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        let pending = await recovery.pendingSats
        XCTAssertEqual(pending, 94467)
    }

    func testACheckedFundingTxidIsNotLookedUpAgain() async {
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] =
            "[{\"spent\":true,\"txid\":\"\(close)\",\"vin\":0}]"
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)"] = "{\(fundingInput),\"vout\":[\(lspOutput)]}"
        let recovery = recovery()
        var runs = await recovery.onRefresh(savedFundingTxid: funding, ldkHasChannels: false, ldkHandlesClose: false)
        XCTAssertTrue(runs)
        await runNow(recovery)
        let requests = ToRemoteMockURLProtocol.requested.count
        runs = await recovery.onRefresh(savedFundingTxid: funding, ldkHasChannels: false, ldkHandlesClose: false)
        XCTAssertFalse(runs)
        await runNow(recovery)
        XCTAssertEqual(ToRemoteMockURLProtocol.requested.count, requests)
    }

    func testExplorerDownKeepsTheSearchGoing() async {
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        let state = await recovery.state
        XCTAssertEqual(state.fundingTxid, funding)
        XCTAssertEqual(state.attempts, 1)
        XCTAssertEqual(state.nextAt, clock + ToRemoteRecovery.backoffSeconds(1))
    }

    func testSearchGivesUpAfterTwoWeeks() async {
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] = "[{\"spent\":false}]"
        var recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        var active = await recovery.state.active
        XCTAssertTrue(active)
        recovery = later(by: ToRemoteRecovery.searchSeconds)
        await runNow(recovery)
        active = await recovery.state.active
        let checked = await recovery.state.checkedFundingTxid
        XCTAssertFalse(active)
        XCTAssertEqual(checked, funding)
    }

    func testFailedBroadcastIsRetriedWithTheSameAddress() async {
        closePaysUs()
        var recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        closeConfirmed()
        ToRemoteMockURLProtocol.postStatus = 400
        await runNow(recovery)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.count, 1)
        let pending = await recovery.pendingSats
        XCTAssertEqual(pending, 94467)

        ToRemoteMockURLProtocol.postStatus = 200
        recovery = later(by: ToRemoteRecovery.settleIntervalSeconds)
        await runNow(recovery)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.count, 2)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.first, ToRemoteMockURLProtocol.posted.last)
        XCTAssertEqual(addressRequests.count, 1)
    }

    /// History gets the close from the sweep, as LDK never reports it (#364).
    func testOnlyABroadcastSweepIsReported() async throws {
        closePaysUs()
        var recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        closeConfirmed()
        ToRemoteMockURLProtocol.postStatus = 400
        await runNow(recovery)
        XCTAssertTrue(swept.entries.isEmpty)

        ToRemoteMockURLProtocol.postStatus = 200
        recovery = later(by: ToRemoteRecovery.settleIntervalSeconds)
        await runNow(recovery)
        let key = try XCTUnwrap(Self.keys.match(try XCTUnwrap(Data(hex: ours))))
        let destination = try XCTUnwrap(Bech32.segwitScriptPubKey(address, expectedHrp: "bc"))
        let sweep = try XCTUnwrap(ToRemoteSweep.build(
            key: key,
            txid: close,
            vout: 3,
            valueSats: 94467,
            destinationScript: destination,
            feeRateSatVb: 2
        ))
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.last, sweep.hex)
        XCTAssertEqual(swept.entries, ["\(sweep.txid) \(sweep.amountSats) \(address)"])
    }

    func testNoFeeRateMeansNoBroadcastYet() async {
        closePaysUs()
        var recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        closeConfirmed()
        await runNow(recovery, feeRate: nil)
        XCTAssertTrue(ToRemoteMockURLProtocol.posted.isEmpty)
        recovery = later(by: ToRemoteRecovery.settleIntervalSeconds)
        await runNow(recovery)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.count, 1)
    }

    func testDustIsDroppedWithoutABroadcast() async {
        closePaysUs()
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)"] =
            "{\(fundingInput),\"vout\":[\(lspOutput),\(lspOutput),\(lspOutput),{\"scriptpubkey\":\"\(ours)\",\"value\":400}]}"
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        let pending = await recovery.pendingSats
        XCTAssertEqual(pending, 400)
        closeConfirmed()
        await runNow(recovery, feeRate: 5)
        let active = await recovery.state.active
        XCTAssertTrue(ToRemoteMockURLProtocol.posted.isEmpty)
        XCTAssertFalse(active)
    }

    // MARK: - Review fixes

    func testAnOutputTooSmallForTheCurrentFeeWaits() async {
        closePaysUs()
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)"] =
            "{\(fundingInput),\"vout\":[\(lspOutput),\(lspOutput),\(lspOutput),{\"scriptpubkey\":\"\(ours)\",\"value\":800}]}"
        var recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        closeConfirmed()
        await runNow(recovery, feeRate: 5)
        XCTAssertTrue(ToRemoteMockURLProtocol.posted.isEmpty)
        let pending = await recovery.pendingSats
        XCTAssertEqual(pending, 800)

        recovery = later(by: ToRemoteRecovery.settleIntervalSeconds)
        await runNow(recovery, feeRate: 2)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.count, 1)
    }

    func testWithoutAVoutTheChannelOutputIsFoundByItsWitness() async {
        // Output 0 is the LSP's change, still unspent; output 1 is the channel, closed by coop.
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] =
            "[{\"spent\":false},{\"spent\":true,\"txid\":\"\(close)\",\"vin\":0}]"
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)"] =
            "{\"vin\":[{\"txid\":\"\(funding)\",\"vout\":1,\"witness\":[\"\",\"30\",\"30\",\"\(multisig)\"]}],\"vout\":[\(lspOutput)]}"
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        let state = await recovery.state
        XCTAssertFalse(state.active)
        XCTAssertEqual(state.checkedFundingTxid, funding)
    }

    func testWithoutAVoutAnUnrelatedSpendDoesNotEndTheCheck() async {
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] =
            "[{\"spent\":false},{\"spent\":true,\"txid\":\"\(close)\",\"vin\":0}]"
        ToRemoteMockURLProtocol.routes["/api/tx/\(close)"] =
            "{\"vin\":[{\"txid\":\"\(funding)\",\"vout\":1,\"witness\":[\"30\",\"02aa\"]}],\"vout\":[\(lspOutput)]}"
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        let armed = await recovery.state.fundingTxid
        XCTAssertEqual(armed, funding)
    }

    func testAZeroFeeEstimateSweepsAtTheFloor() async {
        closePaysUs()
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        closeConfirmed()
        await runNow(recovery, feeRate: 0)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.count, 1)
    }

    /// After a restore the saved funding txid is checked until the LSP's close shows up.
    func testARestoreWaitsForTheLspCloseThenSweeps() async {
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] = "[{\"spent\":false}]"
        var recovery = recovery()
        await recovery.startAfterRestore(fundingTxid: funding, repeats: true)
        await runNow(recovery)
        var active = await recovery.state.active
        XCTAssertTrue(active)

        // The LSP closes after the restored node reconnects.
        closePaysUs()
        recovery = later(by: ToRemoteRecovery.backoffSeconds(1))
        await runNow(recovery)
        let pending = await recovery.pendingSats
        XCTAssertEqual(pending, 94467)

        closeConfirmed()
        recovery = later(by: ToRemoteRecovery.settleIntervalSeconds)
        await runNow(recovery)
        XCTAssertEqual(ToRemoteMockURLProtocol.posted.count, 1)

        ToRemoteMockURLProtocol.routes["/api/tx/\(close)/outspend/3"] =
            "{\"spent\":true,\"txid\":\"sweep\",\"status\":{\"confirmed\":true}}"
        recovery = later(by: ToRemoteRecovery.settleIntervalSeconds)
        await runNow(recovery)
        active = await recovery.state.active
        XCTAssertFalse(active)
    }

    func testAOneTimeRestoreCheckStopsWhenNothingIsFound() async {
        ToRemoteMockURLProtocol.routes["/api/tx/\(funding)/outspends"] = "[{\"spent\":false}]"
        let recovery = recovery()
        await recovery.startAfterRestore(fundingTxid: funding, repeats: false)
        await runNow(recovery)
        let active = await recovery.state.active
        XCTAssertFalse(active)
    }

    func testAOneTimeRestoreCheckIsRetriedWhileTheExplorerIsDown() async {
        var recovery = recovery()
        await recovery.startAfterRestore(fundingTxid: funding, repeats: false)
        await runNow(recovery)
        let waiting = await recovery.state.fundingTxid
        XCTAssertEqual(waiting, funding)

        closePaysUs()
        recovery = later(by: ToRemoteRecovery.backoffSeconds(1))
        await runNow(recovery)
        let pending = await recovery.pendingSats
        XCTAssertEqual(pending, 94467)
    }

    func testAFundingTxNoExplorerKnowsIsNotSpent() async {
        ToRemoteMockURLProtocol.missing = ["/api/tx/\(funding)/outspends"]
        let recovery = recovery()
        await recovery.startAfterRestore(fundingTxid: funding, repeats: false)
        await runNow(recovery)
        let state = await recovery.state
        XCTAssertFalse(state.active)
        XCTAssertEqual(state.checkedFundingTxid, funding)
    }

    func testARestoreLooksAgainForTheCloseOfAChannelTheAppForgot() async {
        closePaysUs()
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        var pending = await recovery.pendingSats
        XCTAssertEqual(pending, 94467)

        // The app forgets the funding txid when the channel closes, so the restore passes none.
        await recovery.startAfterRestore(fundingTxid: nil, repeats: false)
        let state = await recovery.state
        XCTAssertTrue(state.found.isEmpty)
        XCTAssertEqual(state.fundingTxid, funding)
        await runNow(recovery)
        pending = await recovery.pendingSats
        XCTAssertEqual(pending, 94467)
    }

    func testARestoreWithoutAFundingTxidLooksForNothing() async {
        let recovery = recovery()
        await recovery.startAfterRestore(fundingTxid: nil, repeats: true)
        await runNow(recovery)
        let active = await recovery.state.active
        XCTAssertFalse(active)
        XCTAssertTrue(ToRemoteMockURLProtocol.requested.isEmpty)
    }

    func testARestoreStartsOver() async {
        closePaysUs()
        let recovery = recovery()
        await recovery.start(fundingTxid: funding, fundingVout: nil)
        await runNow(recovery)
        closeConfirmed()
        await runNow(recovery)
        var state = await recovery.state
        XCTAssertEqual(state.destination, address)

        let other = String(repeating: "dd", count: 32)
        await recovery.startAfterRestore(fundingTxid: other, repeats: true)
        state = await recovery.state
        XCTAssertTrue(state.found.isEmpty)
        XCTAssertNil(state.destination)
        XCTAssertNil(state.checkedFundingTxid)
        XCTAssertEqual(state.fundingTxid, other)
    }
}
