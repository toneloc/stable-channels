import Foundation
import XCTest
@testable import StableChannels

final class MockTxConfirmationProvider: TxConfirmationProvider, BlockHeightProvider {
    var heightMap: [String: UInt32] = [:]
    var mockCurrentHeight: UInt32 = 800_000

    func blockHeight(for txid: String) async throws -> UInt32? {
        heightMap[txid]
    }

    func currentHeight() async throws -> UInt32 {
        mockCurrentHeight
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
