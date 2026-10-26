import XCTest
@testable import StableChannels

/// Pure receive-recovery rules: Esplora parsing, merged placeholders, wallet-list adoption.
final class MissedReceiveRecoveryTests: XCTestCase {
    private let address = "bc1qaddr"
    private let now: Int64 = 1_800_000_000
    private let txA = String(repeating: "a", count: 64)
    private let txB = String(repeating: "b", count: 64)
    private let txC = String(repeating: "c", count: 64)

    private func tx(_ txid: String, outputs: [(String, Int64)], blockTime: Int64? = nil) -> [String: Any] {
        var status: [String: Any] = ["confirmed": blockTime != nil]
        if let blockTime { status["block_time"] = blockTime }
        return [
            "txid": txid,
            "vout": outputs.map { ["scriptpubkey_address": $0.0, "value": $0.1] },
            "status": status
        ]
    }

    private func json(_ txs: [[String: Any]]) throws -> Data {
        try JSONSerialization.data(withJSONObject: txs)
    }

    // MARK: - Esplora parsing

    func testSumsOnlyOutputsPayingTheAddress() throws {
        let data = try json([tx(
            txA,
            outputs: [(address, 5_000), ("bc1qother", 9_000), (address, 2_000)],
            blockTime: now
        )])

        let receives = MissedReceiveRecovery.recentReceives(json: data, address: address, nowSecs: now)

        XCTAssertEqual(receives, [.init(txid: txA, sats: 7_000, timeSecs: now)])
    }

    func testUnconfirmedCountsAsNow() throws {
        let data = try json([tx(txA, outputs: [(address, 5_000)])])

        let receives = MissedReceiveRecovery.recentReceives(json: data, address: address, nowSecs: now)

        XCTAssertEqual(receives?.first?.timeSecs, now)
    }

    func testDropsOldDustUnrelatedAndInvalidTxids() throws {
        let window = MissedReceiveRecovery.windowSeconds
        let data = try json([
            tx(txA, outputs: [(address, 5_000)], blockTime: now - window - 1),
            tx(txB, outputs: [(address, MissedReceiveRecovery.minSats - 1)], blockTime: now),
            tx(txC, outputs: [("bc1qother", 50_000)], blockTime: now),
            tx("not-a-txid", outputs: [(address, 50_000)], blockTime: now),
            tx(
                String(repeating: "d", count: 64),
                outputs: [(address, MissedReceiveRecovery.minSats)],
                blockTime: now - window
            )
        ])

        let receives = MissedReceiveRecovery.recentReceives(json: data, address: address, nowSecs: now)

        XCTAssertEqual(receives?.map(\.txid), [String(repeating: "d", count: 64)])
    }

    func testNonListBodyIsNil() {
        XCTAssertNil(MissedReceiveRecovery.recentReceives(json: Data("{}".utf8), address: address, nowSecs: now))
        XCTAssertEqual(MissedReceiveRecovery.recentReceives(json: Data("[]".utf8), address: address, nowSecs: now), [])
    }

    func testSatsPaidToRejectsOverflowAndNonPositiveValues() {
        XCTAssertEqual(
            MissedReceiveRecovery.satsPaidTo(tx(txA, outputs: [(address, .max), (address, 1)]), address: address),
            0
        )
        XCTAssertEqual(MissedReceiveRecovery.satsPaidTo(tx(txA, outputs: [(address, -5)]), address: address), 0)
        XCTAssertEqual(MissedReceiveRecovery.satsPaidTo(["txid": txA], address: address), 0)
    }

    func testCheckIsThrottled() {
        let interval = MissedReceiveRecovery.checkIntervalSeconds
        XCTAssertTrue(MissedReceiveRecovery.isCheckDue(lastCheckSecs: 0, nowSecs: now))
        XCTAssertFalse(MissedReceiveRecovery.isCheckDue(lastCheckSecs: now - interval + 1, nowSecs: now))
        XCTAssertTrue(MissedReceiveRecovery.isCheckDue(lastCheckSecs: now - interval, nowSecs: now))
    }

    // MARK: - Balance-delta skip

    func testDeltaIsRecordedOnlyWhenLatestTxidForThisAddressHasPendingRow() {
        func check(_ txid: String?, _ txidAddress: String?, _ receiveAddress: String?, hasRow: Bool = true) -> Bool {
            MissedReceiveRecovery.isDeltaAlreadyRecorded(
                lastReceiveTxid: txid,
                lastReceiveTxidAddress: txidAddress,
                receiveAddress: receiveAddress,
                txidHasPendingRow: { _ in hasRow }
            )
        }
        XCTAssertTrue(check(txA, address, address))
        XCTAssertFalse(check(txA, address, address, hasRow: false))
        XCTAssertFalse(check(txA, "bc1qold", address))
        XCTAssertFalse(check(txA, nil, address))
        XCTAssertFalse(check(nil, address, address))
        XCTAssertFalse(check("", address, address))
        XCTAssertFalse(check(txA, "", ""))
    }

    // MARK: - Merged placeholders

    func testTwoOrMoreSumTo() {
        XCTAssertTrue(MergedReceivePolicy.twoOrMoreSumTo([15_635, 14_332], target: 29_967))
        XCTAssertTrue(MergedReceivePolicy.twoOrMoreSumTo([1, 99, 2, 3], target: 6))
        XCTAssertTrue(MergedReceivePolicy.twoOrMoreSumTo([10, 10], target: 20))
        XCTAssertFalse(MergedReceivePolicy.twoOrMoreSumTo([20], target: 20))
        XCTAssertFalse(MergedReceivePolicy.twoOrMoreSumTo([20, 5], target: 20))
        XCTAssertFalse(MergedReceivePolicy.twoOrMoreSumTo([], target: 20))
        XCTAssertFalse(MergedReceivePolicy.twoOrMoreSumTo([0, 0], target: 0))
    }

    func testMergedPlaceholderOnlyCountsRowsWithinWindowAfterIt() {
        let window = MergedReceivePolicy.windowSeconds
        let placeholder = MergedReceivePolicy.Row(id: 1, amountMsat: 30, createdAt: 1_000)
        func merged(_ receives: [MergedReceivePolicy.Row]) -> [Int64] {
            MergedReceivePolicy.mergedPlaceholderIds(placeholders: [placeholder], receives: receives)
        }
        XCTAssertEqual(
            merged([
                .init(id: 2, amountMsat: 10, createdAt: 1_000),
                .init(id: 3, amountMsat: 20, createdAt: 1_000 + window)
            ]),
            [1]
        )
        XCTAssertEqual(
            merged([
                .init(id: 2, amountMsat: 10, createdAt: 1_000),
                .init(id: 3, amountMsat: 20, createdAt: 1_001 + window)
            ]),
            []
        )
        XCTAssertEqual(
            merged([.init(id: 2, amountMsat: 10, createdAt: 999), .init(id: 3, amountMsat: 20, createdAt: 1_000)]),
            []
        )
    }

    func testMergedPlaceholderOnlySearchesEarliestRows() {
        let cap = MergedReceivePolicy.maxRowsPerPlaceholder
        let placeholder = MergedReceivePolicy.Row(id: 1, amountMsat: 300, createdAt: 0)
        let filler = (0..<(cap - 1))
            .map { MergedReceivePolicy.Row(id: Int64(10 + $0), amountMsat: 1_000, createdAt: 1) }
        let first = MergedReceivePolicy.Row(id: 2, amountMsat: 100, createdAt: 0)
        let late = MergedReceivePolicy.Row(id: 3, amountMsat: 200, createdAt: 2)

        XCTAssertEqual(
            MergedReceivePolicy.mergedPlaceholderIds(placeholders: [placeholder], receives: filler + [late, first]),
            []
        )
        XCTAssertEqual(
            MergedReceivePolicy.mergedPlaceholderIds(
                placeholders: [placeholder],
                receives: Array(filler.dropLast()) + [late, first]
            ),
            [1]
        )
    }

    // MARK: - Wallet-list adoption

    func testCandidatesNeedExactAmountUnrecordedTxidAndRecentWalletSighting() {
        let slack = ReceiveTxidAdoption.slackSeconds
        let row = ReceiveTxidAdoption.Placeholder(id: 1, address: address, amountMsat: 2_323_000, createdAt: now)
        let receives = [
            ReceiveTxidAdoption.WalletReceive(txid: txA, amountMsat: 2_323_000, seenAtSecs: now - slack),
            ReceiveTxidAdoption.WalletReceive(txid: txB, amountMsat: 2_323_000, seenAtSecs: now - slack - 1),
            ReceiveTxidAdoption.WalletReceive(txid: txC, amountMsat: 5_000_000, seenAtSecs: now)
        ]

        XCTAssertEqual(
            ReceiveTxidAdoption.candidates(placeholders: [row], walletReceives: receives, recordedTxids: []),
            [.init(rowId: 1, txid: txA)]
        )
        XCTAssertEqual(
            ReceiveTxidAdoption.candidates(placeholders: [row], walletReceives: receives, recordedTxids: [txA]),
            []
        )
    }

    func testAdoptsOnlyRowsPaidByOneDistinctTxid() {
        let adoptions = ReceiveTxidAdoption.adoptions(paying: [
            .init(rowId: 1, txid: txA),
            .init(rowId: 1, txid: txA),
            .init(rowId: 2, txid: txB),
            .init(rowId: 2, txid: txC)
        ])

        XCTAssertEqual(adoptions, [1: txA])
    }

    // MARK: - Deposit baseline

    func testDepositBaselineSeedsFromBalanceCacheOnceThenPersists() throws {
        let suite = "MissedReceiveRecoveryTests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(42_000, forKey: "cached")

        XCTAssertEqual(DepositBaseline.load(defaults: defaults, seedKey: "cached"), 42_000)
        defaults.set(99_000, forKey: "cached")
        XCTAssertEqual(DepositBaseline.load(defaults: defaults, seedKey: "cached"), 42_000)

        DepositBaseline.save(0, defaults: defaults)
        XCTAssertEqual(DepositBaseline.load(defaults: defaults, seedKey: "cached"), 0)
        XCTAssertEqual(DepositBaseline.load(defaults: nil, seedKey: "cached"), 0)
    }
}

/// Receive rows in SQLite: missed-receive recording, placeholder adoption, merged
/// placeholders (#317), wallet-list txid adoption. Ported from the Android
/// OnchainMissedReceiveDatabaseServiceTest.
final class OnchainReceiveRecoveryRepositoryTests: XCTestCase {
    private var service: DatabaseService!
    private var dataDir: URL!
    private var repo: OnchainReceiveRepository { service.onchainRepo }
    private let since: Int64 = 0

    override func setUpWithError() throws {
        try super.setUpWithError()
        dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("OnchainReceiveRecoveryRepositoryTests-\(UUID().uuidString)")
        service = try DatabaseService(dataDir: dataDir)
    }

    override func tearDown() {
        service = nil
        try? FileManager.default.removeItem(at: dataDir)
        super.tearDown()
    }

    @discardableResult
    private func missed(_ txid: String, _ sats: Int64, address: String = "addr") throws -> Bool {
        try repo.recordMissedReceive(
            txid: txid,
            amountSats: sats,
            amountUSD: nil,
            btcPrice: nil,
            address: address,
            sinceSecs: since
        )
    }

    /// A balance-delta placeholder as KnownAddressDepositRecorder writes it.
    @discardableResult
    private func placeholder(_ sats: Int64, address: String = "addr") throws -> Int64 {
        let resolutionId = try XCTUnwrap(repo.insertOnchainReceiveResolution(address: address))
        XCTAssertTrue(repo.recordOnchainPaymentWithResolution(
            paymentId: "onchain_deposit_\(UUID().uuidString)",
            amountMsat: sats * 1000,
            amountUSD: nil,
            btcPrice: nil,
            resolutionId: resolutionId
        ))
        return try XCTUnwrap(service.rawSQL.query("SELECT MAX(id) FROM payments").first?.optInt64(0))
    }

    private func record(_ paymentId: String, _ type: String, sats: Int64, status: String, txid: String? = nil) throws {
        _ = try service.paymentRepo.recordPayment(
            paymentId: paymentId,
            paymentType: type,
            direction: "received",
            amountMsat: UInt64(sats * 1000),
            amountUSD: nil,
            btcPrice: nil,
            counterparty: nil,
            status: status,
            txid: txid
        )
    }

    private struct Pending: Hashable {
        let txid: String?
        let sats: Int64
    }

    private func pending() throws -> Set<Pending> {
        let rows = try service.rawSQL.query(
            """
            SELECT txid, amount_msat FROM payments
            WHERE payment_type = 'onchain' AND direction = 'received' AND status = 'pending'
            """
        )
        return Set(rows.map { Pending(txid: $0.optString(0), sats: $0.int64(1) / 1000) })
    }

    // MARK: - Missed receives

    func testRecordsMissingDepositOnce() throws {
        XCTAssertTrue(try missed("tx1", 11_732))
        XCTAssertFalse(try missed("tx1", 11_732))
        XCTAssertEqual(try pending(), [Pending(txid: "tx1", sats: 11_732)])
        XCTAssertNotNil(service.paymentRepo.payment(paymentId: "onchain_receive_tx1"))
    }

    func testSkipsWhenCompletedRowWithSameAmountExists() throws {
        try record("old", "onchain", sats: 11_732, status: "completed")

        XCTAssertFalse(try missed("tx1", 11_732))
    }

    func testSkipsTxidAlreadyOnAResolutionRow() throws {
        let id = try XCTUnwrap(repo.insertOnchainReceiveResolution(address: "addr"))
        XCTAssertTrue(repo.updateOnchainReceiveResolution(id: id, txid: "tx1"))

        XCTAssertFalse(try missed("tx1", 11_732))
    }

    func testAdoptsPendingPlaceholderInsteadOfDuplicating() throws {
        try placeholder(11_732)

        XCTAssertTrue(try missed("tx1", 11_732))

        XCTAssertEqual(try pending(), [Pending(txid: "tx1", sats: 11_732)])
        XCTAssertTrue(repo.fetchPendingOnchainReceives().isEmpty, "adoption resolves the resolution row")
        XCTAssertNotNil(service.paymentRepo.payment(paymentId: "onchain_receive_tx1"))
    }

    func testDoesNotAdoptPlaceholderOnAnotherAddressOrAmount() throws {
        try placeholder(11_732, address: "other")
        try placeholder(5_000)

        XCTAssertTrue(try missed("tx1", 11_732))

        XCTAssertEqual(try pending(), [
            Pending(txid: nil, sats: 11_732),
            Pending(txid: nil, sats: 5_000),
            Pending(txid: "tx1", sats: 11_732)
        ])
    }

    func testRecordsDepositWhenTxidlessRowOfSameAmountFailed() throws {
        try record("failed_x", "onchain", sats: 11_732, status: "failed")

        XCTAssertTrue(try missed("tx2", 11_732))
    }

    func testRecordsReplacementWhenSameAmountRowFailed() throws {
        try missed("tx1", 11_732)
        try service.paymentRepo.failPaymentByTxid(txid: "tx1")

        XCTAssertTrue(try missed("tx2", 11_732))
        XCTAssertEqual(try pending(), [Pending(txid: "tx2", sats: 11_732)])
    }

    func testRecordsSecondDepositOfSameAmountWithDifferentTxid() throws {
        try missed("tx1", 11_732)

        XCTAssertTrue(try missed("tx2", 11_732))
        XCTAssertEqual(try pending(), [Pending(txid: "tx1", sats: 11_732), Pending(txid: "tx2", sats: 11_732)])
    }

    func testWaitsWhileSpliceHasNoTxid() throws {
        try record("splice", "splice_out", sats: 25_000, status: "pending")

        XCTAssertFalse(try missed("tx1", 5_991))
        service.paymentRepo.updatePaymentTxid(paymentId: "splice", txid: "splicetx", status: "pending")
        XCTAssertTrue(try missed("tx1", 5_991))
        XCTAssertFalse(try missed("splicetx", 5_991))
        XCTAssertEqual(try pending(), [Pending(txid: "tx1", sats: 5_991)])
    }

    func testWaitsWhileSpliceInHasNoTxid() throws {
        try record("splice", "splice_in", sats: 25_000, status: "pending")

        XCTAssertFalse(try missed("tx1", 5_991))
    }

    // MARK: - Merged placeholders (#317)

    func testMergedPlaceholderIsReplacedByOneRowPerTxid() throws {
        try placeholder(29_967)

        try missed("txA", 15_635)
        XCTAssertEqual(try pending(), [Pending(txid: nil, sats: 29_967), Pending(txid: "txA", sats: 15_635)])

        XCTAssertTrue(try repo.recordReceive(
            txid: "txB",
            amountSats: 14_332,
            amountUSD: nil,
            btcPrice: nil,
            address: "addr"
        ))
        XCTAssertEqual(try pending(), [Pending(txid: "txA", sats: 15_635), Pending(txid: "txB", sats: 14_332)])
        XCTAssertTrue(repo.fetchPendingOnchainReceives().isEmpty, "merged placeholder's resolution row is dropped too")
    }

    func testKeepsPlaceholderWhenLaterRowsDoNotAddUpToIt() throws {
        try placeholder(50_000)

        try missed("tx1", 20_000)
        try missed("tx2", 20_000)
        try missed("tx3", 50_000, address: "other")

        XCTAssertEqual(try pending(), [
            Pending(txid: nil, sats: 50_000),
            Pending(txid: "tx1", sats: 20_000),
            Pending(txid: "tx2", sats: 20_000),
            Pending(txid: "tx3", sats: 50_000)
        ])
    }

    func testMergedPlaceholderIsReplacedWhenALaterDepositIsRecordedToo() throws {
        try placeholder(29_967)
        try placeholder(10_000)

        try missed("txC", 10_000)
        try missed("txB", 14_332)
        try missed("txA", 15_635)

        XCTAssertEqual(try pending(), [
            Pending(txid: "txA", sats: 15_635),
            Pending(txid: "txB", sats: 14_332),
            Pending(txid: "txC", sats: 10_000)
        ])
    }

    func testFailedRowsDoNotCountTowardMergedPlaceholder() throws {
        try placeholder(29_967)

        try missed("txA", 15_635)
        try service.paymentRepo.failPaymentByTxid(txid: "txA")
        try missed("txB", 14_332)

        XCTAssertEqual(try pending(), [Pending(txid: nil, sats: 29_967), Pending(txid: "txB", sats: 14_332)])
    }

    func testDepositsRecordedLongAfterPlaceholderDoNotCountTowardIt() throws {
        try placeholder(30_000)
        try service.rawSQL.execute(
            "UPDATE payments SET created_at = created_at - ?",
            params: [.integer(MergedReceivePolicy.windowSeconds + 60)]
        )

        try missed("tx1", 10_000)
        try missed("tx2", 20_000)

        XCTAssertEqual(try pending(), [
            Pending(txid: nil, sats: 30_000),
            Pending(txid: "tx1", sats: 10_000),
            Pending(txid: "tx2", sats: 20_000)
        ])
    }

    func testSingleLaterRowWithSameAmountDoesNotDropPlaceholder() throws {
        try placeholder(10_000)
        _ = try service.paymentRepo.recordPayment(
            paymentId: "onchain_receive_txS",
            paymentType: "onchain",
            direction: "received",
            amountMsat: 10_000_000,
            amountUSD: nil,
            btcPrice: nil,
            counterparty: nil,
            status: "pending",
            txid: "txS",
            address: "addr"
        )

        try missed("tx2", 3_000)

        XCTAssertEqual(try pending(), [
            Pending(txid: nil, sats: 10_000),
            Pending(txid: "txS", sats: 10_000),
            Pending(txid: "tx2", sats: 3_000)
        ])
    }

    func testRowsRecordedBeforePlaceholderDoNotCountTowardIt() throws {
        try missed("tx1", 10_000)
        try service.rawSQL.execute("UPDATE payments SET created_at = created_at - 60")
        try placeholder(30_000)

        try missed("tx2", 20_000)

        XCTAssertEqual(try pending(), [
            Pending(txid: "tx1", sats: 10_000),
            Pending(txid: nil, sats: 30_000),
            Pending(txid: "tx2", sats: 20_000)
        ])
    }

    // MARK: - Wallet-list adoption

    func testAddresslessPlaceholderIsNeverACandidate() throws {
        try record("onchain_deposit_x", "onchain", sats: 2_323, status: "pending")

        XCTAssertTrue(repo.txidlessPendingReceivesWithAddress().isEmpty)
    }

    func testListsAddressedPlaceholdersWithTheirAddress() throws {
        let id = try placeholder(2_323, address: "addrA")
        try missed("tx9", 5_000, address: "addrA")

        let rows = repo.txidlessPendingReceivesWithAddress()

        XCTAssertEqual(rows.map(\.id), [id])
        XCTAssertEqual(rows.first?.address, "addrA")
        XCTAssertEqual(rows.first?.amountMsat, 2_323_000)
    }

    func testAdoptTxidAttachesOnceAndNeverOverwrites() throws {
        let id = try placeholder(2_323)

        XCTAssertTrue(repo.adoptTxid(rowId: id, txid: "tx1", address: "addr"))
        XCTAssertFalse(repo.adoptTxid(rowId: id, txid: "tx2", address: "addr"))

        XCTAssertEqual(try pending(), [Pending(txid: "tx1", sats: 2_323)])
        XCTAssertTrue(repo.txidlessPendingReceivesWithAddress().isEmpty)
        XCTAssertTrue(repo.fetchPendingOnchainReceives().isEmpty)
    }

    func testAdoptTxidSkipsTxidAlreadyOnARow() throws {
        try missed("tx1", 2_323)
        let id = try placeholder(2_323)

        XCTAssertFalse(repo.adoptTxid(rowId: id, txid: "tx1", address: "addr"))
        XCTAssertEqual(try pending(), [Pending(txid: "tx1", sats: 2_323), Pending(txid: nil, sats: 2_323)])
    }
}

/// The one "outputs paying an address" rule, shared by recovery, txid resolution and the
/// confirmation address check.
final class AddressOutputMatchingTests: XCTestCase {
    func testAddressCaseAndWhitespaceDoNotChangeTheMatch() {
        let tx: [String: Any] = ["vout": [["scriptpubkey_address": "BC1QADDR", "value": Int64(5000)]]]
        XCTAssertEqual(MissedReceiveRecovery.satsPaidTo(tx, address: "bc1qaddr"), 5000)
        XCTAssertEqual(MissedReceiveRecovery.satsPaidTo(tx, address: " BC1QADDR "), 5000)
        XCTAssertEqual(MissedReceiveRecovery.satsPaidTo(tx, address: ""), 0)
        XCTAssertEqual(MissedReceiveRecovery.satsPaidTo(tx, address: "bc1qother"), 0)
    }
}
