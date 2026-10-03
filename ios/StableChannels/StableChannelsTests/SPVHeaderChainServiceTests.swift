import Foundation
import XCTest
@testable import StableChannels

final class MockTxConfirmationProvider: TxConfirmationProvider, BlockHeightProvider {
    var heightMap: [String: UInt32] = [:]
    var mockCurrentHeight: UInt32 = 800_000
    var failingTxids: Set<String> = []
    var currentHeightFails = false
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
        let tempDir = FileManager.default.temporaryDirectory
        dataDir = tempDir.appendingPathComponent("test_spv_\(UUID().uuidString)")
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
        dataDir = FileManager.default.temporaryDirectory
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
}
