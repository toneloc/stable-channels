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
            tx(txB, outputs: [(address, DepositAttribution.minDepositSats - 1)], blockTime: now),
            tx(txC, outputs: [("bc1qother", 50_000)], blockTime: now),
            tx("not-a-txid", outputs: [(address, 50_000)], blockTime: now),
            tx(
                String(repeating: "d", count: 64),
                outputs: [(address, DepositAttribution.minDepositSats)],
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

    func testSatsPaidToRejectsTotalsAboveTheSupply() {
        let cap = OnchainAmount.maxSats
        XCTAssertEqual(MissedReceiveRecovery.satsPaidTo(tx(txA, outputs: [(address, cap)]), address: address), cap)
        XCTAssertEqual(MissedReceiveRecovery.satsPaidTo(tx(txA, outputs: [(address, cap + 1)]), address: address), 0)
        XCTAssertEqual(
            MissedReceiveRecovery.satsPaidTo(tx(txA, outputs: [(address, cap), (address, 1)]), address: address),
            0
        )
    }

    func testCheckIsThrottled() {
        let interval = MissedReceiveRecovery.checkIntervalSeconds
        XCTAssertTrue(MissedReceiveRecovery.isCheckDue(lastCheckSecs: 0, nowSecs: now))
        XCTAssertFalse(MissedReceiveRecovery.isCheckDue(lastCheckSecs: now - interval + 1, nowSecs: now))
        XCTAssertTrue(MissedReceiveRecovery.isCheckDue(lastCheckSecs: now - interval, nowSecs: now))
    }

    // MARK: - Wallet-list adoption

    func testCandidatesNeedARecordedCandidateTheWalletListsAtTheExactAmount() {
        let row = ReceiveTxidAdoption.Placeholder(
            id: 1,
            address: address,
            amountMsat: 2_323_000,
            candidateTxids: [txA, txB]
        )
        // txC pays the same amount but is not a candidate; txB is one, at another amount.
        let wallet = WalletInboundOnchain(txids: [txA, txB, txC], sats: [txA: 2_323, txB: 5_000, txC: 2_323])

        XCTAssertEqual(
            ReceiveTxidAdoption.candidates(placeholders: [row], wallet: wallet, recordedTxids: []),
            [.init(rowId: 1, txid: txA)]
        )
        XCTAssertEqual(
            ReceiveTxidAdoption.candidates(placeholders: [row], wallet: wallet, recordedTxids: [txA]),
            []
        )
        // A placeholder without candidates (written before they were recorded) is never adopted.
        let legacy = ReceiveTxidAdoption.Placeholder(id: 2, address: address, amountMsat: 2_323_000)
        XCTAssertEqual(ReceiveTxidAdoption.candidates(placeholders: [legacy], wallet: wallet, recordedTxids: []), [])
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
        func load(_ defaults: UserDefaults?) -> OnchainDepositBaseline {
            OnchainDepositBaselineStore.load(defaults: defaults, seedSatsKey: "cached", seedTxidsKey: "cachedTxids")
        }
        defaults.set(42_000, forKey: "cached")
        defaults.set([txA], forKey: "cachedTxids")

        XCTAssertEqual(load(defaults).sats, 42_000)
        XCTAssertEqual(load(defaults).txids, [txA])
        // The cache keeps refreshing while detection is deferred; the baseline must not follow it.
        defaults.set(99_000, forKey: "cached")
        defaults.set([txA, txB], forKey: "cachedTxids")
        XCTAssertEqual(load(defaults).sats, 42_000)
        XCTAssertEqual(load(defaults).txids, [txA])

        OnchainDepositBaselineStore.save(OnchainDepositBaseline(sats: 50_000, txids: [txB]), defaults: defaults)
        XCTAssertEqual(load(defaults).sats, 50_000)
        XCTAssertEqual(load(defaults).txids, [txB])

        OnchainDepositBaselineStore.save(OnchainDepositBaseline(sats: 0, txids: nil), defaults: defaults)
        XCTAssertEqual(load(defaults).sats, 0)
        XCTAssertNil(load(defaults).txids)
        XCTAssertEqual(load(nil).sats, 0)
        XCTAssertNil(load(nil).txids)
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
            sinceSecs: since,
            balanceCounted: true,
            hasSpliceWithoutTxid: try service.spliceRepo.hasPendingSpliceWithoutTxid()
        )
    }

    /// A balance-delta placeholder as KnownAddressDepositRecorder writes it.
    @discardableResult
    private func placeholder(_ sats: Int64, address: String = "addr", candidates: Set<String> = []) throws -> Int64 {
        let resolutionId = try XCTUnwrap(repo.insertOnchainReceiveResolution(address: address))
        let paymentId = "onchain_deposit_\(UUID().uuidString)"
        try repo.recordDepositCandidates(paymentId: paymentId, txids: candidates)
        XCTAssertTrue(repo.recordOnchainPaymentWithResolution(
            paymentId: paymentId,
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

    func testRecordsTxidOnlyAResolutionRowNamed() throws {
        // The resolver named it, but no placeholder took it: like the WebSocket path, only a
        // payment row carrying the txid counts as recorded.
        let id = try XCTUnwrap(repo.insertOnchainReceiveResolution(address: "addr"))
        XCTAssertTrue(repo.updateOnchainReceiveResolution(id: id, txid: "tx1"))

        XCTAssertTrue(try missed("tx1", 11_732))
        XCTAssertFalse(try missed("tx1", 11_732))
    }

    func testAdoptsPendingPlaceholderThatListsTheTxidInsteadOfDuplicating() throws {
        let id = try placeholder(11_732, candidates: ["tx1"])

        XCTAssertTrue(try missed("tx1", 11_732))

        XCTAssertEqual(try pending(), [Pending(txid: "tx1", sats: 11_732)])
        XCTAssertTrue(repo.fetchPendingOnchainReceives().isEmpty, "adoption resolves the resolution row")
        XCTAssertEqual(try service.paymentRepo.getPayment(byId: id)?.txid, "tx1")
    }

    func testDoesNotAdoptPlaceholderByAmountAlone() throws {
        // Written before candidates were recorded: an equal amount is no evidence.
        try placeholder(11_732)

        XCTAssertTrue(try missed("tx1", 11_732))

        XCTAssertEqual(try pending(), [Pending(txid: nil, sats: 11_732), Pending(txid: "tx1", sats: 11_732)])
    }

    func testRefusesAnAmountNoTransactionCanHave() throws {
        XCTAssertFalse(try missed("tx1", Int64.max))
        XCTAssertFalse(try missed("tx2", OnchainAmount.maxSats + 1))
        XCTAssertEqual(try pending(), [])
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

    func testMergedPlaceholderIsSplitByItsCandidateTxids() throws {
        try placeholder(29_967, candidates: ["txA", "txB"])

        try missed("txA", 15_635)
        XCTAssertEqual(try pending(), [Pending(txid: nil, sats: 14_332), Pending(txid: "txA", sats: 15_635)])

        XCTAssertTrue(try missed("txB", 14_332))
        XCTAssertEqual(try pending(), [Pending(txid: "txA", sats: 15_635), Pending(txid: "txB", sats: 14_332)])
        XCTAssertTrue(repo.fetchPendingOnchainReceives().isEmpty, "the placeholder took the last txid")
    }

    func testPlaceholderWithoutCandidatesKeepsItsAmount() throws {
        // Rows that add up to it are not evidence either: it ages out by the stale-row rules.
        try placeholder(50_000)

        try missed("tx1", 20_000)
        try missed("tx2", 30_000)

        XCTAssertEqual(try pending(), [
            Pending(txid: nil, sats: 50_000),
            Pending(txid: "tx1", sats: 20_000),
            Pending(txid: "tx2", sats: 30_000)
        ])
    }

    func testEachPlaceholderTakesOnlyItsOwnCandidates() throws {
        try placeholder(29_967, candidates: ["txA", "txB"])
        try placeholder(10_000, candidates: ["txC"])

        try missed("txC", 10_000)
        try missed("txB", 14_332)
        try missed("txA", 15_635)

        XCTAssertEqual(try pending(), [
            Pending(txid: "txA", sats: 15_635),
            Pending(txid: "txB", sats: 14_332),
            Pending(txid: "txC", sats: 10_000)
        ])
    }

    private func receivedSats() throws -> Int64 {
        let rows = try service.rawSQL.query(
            """
            SELECT COALESCE(SUM(amount_msat), 0) FROM payments
            WHERE payment_type = 'onchain' AND direction = 'received' AND status != 'failed'
            """
        )
        return try XCTUnwrap(rows.first?.int64(0)) / 1000
    }

    func testRecoveredDepositSplitsTheMergedPlaceholderThatListsIt() throws {
        // A paid this address and B an older one, both inside one balance jump.
        try placeholder(80_000, candidates: ["txA", "txB"])

        XCTAssertTrue(try missed("txA", 50_000))

        XCTAssertEqual(try pending(), [Pending(txid: nil, sats: 30_000), Pending(txid: "txA", sats: 50_000)])
        XCTAssertEqual(try repo.completeStaleTxidlessReceives(createdBefore: .max, walletTxids: ["txA", "txB"]), 1)
        XCTAssertEqual(try receivedSats(), 80_000, "history counts deposit A once")
    }

    func testRecoveredDepositDoesNotShrinkAPlaceholderThatDoesNotListIt() throws {
        try placeholder(80_000, candidates: ["txB"])
        try placeholder(80_000, address: "other", candidates: ["txA"])

        XCTAssertTrue(try missed("txA", 50_000))

        XCTAssertEqual(try pending(), [Pending(txid: nil, sats: 80_000), Pending(txid: "txA", sats: 50_000)])
        XCTAssertEqual(try receivedSats(), 210_000, "both placeholders keep their full amount")
    }

    // MARK: - Wallet-list adoption

    func testAddresslessPlaceholderIsNeverACandidate() throws {
        try record("onchain_deposit_x", "onchain", sats: 2_323, status: "pending")
        try repo.recordDepositCandidates(paymentId: "onchain_deposit_x", txids: ["tx1"])

        XCTAssertTrue(repo.placeholdersWithCandidates().isEmpty)
    }

    func testListsAddressedPlaceholdersWithTheirCandidates() throws {
        let id = try placeholder(2_323, address: "addrA", candidates: ["tx1", "tx2"])
        try placeholder(4_000, address: "addrA")
        try missed("tx9", 5_000, address: "addrA")

        XCTAssertEqual(repo.placeholdersWithCandidates(), [
            ReceiveTxidAdoption.Placeholder(
                id: id,
                address: "addrA",
                amountMsat: 2_323_000,
                candidateTxids: ["tx1", "tx2"]
            )
        ])
    }

    func testAttachTxidTakesOnlyACandidateOfTheExactAmountOnce() throws {
        let id = try placeholder(2_323, candidates: ["tx1", "tx2"])

        XCTAssertFalse(try repo.attachTxid(rowId: id, txid: "tx3", address: "addr", amountMsat: 2_323_000))
        XCTAssertFalse(try repo.attachTxid(rowId: id, txid: "tx1", address: "other", amountMsat: 2_323_000))
        XCTAssertFalse(try repo.attachTxid(rowId: id, txid: "tx1", address: "addr", amountMsat: 2_000_000))
        XCTAssertTrue(try repo.attachTxid(rowId: id, txid: "tx1", address: "addr", amountMsat: 2_323_000))
        XCTAssertFalse(try repo.attachTxid(rowId: id, txid: "tx2", address: "addr", amountMsat: 2_323_000))

        XCTAssertEqual(try pending(), [Pending(txid: "tx1", sats: 2_323)])
        let row = try XCTUnwrap(service.paymentRepo.getPayment(byId: id))
        XCTAssertEqual(row.status, "pending", "the confirmation poller completes it")
        XCTAssertTrue(row.paymentId?.hasPrefix("onchain_deposit_") == true, "the row keeps its id and candidates")
        XCTAssertTrue(repo.placeholdersWithCandidates().isEmpty)
        XCTAssertTrue(repo.fetchPendingOnchainReceives().isEmpty)
    }

    func testAttachTxidSkipsTxidAlreadyOnARow() throws {
        try missed("tx1", 2_323)
        let id = try placeholder(2_323, candidates: ["tx1"])

        XCTAssertFalse(try repo.attachTxid(rowId: id, txid: "tx1", address: "addr", amountMsat: 2_323_000))
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
