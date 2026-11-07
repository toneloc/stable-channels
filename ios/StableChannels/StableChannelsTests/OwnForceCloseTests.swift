import SCForceClose
import SQLite3
@testable import StableChannels
import XCTest

private final class OwnForceCloseMockURLProtocol: URLProtocol {
    static var outspend: String?
    /// The answer of a second explorer, at other.test.
    static var otherOutspend: String?
    /// Whether the explorers have the funding tx.
    static var hasFunding = true
    static var posted: [String] = []
    /// The host of each post, in order.
    static var postedHosts: [String] = []
    static var postStatus = 200
    static var rejectingHosts: Set<String> = []
    /// Requests arrive on URLSession threads, hence the lock.
    static let lock = NSLock()

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let url = request.url!
        var status = 200
        var body = ""
        Self.lock.lock()
        if request.httpMethod == "POST" {
            let data = request.httpBody ?? request.httpBodyStream.map(Self.read) ?? Data()
            Self.posted.append(String(decoding: data, as: UTF8.self))
            Self.postedHosts.append(url.host ?? "")
            if Self.rejectingHosts.contains(url.host ?? "") {
                status = 400
                body = "min relay fee not met"
            } else {
                status = Self.postStatus
                body = "txid"
            }
        } else if url.path.hasSuffix("/outspend/1"),
                  let outspend = url.host == "other.test" ? Self.otherOutspend : Self.outspend {
            body = outspend
        } else if url.path.hasSuffix("/hex") {
            status = Self.hasFunding ? 200 : 404
            body = Self.hasFunding ? "0200" : "Transaction not found"
        } else {
            status = 503
        }
        Self.lock.unlock()
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

final class OwnForceCloseTests: XCTestCase {
    private let funding = String(repeating: "aa", count: 32)
    private let close = ForceCloseTransactions(
        closeTxid: "c1",
        closeTxHex: "c1hex",
        anchorChildTxid: "b1",
        anchorChildHex: "b1hex"
    )
    private var defaults: UserDefaults!
    private let clock = Clock()
    private let events = Events()

    private final class Clock: @unchecked Sendable {
        var now: TimeInterval = 1_000_000
    }

    private final class Events: @unchecked Sendable {
        var logged: [(String, [String: Any])] = []
        var builds = 0
        var buildError: Error?
        var confirmed: [String] = []
    }

    private struct BuildFailed: Error {}

    override func setUp() {
        super.setUp()
        OwnForceCloseMockURLProtocol.outspend = "{\"spent\":false}"
        OwnForceCloseMockURLProtocol.otherOutspend = nil
        OwnForceCloseMockURLProtocol.hasFunding = true
        OwnForceCloseMockURLProtocol.posted = []
        OwnForceCloseMockURLProtocol.postedHosts = []
        OwnForceCloseMockURLProtocol.postStatus = 200
        OwnForceCloseMockURLProtocol.rejectingHosts = []
        defaults = UserDefaults(suiteName: "OwnForceCloseTests-\(UUID().uuidString)")
    }

    private var session: URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [OwnForceCloseMockURLProtocol.self]
        return URLSession(configuration: config)
    }

    private var chainURLs = ["https://esplora.test/api"]

    private func ownForceClose() -> OwnForceClose {
        let clock = clock
        let events = events
        return OwnForceClose(
            session: session,
            chainURLs: chainURLs,
            defaults: defaults,
            now: { clock.now },
            log: { event, data in events.logged.append((event, data)) }
        )
    }

    @discardableResult
    private func runNow(_ forceClose: OwnForceClose) async -> String? {
        let events = events
        let close = close
        return await forceClose.step(
            build: { _ in
                events.builds += 1
                if let error = events.buildError { throw error }
                return close
            },
            onConfirmed: { paymentId, txid in events.confirmed.append("\(paymentId) \(txid)") }
        )
    }

    private func start(_ forceClose: OwnForceClose) async {
        await forceClose.start(channelId: "ch", paymentId: "close-1", fundingTxid: funding, fundingVout: 1)
    }

    private var posted: [String] {
        OwnForceCloseMockURLProtocol.lock.lock()
        defer { OwnForceCloseMockURLProtocol.lock.unlock() }
        return OwnForceCloseMockURLProtocol.posted
    }

    private var postedHosts: [String] {
        OwnForceCloseMockURLProtocol.lock.lock()
        defer { OwnForceCloseMockURLProtocol.lock.unlock() }
        return OwnForceCloseMockURLProtocol.postedHosts
    }

    func testBroadcastsTheCloseThenTheFeeBumpWhenNoCloseShowsUp() async {
        let forceClose = ownForceClose()
        await start(forceClose)

        let message = await runNow(forceClose)

        XCTAssertEqual(events.builds, 1)
        XCTAssertEqual(posted, ["c1hex", "b1hex"])
        XCTAssertEqual(message, "Close sent.")
        let state = await forceClose.state
        XCTAssertEqual(state?.closeHex, "c1hex")
        XCTAssertEqual(state?.feeBumpHex, "b1hex")
        XCTAssertEqual(state?.broadcastAt, clock.now)
        XCTAssertEqual(state?.nextAt, clock.now + OwnForceClose.checkSeconds)
        XCTAssertEqual(events.logged.map(\.0), ["OWN_FORCE_CLOSE_BROADCAST"])
    }

    func testBroadcastsAgainWithoutBuildingAgainWhileNoCloseShowsUp() async {
        let forceClose = ownForceClose()
        await start(forceClose)
        await runNow(forceClose)
        clock.now += OwnForceClose.checkSeconds
        await runNow(forceClose)
        XCTAssertEqual(posted, ["c1hex", "b1hex"])
        clock.now += OwnForceClose.retrySeconds

        await runNow(forceClose)

        XCTAssertEqual(events.builds, 1)
        XCTAssertEqual(posted, ["c1hex", "b1hex", "c1hex", "b1hex"])
    }

    func testWaitsWhileACloseIsInTheMempool() async {
        OwnForceCloseMockURLProtocol.outspend =
            "{\"spent\":true,\"txid\":\"lsp\",\"vin\":0,\"status\":{\"confirmed\":false}}"
        let forceClose = ownForceClose()
        await start(forceClose)

        let message = await runNow(forceClose)

        XCTAssertNil(message)
        XCTAssertEqual(events.builds, 1)
        XCTAssertEqual(posted, [])
        let nextAt = await forceClose.state?.nextAt
        XCTAssertEqual(nextAt, clock.now + OwnForceClose.checkSeconds)
    }

    func testStopsOnceACloseConfirms() async {
        OwnForceCloseMockURLProtocol.outspend =
            "{\"spent\":true,\"txid\":\"lsp\",\"vin\":0,\"status\":{\"confirmed\":true}}"
        let forceClose = ownForceClose()
        await start(forceClose)

        await runNow(forceClose)

        let state = await forceClose.state
        XCTAssertNil(state)
        XCTAssertNil(defaults.data(forKey: "own_force_close"))
        XCTAssertEqual(events.logged.map(\.0), ["OWN_FORCE_CLOSE_DONE"])
        XCTAssertEqual(events.logged.first?.1["close_txid"] as? String, "lsp")
        XCTAssertEqual(events.confirmed, ["close-1 lsp"])
    }

    func testWaitsWhileNoExplorerAnswers() async {
        OwnForceCloseMockURLProtocol.outspend = nil
        let forceClose = ownForceClose()
        await start(forceClose)

        let message = await runNow(forceClose)

        XCTAssertNil(message)
        // Building needs no explorer.
        XCTAssertEqual(events.builds, 1)
        XCTAssertEqual(posted, [])
        let nextAt = await forceClose.state?.nextAt
        XCTAssertEqual(nextAt, clock.now + OwnForceClose.checkSeconds)
    }

    func testWaitsWhileNoExplorerHasTheFunding() async {
        // Esplora reports an output of a tx it does not have as unspent.
        OwnForceCloseMockURLProtocol.hasFunding = false
        let forceClose = ownForceClose()
        await start(forceClose)

        let message = await runNow(forceClose)

        XCTAssertNil(message)
        XCTAssertEqual(posted, [])
        let nextAt = await forceClose.state?.nextAt
        XCTAssertEqual(nextAt, clock.now + OwnForceClose.checkSeconds)
    }

    func testTriesAgainLaterWhenTheCloseCannotBeBuilt() async {
        events.buildError = BuildFailed()
        let forceClose = ownForceClose()
        await start(forceClose)

        let message = await runNow(forceClose)

        XCTAssertEqual(message, "Close not created yet. Retrying in 10 minutes.")
        let state = await forceClose.state
        XCTAssertNil(state?.closeHex)
        XCTAssertEqual(state?.nextAt, clock.now + OwnForceClose.retrySeconds)
        XCTAssertEqual(events.logged.map(\.0), ["OWN_FORCE_CLOSE_FAILED"])
    }

    func testSaysSoWhenNoExplorerAcceptsTheClose() async {
        OwnForceCloseMockURLProtocol.postStatus = 400
        let forceClose = ownForceClose()
        await start(forceClose)

        let message = await runNow(forceClose)

        XCTAssertEqual(message, "Close not accepted yet. Retrying in 10 minutes.")
    }

    func testOffersASpeedUpOnlyWhileOurCloseIsSlow() async {
        let forceClose = ownForceClose()
        await start(forceClose)
        await runNow(forceClose)
        let sentAt = clock.now
        // No explorer has our close yet: nothing to speed up.
        var state = await forceClose.state
        XCTAssertEqual(state?.isSlow(now: sentAt + OwnForceClose.slowSeconds), false)

        OwnForceCloseMockURLProtocol.outspend =
            "{\"spent\":true,\"txid\":\"c1\",\"vin\":0,\"status\":{\"confirmed\":false}}"
        clock.now += OwnForceClose.checkSeconds
        await runNow(forceClose)
        // Timed from when an explorer first showed our close.
        let seenAt = clock.now
        clock.now += OwnForceClose.checkSeconds
        await runNow(forceClose)
        state = await forceClose.state
        XCTAssertEqual(state?.isSlow(now: seenAt + OwnForceClose.slowSeconds - 1), false)
        XCTAssertEqual(state?.isSlow(now: seenAt + OwnForceClose.slowSeconds), true)

        // The LSP's close replaced ours in the mempool: speeding ours up can't help.
        OwnForceCloseMockURLProtocol.outspend =
            "{\"spent\":true,\"txid\":\"lsp\",\"vin\":0,\"status\":{\"confirmed\":false}}"
        clock.now += OwnForceClose.checkSeconds
        await runNow(forceClose)
        state = await forceClose.state
        XCTAssertEqual(state?.isSlow(now: seenAt + OwnForceClose.slowSeconds), false)
    }

    func testOffersASpeedUpWhileOnlySomeExplorersHaveOurClose() async {
        chainURLs.append("https://other.test/api")
        OwnForceCloseMockURLProtocol.otherOutspend =
            "{\"spent\":true,\"txid\":\"c1\",\"vin\":0,\"status\":{\"confirmed\":false}}"
        let forceClose = ownForceClose()
        await start(forceClose)

        // The explorer without our close gets it again; the one that has it doesn't.
        let first = await runNow(forceClose)
        let seenAt = clock.now
        clock.now += OwnForceClose.retrySeconds
        let second = await runNow(forceClose)

        XCTAssertNil(first)
        XCTAssertNil(second)
        XCTAssertEqual(posted, ["c1hex", "b1hex", "c1hex", "b1hex"])
        XCTAssertEqual(postedHosts, Array(repeating: "esplora.test", count: 4))
        let state = await forceClose.state
        XCTAssertEqual(state?.isSlow(now: seenAt + OwnForceClose.slowSeconds), true)
    }

    func testStopsAfterTheCloseCannotBeBuiltThreeTimes() async {
        events.buildError = BuildFailed()
        let forceClose = ownForceClose()
        await start(forceClose)
        for _ in 1 ..< OwnForceClose.maxBuildFailures {
            let message = await runNow(forceClose)
            XCTAssertEqual(message, "Close not created yet. Retrying in 10 minutes.")
            clock.now += OwnForceClose.retrySeconds
        }

        let message = await runNow(forceClose)

        XCTAssertEqual(message, "Close could not be created. Funds arrive once the provider closes the channel.")
        XCTAssertEqual(events.builds, OwnForceClose.maxBuildFailures)
        let state = await forceClose.state
        XCTAssertNil(state)
        XCTAssertNil(defaults.data(forKey: "own_force_close"))
    }

    func testSendsOurCloseToTheExplorersThatSeeTheFundingUnspent() async {
        chainURLs.append("https://other.test/api")
        OwnForceCloseMockURLProtocol.otherOutspend =
            "{\"spent\":true,\"txid\":\"x\",\"vin\":0,\"status\":{\"confirmed\":false}}"
        let forceClose = ownForceClose()
        await start(forceClose)

        let message = await runNow(forceClose)

        // Another explorer already has a close of the channel, so there is nothing new to tell.
        XCTAssertNil(message)
        XCTAssertEqual(events.builds, 1)
        XCTAssertEqual(postedHosts, ["esplora.test", "esplora.test"])
        let state = await forceClose.state
        XCTAssertEqual(state?.spendTxid, "x")
    }

    func testConfirmsOnlyWhenEveryExplorerThatAnswersAgrees() async {
        chainURLs.append("https://other.test/api")
        let confirmed = "{\"spent\":true,\"txid\":\"lsp\",\"vin\":0,\"status\":{\"confirmed\":true}}"
        OwnForceCloseMockURLProtocol.outspend = confirmed
        OwnForceCloseMockURLProtocol.otherOutspend =
            "{\"spent\":true,\"txid\":\"lsp\",\"vin\":0,\"status\":{\"confirmed\":false}}"
        let forceClose = ownForceClose()
        await start(forceClose)

        await runNow(forceClose)

        let state = await forceClose.state
        XCTAssertEqual(state?.spendTxid, "lsp")
        XCTAssertEqual(events.confirmed, [])

        OwnForceCloseMockURLProtocol.otherOutspend = confirmed
        clock.now += OwnForceClose.checkSeconds
        await runNow(forceClose)

        let done = await forceClose.state
        XCTAssertNil(done)
        XCTAssertEqual(events.confirmed, ["close-1 lsp"])
    }

    func testKeepsAForceCloseStartedDuringAStep() async {
        let forceClose = ownForceClose()
        await start(forceClose)
        let close = close

        await forceClose.step(build: { _ in
            await forceClose.start(
                channelId: "ch2",
                paymentId: "close-2",
                fundingTxid: String(repeating: "bb", count: 32),
                fundingVout: 0
            )
            return close
        })

        let state = await forceClose.state
        XCTAssertEqual(state?.channelId, "ch2")
        XCTAssertNil(state?.closeHex)
    }

    func testAClearStopsFollowingTheClose() async {
        let forceClose = ownForceClose()
        await start(forceClose)
        await runNow(forceClose)

        await forceClose.clear()
        clock.now += OwnForceClose.retrySeconds

        let state = await forceClose.state
        XCTAssertNil(state)
        XCTAssertNil(defaults.data(forKey: "own_force_close"))
        let isDue = await forceClose.isDue
        XCTAssertFalse(isDue)
        let message = await runNow(forceClose)
        XCTAssertNil(message)
        XCTAssertEqual(events.builds, 1)
        XCTAssertEqual(posted, ["c1hex", "b1hex"])
    }

    func testAStepInProgressSavesNothingAfterAClear() async {
        let forceClose = ownForceClose()
        await start(forceClose)
        let close = close

        // The restore lands while the step builds the close.
        await forceClose.step(build: { _ in
            await forceClose.clear()
            return close
        })

        let state = await forceClose.state
        XCTAssertNil(state)
        XCTAssertNil(defaults.data(forKey: "own_force_close"))
        XCTAssertEqual(posted, [])
    }

    func testKeepsTheBuiltCloseAcrossRestarts() async {
        let first = ownForceClose()
        await start(first)
        await runNow(first)

        let state = await ownForceClose().state

        XCTAssertEqual(state?.closeHex, "c1hex")
        XCTAssertEqual(state?.feeBumpTxid, "b1")
        XCTAssertEqual(state?.paymentId, "close-1")
    }

    func testReadsOnlyMonitorsAndTheirUpdates() throws {
        let dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("OwnForceClose-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        var db: OpaquePointer?
        XCTAssertEqual(sqlite3_open(dataDir.appendingPathComponent("ldk_node_data.sqlite").path, &db), SQLITE_OK)
        sqlite3_exec(
            db,
            """
            CREATE TABLE ldk_node_data (primary_namespace TEXT NOT NULL, secondary_namespace TEXT NOT NULL,
                key TEXT NOT NULL, value BLOB);
            INSERT INTO ldk_node_data VALUES ('monitors', '', 'aa_1', x'0102');
            INSERT INTO ldk_node_data VALUES ('monitor_updates', 'aa_1', '3', x'03');
            INSERT INTO ldk_node_data VALUES ('', '', 'manager', x'04');
            INSERT INTO ldk_node_data VALUES ('archived_monitors', '', 'bb_0', x'05');
            """,
            nil,
            nil,
            nil
        )
        sqlite3_close(db)

        let values = try OwnForceClose.readMonitors(dataDir: dataDir)
            .sorted { $0.primaryNamespace < $1.primaryNamespace }

        XCTAssertEqual(values.map(\.primaryNamespace), ["monitor_updates", "monitors"])
        XCTAssertEqual(values.map(\.secondaryNamespace), ["aa_1", ""])
        XCTAssertEqual(values.map(\.key), ["3", "aa_1"])
        XCTAssertEqual(values.map(\.value), [Data([3]), Data([1, 2])])
    }

    func testBroadcastsToEveryExplorer() async {
        OwnForceCloseMockURLProtocol.rejectingHosts = ["two.test"]
        let urls = ["https://one.test/api", "https://two.test/api"]

        let results = await OwnForceClose.broadcast(session: session, chainURLs: urls, hex: "0200")

        XCTAssertEqual(results, [urls[0]: "txid", urls[1]: "error: 400 min relay fee not met"])
        XCTAssertEqual(posted, ["0200", "0200"])
    }
}
