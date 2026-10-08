import Foundation
import XCTest
@testable import StableChannels

final class MockTxConfirmationProvider: TxConfirmationProvider, BlockHeightProvider {
    var heightMap: [String: UInt32] = [:]
    var mockCurrentHeight: UInt32 = 800_000
    var failingTxids: Set<String> = []
    var currentHeightFails = false
    var lookupDelayNanoseconds: UInt64 = 0
    /// When set, every tx lookup suspends until the stream finishes.
    var lookupGate: AsyncStream<Void>?
    private let lock = NSLock()
    private var _lookupCount = 0
    var lookupCount: Int { lock.withLock { _lookupCount } }

    func blockHeight(for txid: String) async throws -> UInt32? {
        // Snapshot the answer before any gate so a blocked lookup returns the state it started with.
        let height = heightMap[txid]
        let fails = failingTxids.contains(txid)
        lock.withLock { _lookupCount += 1 }
        if let lookupGate {
            for await _ in lookupGate {}
        }
        try Task.checkCancellation()
        if lookupDelayNanoseconds > 0 {
            try await Task.sleep(nanoseconds: lookupDelayNanoseconds)
        }
        if fails {
            throw URLError(.notConnectedToInternet)
        }
        return height
    }

    func currentHeight() async throws -> UInt32 {
        if currentHeightFails {
            throw URLError(.notConnectedToInternet)
        }
        return mockCurrentHeight
    }
}

@MainActor
final class SPVHeaderChainServiceTests: XCTestCase {
    var db: DatabaseService!
    var blockHeightService: BlockHeightService!
    var confirmationService: ConfirmationService!
    var confirmationPollingService: ConfirmationPollingService!
    var spvService: SPVHeaderChainService!
    var mockProvider: MockTxConfirmationProvider!
    var dataDir: URL!

    override func setUpWithError() throws {
        try super.setUpWithError()
        let documents = try FileManager.default.url(
            for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        dataDir = documents.appendingPathComponent("test_spv_\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)

        db = try DatabaseService(dataDir: dataDir)

        mockProvider = MockTxConfirmationProvider()
        blockHeightService = BlockHeightService(provider: mockProvider)
        confirmationService = ConfirmationService(provider: mockProvider)
        confirmationPollingService = ConfirmationPollingService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationService: confirmationService
        )
        spvService = SPVHeaderChainService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationPollingService: confirmationPollingService
        )
    }

    override func tearDownWithError() throws {
        db = nil
        try? FileManager.default.removeItem(at: dataDir)
        try super.tearDownWithError()
    }

    // MARK: - Test 1: Ordinary Offline Gap

    func testOrdinaryOfflineGapAdvancesTipAndStoresHeader() async throws {
        // Seed initial tip at #100
        let seedBlock = MempoolWSBlock(
            height: 100,
            id: "hash_100",
            previousblockhash: "hash_99",
            timestamp: 1_700_000_000
        )
        await spvService.processBlockHeader(seedBlock)

        let initialTip = try db.fetchLatestHeader()
        XCTAssertEqual(initialTip?.height, 100)
        XCTAssertEqual(initialTip?.hash, "hash_100")

        // Simulate offline gap: app receives block #105 (gap of 5 blocks)
        mockProvider.mockCurrentHeight = 105
        let gapBlock = MempoolWSBlock(
            height: 105,
            id: "hash_105",
            previousblockhash: "hash_104",
            timestamp: 1_700_000_300
        )
        await spvService.processBlockHeader(gapBlock)

        // Verify tip advanced to #105 and header is stored in SQLite
        let newTip = try db.fetchLatestHeader()
        XCTAssertEqual(newTip?.height, 105)
        XCTAssertEqual(newTip?.hash, "hash_105")
        XCTAssertEqual(blockHeightService.currentHeight, 105)
    }

    // MARK: - Test 2: Orphaned Confirmed Payment Downgraded During Gap

    func testOrphanedConfirmedPaymentDowngradedDuringGap() async throws {
        // Record a payment and mark it completed at block height 100
        _ = try db.paymentRepo.recordPayment(
            paymentId: "tx_orphaned_001",
            paymentType: "onchain",
            direction: "received",
            amountMsat: 100_000_000,
            amountUSD: 50.0,
            btcPrice: 50_000,
            counterparty: nil,
            status: "completed",
            txid: "tx_orphaned_001"
        )
        let created = try XCTUnwrap(db.paymentRepo.getRecentPayments(limit: 1).first)
        try db.paymentRepo.updateConfirmations(
            paymentId: created.id,
            txBlockHeight: 100,
            currentBlockHeight: 105
        )

        // Verify payment is completed in SQLite
        var record = try db.paymentRepo.getPayment(byId: created.id)
        XCTAssertEqual(record?.status, "completed")
        XCTAssertEqual(record?.confirmations, 6)
        XCTAssertEqual(record?.txBlockHeight, 100)

        // Update mock Esplora provider: chain tip is at #105, but transaction 'tx_orphaned_001'
        // is no longer returned by Esplora (returns nil = orphaned in a reorg during offline gap)
        mockProvider.mockCurrentHeight = 105
        mockProvider.heightMap["tx_orphaned_001"] = nil

        // Seed initial tip at #100
        let seedBlock = MempoolWSBlock(
            height: 100,
            id: "hash_100",
            previousblockhash: "hash_99",
            timestamp: 1_700_000_000
        )
        await spvService.processBlockHeader(seedBlock)

        // Process offline gap block #105
        let gapBlock = MempoolWSBlock(
            height: 105,
            id: "hash_105",
            previousblockhash: "hash_104",
            timestamp: 1_700_000_300
        )
        await spvService.processBlockHeader(gapBlock)

        // Verify that the orphaned payment was downgraded to 'pending' with 0 confirmations
        record = try db.paymentRepo.getPayment(byId: created.id)
        XCTAssertEqual(record?.status, "pending")
        XCTAssertEqual(record?.confirmations, 0)
        XCTAssertNil(record?.txBlockHeight)
    }
}

// MARK: - Manual Confirmation Refresh (History pull-to-refresh)

@MainActor
final class ConfirmationPollingRefreshTests: XCTestCase {
    var db: DatabaseService!
    var blockHeightService: BlockHeightService!
    var pollingService: ConfirmationPollingService!
    var mockProvider: MockTxConfirmationProvider!
    var dataDir: URL!
    var updateCount = 0

    override func setUpWithError() throws {
        try super.setUpWithError()
        dataDir = try FileManager.default.url(
            for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        .appendingPathComponent("test_refresh_\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        db = try DatabaseService(dataDir: dataDir)
        mockProvider = MockTxConfirmationProvider()
        blockHeightService = BlockHeightService(provider: mockProvider)
        pollingService = ConfirmationPollingService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationService: ConfirmationService(provider: mockProvider)
        )
        updateCount = 0
        pollingService.onUpdate = { [weak self] in self?.updateCount += 1 }
    }

    override func tearDownWithError() throws {
        db = nil
        try? FileManager.default.removeItem(at: dataDir)
        try super.tearDownWithError()
    }

    private func recordPendingPayment(txid: String) throws -> Int64 {
        _ = try db.paymentRepo.recordPayment(
            paymentId: txid,
            paymentType: "onchain",
            direction: "received",
            amountMsat: 100_000_000,
            amountUSD: 50.0,
            btcPrice: 50_000,
            counterparty: nil,
            status: "pending",
            txid: txid
        )
        return try XCTUnwrap(db.paymentRepo.getRecentPayments(limit: 1).first).id
    }

    func testRefreshFetchesTipAndUpdatesConfirmations() async throws {
        let id = try recordPendingPayment(txid: "tx_refresh_ok")
        mockProvider.mockCurrentHeight = 105
        mockProvider.heightMap["tx_refresh_ok"] = 100

        let result = try await pollingService.refresh()

        XCTAssertEqual(result, .completed(failedLookups: 0))
        XCTAssertNil(result.errorMessage)
        XCTAssertEqual(blockHeightService.currentHeight, 105)
        XCTAssertEqual(try db.paymentRepo.getPayment(byId: id)?.confirmations, 6)
        XCTAssertEqual(updateCount, 1)
    }

    func testRefreshReportsChainTipFailure() async throws {
        _ = try recordPendingPayment(txid: "tx_tip_fail")
        mockProvider.currentHeightFails = true

        let result = try await pollingService.refresh()

        XCTAssertEqual(result, .chainTipUnavailable)
        XCTAssertNotNil(result.errorMessage)
        XCTAssertEqual(mockProvider.lookupCount, 0)
    }

    func testRefreshReportsFailedTransactionLookups() async throws {
        _ = try recordPendingPayment(txid: "tx_lookup_ok")
        _ = try recordPendingPayment(txid: "tx_lookup_fail")
        mockProvider.mockCurrentHeight = 105
        mockProvider.heightMap["tx_lookup_ok"] = 100
        mockProvider.failingTxids = ["tx_lookup_fail"]

        let result = try await pollingService.refresh()

        XCTAssertEqual(result, .completed(failedLookups: 1))
        XCTAssertNotNil(result.errorMessage)
        XCTAssertEqual(updateCount, 1)
    }

    func testRefreshWaitsForInFlightPollThenRunsFreshPass() async throws {
        let id = try recordPendingPayment(txid: "tx_overlap")
        mockProvider.mockCurrentHeight = 105
        blockHeightService.setHeightSilently(105)
        let (gate, release) = AsyncStream<Void>.makeStream()
        mockProvider.lookupGate = gate

        // Automatic poll starts and blocks inside its tx lookup (tx still unconfirmed).
        let poll = Task { await pollingService.pollOnce() }
        while mockProvider.lookupCount == 0 {
            await Task.yield()
        }

        var refreshFinished = false
        let refresh = Task { () -> ConfirmationRefreshResult in
            let result = try await pollingService.refresh()
            refreshFinished = true
            return result
        }
        for _ in 0..<20 {
            await Task.yield()
        }
        XCTAssertFalse(refreshFinished, "refresh must wait for the in-flight poll, not skip or overlap")
        XCTAssertEqual(mockProvider.lookupCount, 1)

        // The tx confirms while the automatic poll is still in flight; only a fresh pass sees it.
        mockProvider.heightMap["tx_overlap"] = 100
        release.finish()
        await poll.value
        let result = try await refresh.value

        XCTAssertEqual(result, .completed(failedLookups: 0))
        XCTAssertEqual(mockProvider.lookupCount, 2)
        XCTAssertEqual(try db.paymentRepo.getPayment(byId: id)?.confirmations, 6)
    }

    func testRefreshUsesManualProvidersWhenConfigured() async throws {
        let id = try recordPendingPayment(txid: "tx_manual")
        // The automatic providers are offline; the manual (fast-fail) ones answer.
        mockProvider.currentHeightFails = true
        mockProvider.failingTxids = ["tx_manual"]
        let manualProvider = MockTxConfirmationProvider()
        manualProvider.mockCurrentHeight = 105
        manualProvider.heightMap["tx_manual"] = 100
        pollingService = ConfirmationPollingService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationService: ConfirmationService(provider: mockProvider),
            manualTipProvider: manualProvider,
            manualConfirmationService: ConfirmationService(provider: manualProvider)
        )

        let result = try await pollingService.refresh()

        XCTAssertEqual(result, .completed(failedLookups: 0))
        XCTAssertEqual(blockHeightService.currentHeight, 105)
        XCTAssertEqual(mockProvider.lookupCount, 0)
        XCTAssertEqual(try db.paymentRepo.getPayment(byId: id)?.confirmations, 6)
    }

    /// Offline: a real single-attempt resolver must give up promptly rather than retrying with backoff.
    func testRefreshFailsFastWhenOffline() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        // A nil handler makes every request fail with a URLError, like having no network.
        MockURLProtocol.requestHandler = nil
        let session = URLSession(configuration: config)
        pollingService = ConfirmationPollingService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationService: ConfirmationService(provider: mockProvider),
            manualTipProvider: BlockHeightResolver(
                chainURLs: ["https://primary.local", "https://fallback.local"],
                urlSession: session,
                maxAttempts: 1
            )
        )

        let start = Date()
        let result = try await pollingService.refresh()

        XCTAssertEqual(result, .chainTipUnavailable)
        XCTAssertLessThan(Date().timeIntervalSince(start), 2)
    }

    func testStaleManualTipLeavesRowsAndTipUntouched() async throws {
        let id = try recordPendingPayment(txid: "stale_tip")
        blockHeightService.setHeightSilently(110)
        mockProvider.mockCurrentHeight = 105
        mockProvider.heightMap["stale_tip"] = 100
        for useManualProvider in [false, true] {
            pollingService = makePollingService(timeout: .seconds(1), manual: useManualProvider)
            let result = try await pollingService.refresh()
            XCTAssertEqual(result, .chainTipUnavailable)
            XCTAssertEqual(blockHeightService.currentHeight, 110)
            XCTAssertEqual(mockProvider.lookupCount, 0)
            XCTAssertEqual(try db.paymentRepo.getPayment(byId: id)?.confirmations, 0)
            XCTAssertNil(try db.paymentRepo.getPayment(byId: id)?.txBlockHeight)
        }
    }

    func testCancellationWhileWaitingDoesNotCancelAutomaticPoll() async throws {
        _ = try recordPendingPayment(txid: "waiting_cancel")
        blockHeightService.setHeightSilently(105)
        let (gate, release) = AsyncStream<Void>.makeStream()
        mockProvider.lookupGate = gate
        let poll = Task { await pollingService.pollOnce() }
        while mockProvider.lookupCount == 0 {
            await Task.yield()
        }
        let refresh = Task { try await pollingService.refresh() }
        let survivingRefresh = Task { try await pollingService.refresh() }
        try await Task.sleep(for: .milliseconds(20))
        let start = ContinuousClock.now
        refresh.cancel()
        do {
            _ = try await refresh.value
            XCTFail("Expected CancellationError")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
        XCTAssertLessThan(start.duration(to: .now), .seconds(1))
        XCTAssertFalse(poll.isCancelled)
        // The cancelled waiter must not release the automatic pass's ownership.
        await pollingService.pollOnce()
        XCTAssertEqual(mockProvider.lookupCount, 1)
        release.finish()
        await poll.value
        let result = try await survivingRefresh.value
        XCTAssertEqual(result, .completed(failedLookups: 0))
    }

    func testDeadlineIncludesWaitAndAllPendingRows() async throws {
        _ = try recordPendingPayment(txid: "deadline_one")
        _ = try recordPendingPayment(txid: "deadline_two")
        blockHeightService.setHeightSilently(105)
        mockProvider.mockCurrentHeight = 105
        let (gate, release) = AsyncStream<Void>.makeStream()
        let automaticProvider = MockTxConfirmationProvider()
        automaticProvider.lookupGate = gate
        mockProvider.lookupDelayNanoseconds = 300_000_000
        pollingService = ConfirmationPollingService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationService: ConfirmationService(provider: automaticProvider),
            manualTipProvider: mockProvider,
            manualConfirmationService: ConfirmationService(provider: mockProvider),
            manualRefreshTimeout: .seconds(1)
        )
        let poll = Task { await pollingService.pollOnce() }
        while automaticProvider.lookupCount == 0 {
            await Task.yield()
        }
        let start = ContinuousClock.now
        let refresh = Task { try await pollingService.refresh() }
        try await Task.sleep(for: .milliseconds(600))
        release.finish()
        let result = try await refresh.value
        XCTAssertEqual(result, .timedOut)
        XCTAssertNotNil(result.errorMessage)
        XCTAssertLessThan(start.duration(to: .now), .milliseconds(1300))
        XCTAssertEqual(mockProvider.lookupCount, 2, "Deadline spans waiting plus multiple pending lookups")
        await poll.value
        mockProvider.lookupDelayNanoseconds = 0
        let retry = try await pollingService.refresh()
        XCTAssertEqual(retry, .completed(failedLookups: 0))
    }

    func testTimeoutWhileWaitingLeavesAutomaticPassRunning() async throws {
        _ = try recordPendingPayment(txid: "waiting_timeout")
        blockHeightService.setHeightSilently(105)
        pollingService = makePollingService(timeout: .milliseconds(100))
        let (gate, release) = AsyncStream<Void>.makeStream()
        mockProvider.lookupGate = gate
        let poll = Task { await pollingService.pollOnce() }
        while mockProvider.lookupCount == 0 {
            await Task.yield()
        }
        let result = try await pollingService.refresh()
        XCTAssertEqual(result, .timedOut)
        await pollingService.pollOnce()
        XCTAssertEqual(mockProvider.lookupCount, 1)
        release.finish()
        await poll.value
    }

    func testDeadlineCancelsRealResolverRequestAndReleasesPass() async throws {
        let ids = try (0..<3).map { try recordPendingPayment(txid: "slow_\($0)") }
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [HangingConfirmationURLProtocol.self]
        let requestStopped = expectation(description: "Deadline cancels the underlying request")
        requestStopped.assertForOverFulfill = true
        HangingConfirmationURLProtocol.reset(requestStopped: requestStopped)
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel() }
        pollingService = ConfirmationPollingService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationService: ConfirmationService(provider: mockProvider),
            manualTipProvider: BlockHeightResolver(
                chainURLs: ["https://slow.local"],
                urlSession: session,
                maxAttempts: 1
            ),
            manualConfirmationService: ConfirmationService(provider: TxConfirmationResolver(
                chainURLs: ["https://slow.local"], urlSession: session, maxAttempts: 1
            )),
            manualRefreshTimeout: .milliseconds(150)
        )
        let start = ContinuousClock.now
        let result = try await pollingService.refresh()
        XCTAssertEqual(result, .timedOut)
        XCTAssertLessThan(start.duration(to: .now), .seconds(1))
        XCTAssertEqual(HangingConfirmationURLProtocol.startedCount, 1)
        // URLSession can resume data(for:) before its protocol queue delivers stopLoading.
        await fulfillment(of: [requestStopped], timeout: 1)
        XCTAssertEqual(HangingConfirmationURLProtocol.stoppedCount, 1)
        for id in ids {
            XCTAssertEqual(try db.paymentRepo.getPayment(byId: id)?.confirmations, 0)
        }
        // A fresh automatic pass proves isPolling was released after cancellation.
        await pollingService.pollOnce()
        XCTAssertEqual(mockProvider.lookupCount, 3)
    }

    func testViewCancellationStopsRealRequestAndThrowsCancellation() async throws {
        _ = try recordPendingPayment(txid: "cancel_network")
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [HangingConfirmationURLProtocol.self]
        let requestStopped = expectation(description: "View cancellation stops the underlying request")
        requestStopped.assertForOverFulfill = true
        HangingConfirmationURLProtocol.reset(requestStopped: requestStopped)
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel() }
        pollingService = ConfirmationPollingService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationService: ConfirmationService(provider: mockProvider),
            manualTipProvider: BlockHeightResolver(
                chainURLs: ["https://slow.local"],
                urlSession: session,
                maxAttempts: 1
            ),
            manualConfirmationService: ConfirmationService(provider: TxConfirmationResolver(
                chainURLs: ["https://slow.local"], urlSession: session, maxAttempts: 1
            ))
        )
        let refresh = Task { try await pollingService.refresh() }
        while HangingConfirmationURLProtocol.startedCount == 0 {
            await Task.yield()
        }
        let start = ContinuousClock.now
        refresh.cancel()
        do {
            _ = try await refresh.value
            XCTFail("Expected CancellationError")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
        XCTAssertLessThan(start.duration(to: .now), .seconds(1))
        await fulfillment(of: [requestStopped], timeout: 1)
        XCTAssertEqual(HangingConfirmationURLProtocol.stoppedCount, 1)
        await pollingService.pollOnce()
        XCTAssertEqual(mockProvider.lookupCount, 1)
    }

    func testHistoryPreservesGoodRowsAndNetworkErrorAcrossReloads() throws {
        let id = try recordPendingPayment(txid: "history_preserve")
        var history = HistoryContent()
        history.reload(database: db)
        history.networkError = ConfirmationRefreshResult.timedOut.errorMessage
        let networkError = history.networkError
        history.reload(database: nil)
        XCTAssertEqual(history.payments.map(\.id), [id])
        XCTAssertNotNil(history.errorMessage)
        history.reload(loadTrades: { [] }, loadPayments: { throw URLError(.unknown) })
        XCTAssertEqual(history.payments.map(\.id), [id])
        XCTAssertNotNil(history.loadError)
        history.reload(database: db)
        XCTAssertNil(history.loadError)
        XCTAssertEqual(history.errorMessage, networkError)
        history.networkError = ConfirmationRefreshResult.completed(failedLookups: 0).errorMessage
        XCTAssertNil(history.errorMessage)
        history.networkError = networkError
        history.clearErrors()
        XCTAssertNil(history.errorMessage)
        XCTAssertEqual(history.payments.map(\.id), [id])
    }

    func testAutomaticPollPublishesPartialFailureAndLaterSuccess() async throws {
        _ = try recordPendingPayment(txid: "poll_result")
        blockHeightService.setHeightSilently(105)
        mockProvider.failingTxids = ["poll_result"]
        var results: [ConfirmationRefreshResult] = []
        pollingService.onRefreshResult = { results.append($0) }

        await pollingService.pollOnce()
        mockProvider.failingTxids = []
        mockProvider.heightMap["poll_result"] = 100
        await pollingService.pollOnce()

        XCTAssertEqual(results, [.completed(failedLookups: 1), .completed(failedLookups: 0)])
    }

    func testAutomaticPollWithNothingPendingPublishesNothing() async {
        blockHeightService.setHeightSilently(105)
        var results: [ConfirmationRefreshResult] = []
        pollingService.onRefreshResult = { results.append($0) }

        await pollingService.pollOnce()

        XCTAssertTrue(results.isEmpty)
        XCTAssertEqual(mockProvider.lookupCount, 0)
    }

    func testHistoryBannerSetByTipFailureAndKeptUntilCleanResult() {
        var history = HistoryContent()
        history.apply(refreshResult: .chainTipUnavailable)
        XCTAssertEqual(history.errorMessage, ConfirmationRefreshResult.chainTipUnavailable.errorMessage)
        XCTAssertNotNil(history.errorMessage)

        history.apply(refreshResult: .timedOut)
        XCTAssertNotNil(history.errorMessage)
        history.apply(refreshResult: .completed(failedLookups: 1))
        XCTAssertNotNil(history.errorMessage)

        history.apply(refreshResult: .completed(failedLookups: 0))
        XCTAssertNil(history.errorMessage)
    }

    func testRevalidationDoesNotReportSuccessWhenTipRefreshFails() async {
        blockHeightService.setHeightSilently(105)
        mockProvider.currentHeightFails = true
        var results: [ConfirmationRefreshResult] = []
        pollingService.onRefreshResult = { results.append($0) }

        await pollingService.revalidateRecentPayments()

        XCTAssertEqual(results, [.chainTipUnavailable])
    }

    private func makePollingService(timeout: Duration, manual: Bool = false) -> ConfirmationPollingService {
        ConfirmationPollingService(
            databaseService: db,
            blockHeightService: blockHeightService,
            confirmationService: ConfirmationService(provider: mockProvider),
            manualTipProvider: manual ? mockProvider : nil,
            manualConfirmationService: manual ? ConfirmationService(provider: mockProvider) : nil,
            manualRefreshTimeout: timeout
        )
    }
}

private final class HangingConfirmationURLProtocol: URLProtocol {
    private static let lock = NSLock()
    private static var started = 0
    private static var stopped = 0
    private static var requestStopped: XCTestExpectation?
    private var cancellationExpectation: XCTestExpectation?
    static var startedCount: Int { lock.withLock { started } }
    static var stoppedCount: Int { lock.withLock { stopped } }

    static func reset(requestStopped: XCTestExpectation) {
        lock.withLock {
            started = 0
            stopped = 0
            Self.requestStopped = requestStopped
        }
    }

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        if request.url!.path == "/blocks/tip/height" {
            let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data("105".utf8))
            client?.urlProtocolDidFinishLoading(self)
        } else {
            Self.lock.withLock {
                Self.started += 1
                cancellationExpectation = Self.requestStopped
            }
        }
    }

    override func stopLoading() {
        if request.url?.path != "/blocks/tip/height" {
            let requestStopped = Self.lock.withLock {
                Self.stopped += 1
                return cancellationExpectation
            }
            requestStopped?.fulfill()
        }
    }
}
