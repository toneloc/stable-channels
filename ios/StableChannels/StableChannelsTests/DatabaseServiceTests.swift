import SQLite3
import XCTest
@testable import StableChannels

final class DatabaseServiceTests: XCTestCase {
    private var service: DatabaseService!
    private var dataDir: URL!

    override func setUp() {
        super.setUp()
        dataDir = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("DatabaseServiceTests-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        service = try? DatabaseService(dataDir: dataDir)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dataDir)
        service = nil
        super.tearDown()
    }

    // MARK: - Atomic backing updates

    func testBackingDeltaIsAtomicAndDuplicateReturnsStoredBacking() throws {
        try service.channelRepo.saveChannel(
            channelId: "channel-1",
            userChannelId: "user-channel-1",
            expectedUSD: 100,
            backingSats: 1_000,
            note: nil
        )

        let first = try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: "payment-1",
            paymentType: "stability",
            direction: "received",
            amountMsat: 100_000,
            amountUSD: 1,
            btcPrice: 100_000,
            status: "completed",
            userChannelId: "user-channel-1",
            backingDeltaSats: 100
        )
        XCTAssertTrue(first.isNewPayment)
        XCTAssertEqual(first.backingSats, 1_100)

        let duplicate = try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: "payment-1",
            paymentType: "stability",
            direction: "received",
            amountMsat: 100_000,
            amountUSD: 1,
            btcPrice: 100_000,
            status: "completed",
            userChannelId: "user-channel-1",
            backingDeltaSats: 100
        )
        XCTAssertFalse(duplicate.isNewPayment)
        XCTAssertEqual(duplicate.backingSats, 1_100)

        let second = try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: "payment-2",
            paymentType: "stability",
            direction: "received",
            amountMsat: 50_000,
            amountUSD: 0.5,
            btcPrice: 100_000,
            status: "completed",
            userChannelId: "user-channel-1",
            backingDeltaSats: 50
        )
        XCTAssertEqual(second.backingSats, 1_150)

        let outgoing = try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: "payment-outgoing",
            paymentType: "stability",
            direction: "sent",
            amountMsat: 200_000,
            amountUSD: 2,
            btcPrice: 100_000,
            status: "pending",
            userChannelId: "user-channel-1",
            backingDeltaSats: -200
        )
        XCTAssertTrue(outgoing.isNewPayment)
        XCTAssertEqual(outgoing.backingSats, 950)

        let outgoingReplay = try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: "payment-outgoing",
            paymentType: "stability",
            direction: "sent",
            amountMsat: 200_000,
            amountUSD: 2,
            btcPrice: 100_000,
            status: "pending",
            userChannelId: "user-channel-1",
            backingDeltaSats: -200
        )
        XCTAssertFalse(outgoingReplay.isNewPayment)
        XCTAssertEqual(outgoingReplay.backingSats, 950)

        try service.channelRepo.saveChannelPreservingBacking(
            channelId: "channel-1",
            userChannelId: "user-channel-1",
            expectedUSD: 125,
            note: "metadata-only"
        )
        let stored = try XCTUnwrap(service.channelRepo.loadChannel(userChannelId: "user-channel-1"))
        XCTAssertEqual(stored.backingSats, 950)
        XCTAssertEqual(stored.expectedUSD, 125)
    }

    func testDebitBelowZeroClampsToZeroAndSucceeds() throws {
        try service.channelRepo.saveChannel(
            channelId: "channel-1",
            userChannelId: "user-channel-1",
            expectedUSD: 100,
            backingSats: 500,
            note: nil
        )

        // A debit larger than the stored backing clamps to 0 and still records
        // the payment — it runs after a successful keysend, so refusing to
        // record would wedge reconcile forever.
        let clamped = try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: "payment-too-large",
            paymentType: "stability",
            direction: "sent",
            amountMsat: 2_000_000,
            amountUSD: 20,
            btcPrice: 100_000,
            status: "pending",
            userChannelId: "user-channel-1",
            backingDeltaSats: -2_000
        )
        XCTAssertTrue(clamped.isNewPayment)
        XCTAssertEqual(clamped.backingSats, 0)

        let stored = try XCTUnwrap(service.channelRepo.loadChannel(userChannelId: "user-channel-1"))
        XCTAssertEqual(stored.backingSats, 0)

        // Replay of the same payment dedups and reports the clamped backing.
        let replay = try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: "payment-too-large",
            paymentType: "stability",
            direction: "sent",
            amountMsat: 2_000_000,
            amountUSD: 20,
            btcPrice: 100_000,
            status: "pending",
            userChannelId: "user-channel-1",
            backingDeltaSats: -2_000
        )
        XCTAssertFalse(replay.isNewPayment)
        XCTAssertEqual(replay.backingSats, 0)
    }

    func testBackingUpdateWithMissingChannelRowThrowsDedicatedError() throws {
        XCTAssertThrowsError(
            try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
                paymentId: "payment-no-row",
                paymentType: "stability",
                direction: "sent",
                amountMsat: 100_000,
                amountUSD: 1,
                btcPrice: 100_000,
                status: "pending",
                userChannelId: "no-such-channel",
                backingDeltaSats: -100
            )
        ) { error in
            guard case DatabaseError.missingChannelRow(let ucid) = error else {
                return XCTFail("Expected missingChannelRow, got \(error)")
            }
            XCTAssertEqual(ucid, "no-such-channel")
        }

        // The whole transaction rolled back: after the channel row is recreated,
        // the same payment id inserts as new (no orphan payments row).
        try service.channelRepo.saveChannel(
            channelId: "channel-1",
            userChannelId: "no-such-channel",
            expectedUSD: 100,
            backingSats: 1_000,
            note: nil
        )
        let retried = try service.paymentRepo.recordPaymentAndMaybeUpdateBacking(
            paymentId: "payment-no-row",
            paymentType: "stability",
            direction: "sent",
            amountMsat: 100_000,
            amountUSD: 1,
            btcPrice: 100_000,
            status: "pending",
            userChannelId: "no-such-channel",
            backingDeltaSats: -100
        )
        XCTAssertTrue(retried.isNewPayment)
        XCTAssertEqual(retried.backingSats, 900)
    }

    // MARK: - pending_stability_send

    func testClaimPendingSendClaimDenyClearCycle() throws {
        XCTAssertNil(service.stabilityRepo.loadPendingSend())

        // First claim wins the slot with an empty payment id.
        XCTAssertTrue(service.stabilityRepo.claimPendingSend(amountMsat: 123_000, price: 100_000))
        var pending = try XCTUnwrap(service.stabilityRepo.loadPendingSend())
        XCTAssertEqual(pending.paymentId, "")
        XCTAssertEqual(pending.amountMsat, 123_000)
        XCTAssertEqual(pending.price, 100_000)
        XCTAssertGreaterThan(pending.createdAt, 0)

        // Second claim is denied while the marker exists.
        XCTAssertFalse(service.stabilityRepo.claimPendingSend(amountMsat: 456_000, price: 100_000))
        pending = try XCTUnwrap(service.stabilityRepo.loadPendingSend())
        XCTAssertEqual(pending.amountMsat, 123_000, "Denied claim must not overwrite the marker")

        // The real payment id attaches once the keysend returns.
        XCTAssertTrue(service.stabilityRepo.setPendingSendPaymentId("payment-abc"))
        pending = try XCTUnwrap(service.stabilityRepo.loadPendingSend())
        XCTAssertEqual(pending.paymentId, "payment-abc")
        XCTAssertEqual(pending.amountMsat, 123_000)

        // Clear frees the slot for the next claim.
        service.stabilityRepo.clearPendingSend()
        XCTAssertNil(service.stabilityRepo.loadPendingSend())
        XCTAssertTrue(service.stabilityRepo.claimPendingSend(amountMsat: 456_000, price: 90_000))
    } // MARK: - pending_operations

    func testPendingOperationsInsertFetch() {
        let ok = service.pendingOpRepo.insertPendingOperation(
            opId: "close-abc",
            opType: "channel_close",
            fundingOutpointTxid: "deadbeef",
            fundingOutpointVout: 1
        )
        XCTAssertTrue(ok)

        let ops = service.pendingOpRepo.fetchPendingOperations()
        XCTAssertEqual(ops.count, 1)
        let op = ops[0]
        XCTAssertEqual(op.opId, "close-abc")
        XCTAssertEqual(op.opType, "channel_close")
        XCTAssertEqual(op.fundingOutpointTxid, "deadbeef")
        XCTAssertEqual(op.fundingOutpointVout, 1)
        XCTAssertEqual(op.status, "pending")
        XCTAssertNil(op.closingTxid)
        XCTAssertNil(op.resolvedAt)
    }

    func testPendingOperationsUpdatePreservesRow() {
        _ = service.pendingOpRepo.insertPendingOperation(
            opId: "close-xyz",
            opType: "channel_close",
            fundingOutpointTxid: "cafebabe",
            fundingOutpointVout: 0
        )
        let ok = service.pendingOpRepo.updatePendingOperation(
            opId: "close-xyz",
            closingTxid: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            status: "resolved"
        )
        XCTAssertTrue(ok)

        // fetchPendingOperations() filters by status='pending', so the
        // resolved row is excluded. Use the PK lookup instead.
        let op = service.pendingOpRepo.fetchPendingOperation(opId: "close-xyz")
        XCTAssertNotNil(op)
        guard let op else { return }
        XCTAssertEqual(op.opId, "close-xyz")
        XCTAssertEqual(op.opType, "channel_close")
        XCTAssertEqual(op.fundingOutpointTxid, "cafebabe")
        XCTAssertEqual(op.fundingOutpointVout, 0)
        XCTAssertEqual(op.closingTxid,
                       "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        XCTAssertEqual(op.status, "resolved")
        XCTAssertNotNil(op.resolvedAt)
    }

    func testUpdatePendingOperationOnlyUpdatesPending() {
        _ = service.pendingOpRepo.insertPendingOperation(
            opId: "close-q",
            opType: "channel_close",
            fundingOutpointTxid: nil,
            fundingOutpointVout: nil
        )
        // First update succeeds and flips status to resolved.
        let first = service.pendingOpRepo.updatePendingOperation(
            opId: "close-q",
            closingTxid: "first",
            status: "resolved"
        )
        XCTAssertTrue(first)

        // Second update must be a no-op because the row is no longer pending.
        let second = service.pendingOpRepo.updatePendingOperation(
            opId: "close-q",
            closingTxid: "second",
            status: "resolved"
        )
        XCTAssertFalse(second, "Second update must not clobber a resolved row")

        // fetchPendingOperations() filters by status='pending', so the resolved
        // row is excluded. Use the PK lookup to verify the first txid stuck.
        let op = service.pendingOpRepo.fetchPendingOperation(opId: "close-q")
        XCTAssertEqual(op?.closingTxid, "first")
    }

    // MARK: - onchain_receive_txids

    func testInsertOnchainReceiveResolution_returnsNonZeroId() {
        let id = service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qexampleaddress")
        XCTAssertNotNil(id)
        let unwrapped = try? XCTUnwrap(id)
        XCTAssertGreaterThan(unwrapped ?? 0, 0)
    }

    func testFetchPendingOnchainReceives_returnsInsertedRow() {
        _ = try? XCTUnwrap(service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qfirst"))
        _ = try? XCTUnwrap(service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qsecond"))

        let pending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertEqual(pending.count, 2)
        XCTAssertEqual(pending[0].address, "bc1qfirst")
        XCTAssertEqual(pending[1].address, "bc1qsecond")
        XCTAssertEqual(pending[0].status, "pending")
        XCTAssertNil(pending[0].txid)
        XCTAssertNil(pending[0].resolvedAt)
    }

    func testUpdateOnchainReceiveResolution_setsTxidAndMarksResolved() {
        let id = try? XCTUnwrap(
            service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qtoupdate")
        )
        let txid = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"

        let ok = service.onchainRepo.updateOnchainReceiveResolution(
            id: id ?? 0,
            txid: txid
        )
        XCTAssertTrue(ok)

        let pending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertTrue(pending.isEmpty, "Resolved rows must not appear in pending fetch")

        // Re-insert another pending row so we can confirm via absence the resolved
        // row is no longer in the pending list.
        _ = try? XCTUnwrap(service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qother"))
        let stillPending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertEqual(stillPending.count, 1)
        XCTAssertEqual(stillPending[0].address, "bc1qother")
    }

    func testFetchPendingOnchainReceives_excludesResolvedRows() {
        let id = try? XCTUnwrap(
            service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qresolve")
        )
        XCTAssertTrue(
            service.onchainRepo.updateOnchainReceiveResolution(
                id: id ?? 0,
                txid: String(repeating: "f", count: 64)
            )
        )

        let pending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertTrue(pending.isEmpty)
    }

    func testUpdateOnchainReceiveResolution_onlyAffectsTargetRow() {
        let idA = try? XCTUnwrap(service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qA"))
        let idB = try? XCTUnwrap(service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qB"))
        let txidA = String(repeating: "a", count: 64)
        let txidB = String(repeating: "b", count: 64)

        XCTAssertTrue(
            service.onchainRepo.updateOnchainReceiveResolution(id: idA ?? 0, txid: txidA)
        )

        let pending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].id, idB)
        XCTAssertEqual(pending[0].address, "bc1qB")
        XCTAssertNil(pending[0].txid)

        XCTAssertTrue(
            service.onchainRepo.updateOnchainReceiveResolution(id: idB ?? 0, txid: txidB)
        )
        let finalPending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertTrue(finalPending.isEmpty)
    }

    // MARK: - Onchain receive integration flow

    /// Full on-chain receive lifecycle: insert resolution -> verify pending ->
    /// update with real txid -> verify resolved (no longer in pending list) ->
    /// verify latest resolved txid is queryable.
    func testOnchainReceiveFlow_pendingRowGetsTxidOnUpdate() {
        let address = "tb1qfakeaddressforintegrationtest1234567890abcdef"

        // 1) Insert resolution row
        guard let resolutionId = service.onchainRepo.insertOnchainReceiveResolution(address: address) else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }
        XCTAssertGreaterThan(resolutionId, 0)

        // 2) Verify the row is in pending state
        let pending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].address, address)
        XCTAssertEqual(pending[0].id, resolutionId)
        XCTAssertNil(pending[0].txid)
        XCTAssertEqual(pending[0].status, "pending")

        // 3) Update with a real txid
        let txid = String(repeating: "a", count: 64)
        let updated = service.onchainRepo.updateOnchainReceiveResolution(id: resolutionId, txid: txid)
        XCTAssertTrue(updated)

        // 4) Verify the row is no longer in the pending fetch
        let stillPending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertEqual(stillPending.count, 0, "Resolved row should not appear in pending fetch")

        // 5) Latest resolved txid
        XCTAssertEqual(service.onchainRepo.fetchLatestResolvedOnchainTxid(), txid)
    }

    /// Dedup invariant: a second `updateOnchainReceiveResolution` on the same
    /// row must return false (the SQL is gated by `status = 'pending'`, so a
    /// resolved row is no longer updatable via this method).
    func testUpdateOnchainReceiveResolution_returnsFalseOnSecondCall() {
        guard let id = service.onchainRepo.insertOnchainReceiveResolution(address: "tb1qtest") else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }
        let txidA = String(repeating: "a", count: 64)
        let txidB = String(repeating: "b", count: 64)

        XCTAssertTrue(service.onchainRepo.updateOnchainReceiveResolution(id: id, txid: txidA))
        XCTAssertFalse(
            service.onchainRepo.updateOnchainReceiveResolution(id: id, txid: txidB),
            "Second update must be a no-op (row is no longer pending)"
        )

        // The original txid must be preserved.
        XCTAssertEqual(service.onchainRepo.fetchLatestResolvedOnchainTxid(), txidA)
    }

    /// `fetchPendingOnchainReceiveRow` returns the row tied to a given
    /// `resolution_id`; a non-matching id returns nil.
    func testFetchPendingOnchainReceiveRow_returnsMatchingRow() {
        guard let resId = service.onchainRepo.insertOnchainReceiveResolution(address: "tb1qtest") else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }

        let ok = service.onchainRepo.recordOnchainPaymentWithResolution(
            paymentId: "p1",
            amountMsat: 50_000_000,
            amountUSD: 100.0,
            btcPrice: 50_000.0,
            resolutionId: resId
        )
        XCTAssertTrue(ok)

        let row = service.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resId)
        XCTAssertNotNil(row)
        XCTAssertEqual(row?.paymentId, "p1")
        XCTAssertEqual(row?.amountMsat, 50_000_000)

        // Different resolutionId returns nil (no row linked to it).
        XCTAssertNil(service.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resId + 1))
    }

    /// `recordOnchainPaymentWithResolution` writes a row that is
    /// (a) visible in `fetchPendingOnchainReceives` for the same resolution,
    /// (b) survives rollback of the resolution row by the cleanup path
    /// (we just verify the write itself succeeds and is findable).
    func testRecordOnchainPaymentWithResolution_writesResolutionId() {
        guard let resId = service.onchainRepo.insertOnchainReceiveResolution(address: "tb1qtest") else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }

        XCTAssertTrue(
            service.onchainRepo.recordOnchainPaymentWithResolution(
                paymentId: "p1",
                amountMsat: 1_000_000,
                amountUSD: nil,
                btcPrice: nil,
                resolutionId: resId
            )
        )

        // The payments row exists, linked to the resolution.
        let row = service.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resId)
        XCTAssertNotNil(row, "Inserted payment must be findable via resolutionId")
        XCTAssertEqual(row?.paymentId, "p1")
        XCTAssertEqual(row?.amountMsat, 1_000_000)

        // And the resolution row itself is still in pending state.
        let pending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].id, resId)
    }

    /// `deleteOnchainReceiveResolution` removes the resolution row; the
    /// payments row linked to it survives (resolution cleanup must not
    /// cascade — the resolution is a separate concern from the payment).
    func testDeleteOnchainReceiveResolution_removesResolutionRow() {
        guard let resId = service.onchainRepo.insertOnchainReceiveResolution(address: "tb1qtest") else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }
        XCTAssertTrue(
            service.onchainRepo.recordOnchainPaymentWithResolution(
                paymentId: "p1",
                amountMsat: 1_000_000,
                amountUSD: nil,
                btcPrice: nil,
                resolutionId: resId
            )
        )

        // Delete the resolution row.
        XCTAssertTrue(service.onchainRepo.deleteOnchainReceiveResolution(id: resId))

        // The resolution is gone.
        let pending = service.onchainRepo.fetchPendingOnchainReceives()
        XCTAssertTrue(pending.isEmpty, "Resolution row must be deleted")

        // The payments row survives — it's the user-facing record.
        let row = service.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resId)
        XCTAssertNotNil(row, "Payments row must survive resolution deletion")
    }

    /// `fetchLatestResolvedOnchainTxid` returns a resolved txid (the
    /// schema uses whole-second `strftime('%s','now')` for `resolved_at`,
    /// so we cannot reliably distinguish "most recent" within a single
    /// second — both txids are correct answers in that case. We assert
    /// the basic contract: a resolved txid is queryable, and after two
    /// distinct resolutions the call still returns one of them).
    func testFetchLatestResolvedOnchainTxid_returnsResolvedTxid() {
        let idA = try? XCTUnwrap(service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qA"))
        let idB = try? XCTUnwrap(service.onchainRepo.insertOnchainReceiveResolution(address: "bc1qB"))
        let txidA = String(repeating: "a", count: 64)
        let txidB = String(repeating: "b", count: 64)

        XCTAssertTrue(service.onchainRepo.updateOnchainReceiveResolution(id: idA ?? 0, txid: txidA))
        let first = service.onchainRepo.fetchLatestResolvedOnchainTxid()
        XCTAssertTrue(
            first == txidA || first == txidB,
            "Expected a resolved txid, got \(first ?? "nil")"
        )

        XCTAssertTrue(service.onchainRepo.updateOnchainReceiveResolution(id: idB ?? 0, txid: txidB))
        let second = service.onchainRepo.fetchLatestResolvedOnchainTxid()
        XCTAssertTrue(
            second == txidA || second == txidB,
            "Expected a resolved txid, got \(second ?? "nil")"
        )

        // Sanity: with both resolved, fetchLatestResolvedOnchainTxid
        // must not return nil.
        XCTAssertNotNil(second)
    }

    func testPaymentByTxid() throws {
        let txid = String(repeating: "c", count: 64)
        let paymentId = "onchain_receive_\(txid)"
        let address = "tb1qtestaddress"

        let ok = try service.paymentRepo.recordPayment(
            paymentId: paymentId,
            paymentType: "onchain",
            direction: "received",
            amountMsat: 100_000_000,
            amountUSD: 60.0,
            btcPrice: 60_000.0,
            counterparty: nil,
            status: "pending",
            txid: txid,
            address: address
        )
        XCTAssertTrue(ok)

        let payment = service.paymentRepo.payment(txid: txid)
        XCTAssertNotNil(payment)
        XCTAssertEqual(payment?.paymentId, paymentId)
        XCTAssertEqual(payment?.amountMsat, 100_000_000)
        XCTAssertEqual(payment?.address, address)
    }

    // MARK: - Price History Pruning Tests

    func testPruneHistoricalDataPurgesStalePriceHistory() throws {
        let now = Int64(Date().timeIntervalSince1970)
        let freshTimestamp = now - 3600 // 1 hour ago
        let staleTimestamp = now - 10_000_000 // > 90 days ago (90 days = 7,776,000 sec)

        // Insert fresh and stale price history records
        try service.rawSQL.execute(
            "INSERT INTO price_history (price, source, timestamp) VALUES (?, ?, ?)",
            params: [.real(60_000), .text("test_fresh"), .integer(freshTimestamp)]
        )
        try service.rawSQL.execute(
            "INSERT INTO price_history (price, source, timestamp) VALUES (?, ?, ?)",
            params: [.real(50_000), .text("test_stale"), .integer(staleTimestamp)]
        )

        // Re-init DatabaseService to trigger pruneHistoricalData()
        service = nil
        let newService = try DatabaseService(dataDir: dataDir)

        let history = try newService.priceRepo.getPriceHistory(hours: 24 * 365) // Query 1 year
        XCTAssertEqual(history.count, 1)
        XCTAssertEqual(history.first?.price, 60_000)
    }

    // MARK: - Query Indexes & Deduplication Tests

    func testCustomQueryIndexesAndUniquePaymentIdCreated() throws {
        let indexRows = try service.rawSQL
            .query("SELECT name FROM sqlite_master WHERE type='index' AND name='idx_payments_payment_id_unique';")
        XCTAssertEqual(
            indexRows.count,
            1,
            "Expected unique payment index idx_payments_payment_id_unique to exist"
        )

        let statusIndexRows = try service.rawSQL
            .query("SELECT name FROM sqlite_master WHERE type='index' AND name='idx_payments_status';")
        XCTAssertEqual(
            statusIndexRows.count,
            1,
            "Expected payment status index idx_payments_status to exist"
        )
    }

    func testUpgradeFromLegacyDBWithDuplicatePaymentIdsSucceeds() throws {
        // Build a pre-unique-index DB by hand: old payments schema containing
        // duplicate payment_id rows (the pre-NodeDirLock multi-writer shape) plus
        // legitimate empty-string payment_ids. Init must dedup then index, not throw.
        let legacyDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("test_legacy_\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: legacyDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: legacyDir) }

        var legacyDB: OpaquePointer?
        let path = legacyDir.appendingPathComponent(DatabaseService.dbFilename).path
        XCTAssertEqual(sqlite3_open(path, &legacyDB), SQLITE_OK)
        let legacySQL = """
        CREATE TABLE payments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            payment_id TEXT,
            payment_type TEXT NOT NULL DEFAULT 'manual',
            direction TEXT NOT NULL,
            amount_msat INTEGER NOT NULL,
            amount_usd REAL,
            btc_price REAL,
            counterparty TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            fee_msat INTEGER NOT NULL DEFAULT 0,
            txid TEXT,
            address TEXT,
            confirmations INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        INSERT INTO payments (payment_id, direction, amount_msat, status) VALUES ('dup_pay', 'received', 1000, 'pending');
        INSERT INTO payments (payment_id, direction, amount_msat, status) VALUES ('dup_pay', 'received', 1000, 'completed');
        INSERT INTO payments (payment_id, direction, amount_msat, status) VALUES ('', 'received', 1, 'completed');
        INSERT INTO payments (payment_id, direction, amount_msat, status) VALUES ('', 'received', 2, 'completed');
        """
        XCTAssertEqual(sqlite3_exec(legacyDB, legacySQL, nil, nil, nil), SQLITE_OK)
        sqlite3_close(legacyDB)

        let upgraded = try DatabaseService(dataDir: legacyDir)

        // First-recorded row per payment_id survives.
        let dupRows = try upgraded.rawSQL.query(
            "SELECT status FROM payments WHERE payment_id = 'dup_pay'"
        )
        XCTAssertEqual(dupRows.count, 1)
        XCTAssertEqual(dupRows.first?.first as? String, "pending")

        // Empty-string payment_ids are outside the index predicate and untouched.
        let emptyCount = try upgraded.rawSQL.query(
            "SELECT COUNT(*) FROM payments WHERE payment_id = ''"
        )
        XCTAssertEqual(emptyCount.first?.first as? Int64, 2)

        // The unique index exists and enforces from now on.
        let indexRows = try upgraded.rawSQL.query(
            "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_payments_payment_id_unique'"
        )
        XCTAssertEqual(indexRows.count, 1)
    }

    func testRecordPaymentIgnoresConcurrentDuplicate() throws {
        // Simulate losing the check-then-insert race: a row with the same
        // payment_id already exists when recordPayment's INSERT runs.
        XCTAssertTrue(try service.paymentRepo.recordPayment(
            paymentId: "race_pay", paymentType: "lightning", direction: "received",
            amountMsat: 5000, amountUSD: nil, btcPrice: nil, counterparty: nil, status: "completed"
        ))
        XCTAssertFalse(try service.paymentRepo.recordPayment(
            paymentId: "race_pay", paymentType: "lightning", direction: "received",
            amountMsat: 5000, amountUSD: nil, btcPrice: nil, counterparty: nil, status: "completed"
        ))
        let count = try service.rawSQL.query(
            "SELECT COUNT(*) FROM payments WHERE payment_id = 'race_pay'"
        )
        XCTAssertEqual(count.first?.first as? Int64, 1)
    }

    func testUniquePaymentIdIndexEnforcesDeduplication() throws {
        try service.rawSQL.execute(
            "INSERT INTO payments (payment_id, direction, amount_msat, status) VALUES ('unique_pay_1', 'sent', 100000, 'completed')"
        )

        // Second insert with exact same payment_id should throw a unique constraint error
        XCTAssertThrowsError(
            try service.rawSQL.execute(
                "INSERT INTO payments (payment_id, direction, amount_msat, status) VALUES ('unique_pay_1', 'sent', 100000, 'completed')"
            )
        )
    }

    // MARK: - Mobile trade-result protocol

    func testTradeProtocolHashAndFeeVectorsMatchRust() {
        let payload = #"{"type":"TRADE_V1","user_channel_id":"7","expected_usd":25.0}"#
        XCTAssertEqual(
            TradeProtocol.requestHash(Data(payload.utf8)),
            "c07dcdff3aae2fc7ebd4fb19a7f1cd60b8e61c94a89acd35c5c600935d671602"
        )

        XCTAssertEqual(
            TradeProtocol.expectedTradeFeeMsat(
                oldExpectedUSD: 50, newExpectedUSD: 99.5, quotePrice: 100_000
            ),
            500_000
        )
        XCTAssertEqual(
            TradeProtocol.expectedTradeFeeMsat(
                oldExpectedUSD: 100, newExpectedUSD: 50, quotePrice: 100_000
            ),
            500_000
        )
        XCTAssertEqual(
            TradeProtocol.expectedTradeFeeMsat(
                oldExpectedUSD: 1, newExpectedUSD: 0, quotePrice: 1_000_000
            ),
            1_000
        )
        XCTAssertEqual(
            TradeProtocol.expectedTradeFeeMsat(
                oldExpectedUSD: 1, newExpectedUSD: 0.1, quotePrice: 1_000_000
            ),
            1
        )
        XCTAssertEqual(TradeProtocol.normalizeExpectedUSD(0.009), 0)
        XCTAssertEqual(
            TradeProtocol.expectedTradeFeeMsat(
                oldExpectedUSD: 0, newExpectedUSD: 0, quotePrice: 100_000
            ),
            1
        )
    }

    func testSignedTradeControlRequiresExactBytesAndCompleteCorrelation() throws {
        let identifier = String(repeating: "ab", count: 32)
        let payload = """
        {"type":"SYNC_V1","channel_id":"\(
            identifier
        )","user_channel_id":"7","expected_usd":25.0,"backing_sats":31250,"sync_version":4,"trade_id":"\(
            identifier
        )","trade_payment_id":"\(identifier)","request_hash":"\(identifier)"}
        """
        let envelope = try JSONSerialization.data(withJSONObject: [
            "payload": payload,
            "signature": "valid"
        ])
        let parsed = TradeProtocol.parseSignedControl(
            data: envelope,
            expectedCounterparty: "peer",
            verifySignature: { bytes, signature, peer in
                bytes == Array(payload.utf8) && signature == "valid" && peer == "peer"
            }
        )
        guard case .sync(let sync) = parsed else {
            return XCTFail("Expected correlated SYNC_V1")
        }
        XCTAssertEqual(sync.correlation?.tradeId, identifier)

        let partialPayload = payload.replacingOccurrences(
            of: ",\"request_hash\":\"\(identifier)\"",
            with: ""
        )
        let partialEnvelope = try JSONSerialization.data(withJSONObject: [
            "payload": partialPayload,
            "signature": "valid"
        ])
        XCTAssertNil(TradeProtocol.parseSignedControl(
            data: partialEnvelope,
            expectedCounterparty: "peer",
            verifySignature: { _, _, _ in true }
        ))
        XCTAssertNil(TradeProtocol.parseSignedControl(
            data: envelope,
            expectedCounterparty: "peer",
            verifySignature: { _, _, _ in false }
        ))

        let fractionalPayload = payload.replacingOccurrences(
            of: "\"sync_version\":4",
            with: "\"sync_version\":1.5"
        )
        let fractionalEnvelope = try JSONSerialization.data(withJSONObject: [
            "payload": fractionalPayload,
            "signature": "valid"
        ])
        XCTAssertNil(TradeProtocol.parseSignedControl(
            data: fractionalEnvelope,
            expectedCounterparty: "peer",
            verifySignature: { _, _, _ in true }
        ))

        let booleanPayload = payload.replacingOccurrences(
            of: "\"expected_usd\":25.0",
            with: "\"expected_usd\":true"
        )
        let booleanEnvelope = try JSONSerialization.data(withJSONObject: [
            "payload": booleanPayload,
            "signature": "valid"
        ])
        XCTAssertNil(TradeProtocol.parseSignedControl(
            data: booleanEnvelope,
            expectedCounterparty: "peer",
            verifySignature: { _, _, _ in true }
        ))
    }

    func testPreparedTradeWaitsForCorrelatedAcceptanceBeforeUpdatingAllocation() throws {
        let channelId = String(repeating: "ab", count: 32)
        let paymentId = String(repeating: "cd", count: 32)
        let tradeId = String(repeating: "ef", count: 32)
        try service.channelRepo.saveChannel(
            channelId: channelId,
            userChannelId: "7",
            expectedUSD: 50,
            backingSats: 55_000,
            nativeSats: 45_000,
            note: nil,
            receiverSats: 100_000,
            latestPrice: 100_000
        )
        let prepared = try XCTUnwrap(TradeProtocol.prepare(
            channelId: channelId,
            userChannelId: "7",
            currentExpectedUSD: 50,
            currentBackingSats: 55_000,
            receiverSats: 100_000,
            action: "sell",
            amountUSD: 10,
            amountBTC: 0.000099,
            feeUSD: 0.1,
            newExpectedUSD: 59.9,
            quotePrice: 100_000,
            now: 1_786_310_000,
            tradeId: tradeId
        ))
        let tradeDbId = try service.channelRepo.recordPreparedTrade(prepared)
        let adopted = try XCTUnwrap(service.channelRepo.adoptUnattachedPreparedTrade(
            paymentId: paymentId,
            amountMsat: prepared.feeMsat,
            now: 1_786_310_001
        ))
        XCTAssertEqual(adopted.tradeDbId, tradeDbId)
        XCTAssertTrue(try service.channelRepo.tradePaymentExists(paymentId: paymentId))

        let beforeAcceptance = try XCTUnwrap(
            service.channelRepo.loadChannel(userChannelId: "7")
        )
        XCTAssertEqual(beforeAcceptance.expectedUSD, 50)
        XCTAssertEqual(beforeAcceptance.backingSats, 55_000)
        XCTAssertEqual(
            try service.channelRepo.unresolvedTradePayments()[paymentId]?.status,
            "fee_paid"
        )

        let sync = TradeControlMessage.Sync(
            channelId: channelId,
            userChannelId: "7",
            expectedUSD: prepared.newExpectedUSD,
            backingSats: prepared.newBackingSats + 1,
            syncVersion: 1,
            correlation: TradeCorrelation(
                tradeId: tradeId,
                tradePaymentId: paymentId,
                requestHash: prepared.requestHash
            )
        )
        XCTAssertTrue(try service.channelRepo.markTradeResponseNotCommittable(.sync(sync)))
        XCTAssertEqual(
            try service.channelRepo.unresolvedTradePayments()[paymentId]?.status,
            "uncertain"
        )
        let accepted = service.channelRepo.applyCorrelatedTradeAcceptance(sync)
        guard case .applied = accepted.status else {
            return XCTFail("Expected correlated acceptance to commit")
        }
        XCTAssertEqual(accepted.localBackingSats, prepared.newBackingSats)
        XCTAssertEqual(accepted.peerBackingSats, prepared.newBackingSats + 1)

        let afterAcceptance = try XCTUnwrap(
            service.channelRepo.loadChannel(userChannelId: "7")
        )
        XCTAssertEqual(afterAcceptance.expectedUSD, prepared.newExpectedUSD, accuracy: 0.000000001)
        XCTAssertEqual(afterAcceptance.backingSats, prepared.newBackingSats)
        XCTAssertEqual(afterAcceptance.syncVersion, 1)
        XCTAssertNil(try service.channelRepo.unresolvedTradePayments()[paymentId])

        let duplicate = service.channelRepo.applyCorrelatedTradeAcceptance(sync)
        guard case .duplicate = duplicate.status else {
            return XCTFail("Expected accepted response replay to be idempotent")
        }

        let superseded = try XCTUnwrap(TradeProtocol.prepare(
            channelId: channelId,
            userChannelId: "7",
            currentExpectedUSD: prepared.newExpectedUSD,
            currentBackingSats: prepared.newBackingSats,
            receiverSats: 100_000,
            action: "buy",
            amountUSD: 1,
            amountBTC: 0.0000099,
            feeUSD: 0.01,
            newExpectedUSD: prepared.newExpectedUSD - 1,
            quotePrice: 100_000,
            now: 1_786_310_001,
            tradeId: String(repeating: "aa", count: 32)
        ))
        let supersededDbId = try service.channelRepo.recordPreparedTrade(superseded)
        let supersededPaymentId = String(repeating: "bb", count: 32)
        XCTAssertTrue(try service.channelRepo.attachTradePaymentId(
            tradeDbId: supersededDbId,
            paymentId: supersededPaymentId
        ))
        let staleAcceptance = TradeControlMessage.Sync(
            channelId: channelId,
            userChannelId: "7",
            expectedUSD: superseded.newExpectedUSD,
            backingSats: superseded.newBackingSats,
            syncVersion: 1,
            correlation: TradeCorrelation(
                tradeId: superseded.tradeId,
                tradePaymentId: supersededPaymentId,
                requestHash: superseded.requestHash
            )
        )
        let staleResult = service.channelRepo.applyCorrelatedTradeAcceptance(staleAcceptance)
        guard case .applied = staleResult.status else {
            return XCTFail("A superseded acceptance must still resolve its trade")
        }
        XCTAssertEqual(staleResult.allocationApplied, false)
        let afterSuperseded = try XCTUnwrap(
            service.channelRepo.loadChannel(userChannelId: "7")
        )
        XCTAssertEqual(afterSuperseded.expectedUSD, prepared.newExpectedUSD, accuracy: 0.000000001)
        XCTAssertEqual(afterSuperseded.backingSats, prepared.newBackingSats)
        XCTAssertEqual(afterSuperseded.syncVersion, 1)
        XCTAssertNil(try service.channelRepo.unresolvedTradePayments()[supersededPaymentId])
    }

    func testFailedPaymentRecoversPreparedTradeWhenPaymentIdAttachmentWasLost() throws {
        let channelId = String(repeating: "12", count: 32)
        let paymentId = String(repeating: "34", count: 32)
        let prepared = try XCTUnwrap(TradeProtocol.prepare(
            channelId: channelId,
            userChannelId: "9",
            currentExpectedUSD: 25,
            currentBackingSats: 25_000,
            receiverSats: 100_000,
            action: "buy",
            amountUSD: 5,
            amountBTC: 0.0000495,
            feeUSD: 0.05,
            newExpectedUSD: 20,
            quotePrice: 100_000,
            now: 1_786_310_000,
            tradeId: String(repeating: "56", count: 32)
        ))
        let tradeDbId = try service.channelRepo.recordPreparedTrade(prepared)

        let failed = try XCTUnwrap(service.channelRepo.failUnattachedPreparedTrade(
            paymentId: paymentId,
            amountMsat: prepared.feeMsat,
            now: 1_786_310_001
        ))

        XCTAssertEqual(failed.tradeDbId, tradeDbId)
        XCTAssertEqual(failed.status, "send_failed")
        XCTAssertTrue(try service.channelRepo.tradePaymentExists(paymentId: paymentId))
        XCTAssertNil(try service.channelRepo.unresolvedTradePayments()[paymentId])
    }

    func testLegacyTradeSchemaMigratesWithoutLosingRows() throws {
        let legacyDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("test_trade_migration_\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: legacyDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: legacyDir) }

        var legacyDB: OpaquePointer?
        let path = legacyDir.appendingPathComponent(DatabaseService.dbFilename).path
        XCTAssertEqual(sqlite3_open(path, &legacyDB), SQLITE_OK)
        let legacySQL = """
        CREATE TABLE channels (
            channel_id TEXT PRIMARY KEY, user_channel_id TEXT UNIQUE,
            expected_usd REAL NOT NULL DEFAULT 0.0, stable_sats INTEGER NOT NULL DEFAULT 0,
            note TEXT, created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        CREATE TABLE trades (
            id INTEGER PRIMARY KEY AUTOINCREMENT, channel_id TEXT NOT NULL,
            action TEXT NOT NULL, amount_usd REAL NOT NULL, amount_btc REAL NOT NULL DEFAULT 0.0,
            btc_price REAL NOT NULL, fee_usd REAL NOT NULL DEFAULT 0.0,
            payment_id TEXT, status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        INSERT INTO channels (channel_id, user_channel_id, expected_usd, stable_sats)
            VALUES ('legacy-channel', 'legacy-user-channel', 25.0, 25000);
        INSERT INTO trades (channel_id, action, amount_usd, btc_price, status)
            VALUES ('legacy-channel', 'buy', 5.0, 100000.0, 'pending');
        """
        XCTAssertEqual(sqlite3_exec(legacyDB, legacySQL, nil, nil, nil), SQLITE_OK)
        sqlite3_close(legacyDB)

        let upgraded = try DatabaseService(dataDir: legacyDir)
        let channelColumns = try upgraded.rawSQL.query("PRAGMA table_info(channels)")
            .compactMap { $0[1] as? String }
        let tradeColumns = try upgraded.rawSQL.query("PRAGMA table_info(trades)")
            .compactMap { $0[1] as? String }
        XCTAssertTrue(channelColumns.contains("sync_version"))
        XCTAssertTrue(tradeColumns.contains("trade_id"))
        XCTAssertTrue(tradeColumns.contains("uncertainty_reason"))

        let legacyChannel = try XCTUnwrap(
            upgraded.channelRepo.loadChannel(userChannelId: "legacy-user-channel")
        )
        XCTAssertEqual(legacyChannel.expectedUSD, 25)
        XCTAssertEqual(legacyChannel.backingSats, 25_000)
        let legacyTradeCount = try upgraded.rawSQL.query("SELECT COUNT(*) FROM trades")
        XCTAssertEqual(legacyTradeCount.first?.first as? Int64, 1)
    }
}
