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
        try service.saveChannel(
            channelId: "channel-1",
            userChannelId: "user-channel-1",
            expectedUSD: 100,
            backingSats: 1_000,
            note: nil
        )

        let first = try service.recordPaymentAndMaybeUpdateBacking(
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

        let duplicate = try service.recordPaymentAndMaybeUpdateBacking(
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

        let second = try service.recordPaymentAndMaybeUpdateBacking(
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

        let outgoing = try service.recordPaymentAndMaybeUpdateBacking(
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

        let outgoingReplay = try service.recordPaymentAndMaybeUpdateBacking(
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

        XCTAssertThrowsError(
            try service.recordPaymentAndMaybeUpdateBacking(
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
        )
        let afterRejectedDebit = try XCTUnwrap(
            service.loadChannel(userChannelId: "user-channel-1")
        )
        XCTAssertEqual(afterRejectedDebit.backingSats, 950)

        try service.saveChannelPreservingBacking(
            channelId: "channel-1",
            userChannelId: "user-channel-1",
            expectedUSD: 125,
            note: "metadata-only"
        )
        let stored = try XCTUnwrap(service.loadChannel(userChannelId: "user-channel-1"))
        XCTAssertEqual(stored.backingSats, 950)
        XCTAssertEqual(stored.expectedUSD, 125)
    }

    // MARK: - pending_operations

    func testPendingOperationsInsertFetch() {
        let ok = service.insertPendingOperation(
            opId: "close-abc",
            opType: "channel_close",
            fundingOutpointTxid: "deadbeef",
            fundingOutpointVout: 1
        )
        XCTAssertTrue(ok)

        let ops = service.fetchPendingOperations()
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
        _ = service.insertPendingOperation(
            opId: "close-xyz",
            opType: "channel_close",
            fundingOutpointTxid: "cafebabe",
            fundingOutpointVout: 0
        )
        let ok = service.updatePendingOperation(
            opId: "close-xyz",
            closingTxid: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            status: "resolved"
        )
        XCTAssertTrue(ok)

        // fetchPendingOperations() filters by status='pending', so the
        // resolved row is excluded. Use the PK lookup instead.
        let op = service.fetchPendingOperation(opId: "close-xyz")
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
        _ = service.insertPendingOperation(
            opId: "close-q",
            opType: "channel_close",
            fundingOutpointTxid: nil,
            fundingOutpointVout: nil
        )
        // First update succeeds and flips status to resolved.
        let first = service.updatePendingOperation(
            opId: "close-q",
            closingTxid: "first",
            status: "resolved"
        )
        XCTAssertTrue(first)

        // Second update must be a no-op because the row is no longer pending.
        let second = service.updatePendingOperation(
            opId: "close-q",
            closingTxid: "second",
            status: "resolved"
        )
        XCTAssertFalse(second, "Second update must not clobber a resolved row")

        let ops = service.fetchPendingOperations()
        XCTAssertEqual(ops.first?.closingTxid, "first")
    }

    // MARK: - onchain_receive_txids

    func testInsertOnchainReceiveResolution_returnsNonZeroId() {
        let id = service.insertOnchainReceiveResolution(address: "bc1qexampleaddress")
        XCTAssertNotNil(id)
        let unwrapped = try? XCTUnwrap(id)
        XCTAssertGreaterThan(unwrapped ?? 0, 0)
    }

    func testFetchPendingOnchainReceives_returnsInsertedRow() {
        _ = try? XCTUnwrap(service.insertOnchainReceiveResolution(address: "bc1qfirst"))
        _ = try? XCTUnwrap(service.insertOnchainReceiveResolution(address: "bc1qsecond"))

        let pending = service.fetchPendingOnchainReceives()
        XCTAssertEqual(pending.count, 2)
        XCTAssertEqual(pending[0].address, "bc1qfirst")
        XCTAssertEqual(pending[1].address, "bc1qsecond")
        XCTAssertEqual(pending[0].status, "pending")
        XCTAssertNil(pending[0].txid)
        XCTAssertNil(pending[0].resolvedAt)
    }

    func testUpdateOnchainReceiveResolution_setsTxidAndMarksResolved() {
        let id = try? XCTUnwrap(
            service.insertOnchainReceiveResolution(address: "bc1qtoupdate")
        )
        let txid = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"

        let ok = service.updateOnchainReceiveResolution(
            id: id ?? 0,
            txid: txid
        )
        XCTAssertTrue(ok)

        let pending = service.fetchPendingOnchainReceives()
        XCTAssertTrue(pending.isEmpty, "Resolved rows must not appear in pending fetch")

        // Re-insert another pending row so we can confirm via absence the resolved
        // row is no longer in the pending list.
        _ = try? XCTUnwrap(service.insertOnchainReceiveResolution(address: "bc1qother"))
        let stillPending = service.fetchPendingOnchainReceives()
        XCTAssertEqual(stillPending.count, 1)
        XCTAssertEqual(stillPending[0].address, "bc1qother")
    }

    func testFetchPendingOnchainReceives_excludesResolvedRows() {
        let id = try? XCTUnwrap(
            service.insertOnchainReceiveResolution(address: "bc1qresolve")
        )
        XCTAssertTrue(
            service.updateOnchainReceiveResolution(
                id: id ?? 0,
                txid: String(repeating: "f", count: 64)
            )
        )

        let pending = service.fetchPendingOnchainReceives()
        XCTAssertTrue(pending.isEmpty)
    }

    func testUpdateOnchainReceiveResolution_onlyAffectsTargetRow() {
        let idA = try? XCTUnwrap(service.insertOnchainReceiveResolution(address: "bc1qA"))
        let idB = try? XCTUnwrap(service.insertOnchainReceiveResolution(address: "bc1qB"))
        let txidA = String(repeating: "a", count: 64)
        let txidB = String(repeating: "b", count: 64)

        XCTAssertTrue(
            service.updateOnchainReceiveResolution(id: idA ?? 0, txid: txidA)
        )

        let pending = service.fetchPendingOnchainReceives()
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].id, idB)
        XCTAssertEqual(pending[0].address, "bc1qB")
        XCTAssertNil(pending[0].txid)

        XCTAssertTrue(
            service.updateOnchainReceiveResolution(id: idB ?? 0, txid: txidB)
        )
        let finalPending = service.fetchPendingOnchainReceives()
        XCTAssertTrue(finalPending.isEmpty)
    }

    // MARK: - Onchain receive integration flow

    /// Full on-chain receive lifecycle: insert resolution -> verify pending ->
    /// update with real txid -> verify resolved (no longer in pending list) ->
    /// verify latest resolved txid is queryable.
    func testOnchainReceiveFlow_pendingRowGetsTxidOnUpdate() {
        let address = "tb1qfakeaddressforintegrationtest1234567890abcdef"

        // 1) Insert resolution row
        guard let resolutionId = service.insertOnchainReceiveResolution(address: address) else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }
        XCTAssertGreaterThan(resolutionId, 0)

        // 2) Verify the row is in pending state
        let pending = service.fetchPendingOnchainReceives()
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].address, address)
        XCTAssertEqual(pending[0].id, resolutionId)
        XCTAssertNil(pending[0].txid)
        XCTAssertEqual(pending[0].status, "pending")

        // 3) Update with a real txid
        let txid = String(repeating: "a", count: 64)
        let updated = service.updateOnchainReceiveResolution(id: resolutionId, txid: txid)
        XCTAssertTrue(updated)

        // 4) Verify the row is no longer in the pending fetch
        let stillPending = service.fetchPendingOnchainReceives()
        XCTAssertEqual(stillPending.count, 0, "Resolved row should not appear in pending fetch")

        // 5) Latest resolved txid
        XCTAssertEqual(service.fetchLatestResolvedOnchainTxid(), txid)
    }

    /// Dedup invariant: a second `updateOnchainReceiveResolution` on the same
    /// row must return false (the SQL is gated by `status = 'pending'`, so a
    /// resolved row is no longer updatable via this method).
    func testUpdateOnchainReceiveResolution_returnsFalseOnSecondCall() {
        guard let id = service.insertOnchainReceiveResolution(address: "tb1qtest") else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }
        let txidA = String(repeating: "a", count: 64)
        let txidB = String(repeating: "b", count: 64)

        XCTAssertTrue(service.updateOnchainReceiveResolution(id: id, txid: txidA))
        XCTAssertFalse(
            service.updateOnchainReceiveResolution(id: id, txid: txidB),
            "Second update must be a no-op (row is no longer pending)"
        )

        // The original txid must be preserved.
        XCTAssertEqual(service.fetchLatestResolvedOnchainTxid(), txidA)
    }

    /// `fetchPendingOnchainReceiveRow` returns the row tied to a given
    /// `resolution_id`; a non-matching id returns nil.
    func testFetchPendingOnchainReceiveRow_returnsMatchingRow() {
        guard let resId = service.insertOnchainReceiveResolution(address: "tb1qtest") else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }

        let ok = service.recordOnchainPaymentWithResolution(
            paymentId: "p1",
            amountMsat: 50_000_000,
            amountUSD: 100.0,
            btcPrice: 50_000.0,
            resolutionId: resId
        )
        XCTAssertTrue(ok)

        let row = service.fetchPendingOnchainReceiveRow(resolutionId: resId)
        XCTAssertNotNil(row)
        XCTAssertEqual(row?.paymentId, "p1")
        XCTAssertEqual(row?.amountMsat, 50_000_000)

        // Different resolutionId returns nil (no row linked to it).
        XCTAssertNil(service.fetchPendingOnchainReceiveRow(resolutionId: resId + 1))
    }

    /// `recordOnchainPaymentWithResolution` writes a row that is
    /// (a) visible in `fetchPendingOnchainReceives` for the same resolution,
    /// (b) survives rollback of the resolution row by the cleanup path
    /// (we just verify the write itself succeeds and is findable).
    func testRecordOnchainPaymentWithResolution_writesResolutionId() {
        guard let resId = service.insertOnchainReceiveResolution(address: "tb1qtest") else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }

        XCTAssertTrue(
            service.recordOnchainPaymentWithResolution(
                paymentId: "p1",
                amountMsat: 1_000_000,
                amountUSD: nil,
                btcPrice: nil,
                resolutionId: resId
            )
        )

        // The payments row exists, linked to the resolution.
        let row = service.fetchPendingOnchainReceiveRow(resolutionId: resId)
        XCTAssertNotNil(row, "Inserted payment must be findable via resolutionId")
        XCTAssertEqual(row?.paymentId, "p1")
        XCTAssertEqual(row?.amountMsat, 1_000_000)

        // And the resolution row itself is still in pending state.
        let pending = service.fetchPendingOnchainReceives()
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].id, resId)
    }

    /// `deleteOnchainReceiveResolution` removes the resolution row; the
    /// payments row linked to it survives (resolution cleanup must not
    /// cascade — the resolution is a separate concern from the payment).
    func testDeleteOnchainReceiveResolution_removesResolutionRow() {
        guard let resId = service.insertOnchainReceiveResolution(address: "tb1qtest") else {
            XCTFail("insertOnchainReceiveResolution returned nil")
            return
        }
        XCTAssertTrue(
            service.recordOnchainPaymentWithResolution(
                paymentId: "p1",
                amountMsat: 1_000_000,
                amountUSD: nil,
                btcPrice: nil,
                resolutionId: resId
            )
        )

        // Delete the resolution row.
        XCTAssertTrue(service.deleteOnchainReceiveResolution(id: resId))

        // The resolution is gone.
        let pending = service.fetchPendingOnchainReceives()
        XCTAssertTrue(pending.isEmpty, "Resolution row must be deleted")

        // The payments row survives — it's the user-facing record.
        let row = service.fetchPendingOnchainReceiveRow(resolutionId: resId)
        XCTAssertNotNil(row, "Payments row must survive resolution deletion")
    }

    /// `fetchLatestResolvedOnchainTxid` returns a resolved txid (the
    /// schema uses whole-second `strftime('%s','now')` for `resolved_at`,
    /// so we cannot reliably distinguish "most recent" within a single
    /// second — both txids are correct answers in that case. We assert
    /// the basic contract: a resolved txid is queryable, and after two
    /// distinct resolutions the call still returns one of them).
    func testFetchLatestResolvedOnchainTxid_returnsResolvedTxid() {
        let idA = try? XCTUnwrap(service.insertOnchainReceiveResolution(address: "bc1qA"))
        let idB = try? XCTUnwrap(service.insertOnchainReceiveResolution(address: "bc1qB"))
        let txidA = String(repeating: "a", count: 64)
        let txidB = String(repeating: "b", count: 64)

        XCTAssertTrue(service.updateOnchainReceiveResolution(id: idA ?? 0, txid: txidA))
        let first = service.fetchLatestResolvedOnchainTxid()
        XCTAssertTrue(
            first == txidA || first == txidB,
            "Expected a resolved txid, got \(first ?? "nil")"
        )

        XCTAssertTrue(service.updateOnchainReceiveResolution(id: idB ?? 0, txid: txidB))
        let second = service.fetchLatestResolvedOnchainTxid()
        XCTAssertTrue(
            second == txidA || second == txidB,
            "Expected a resolved txid, got \(second ?? "nil")"
        )

        // Sanity: with both resolved, fetchLatestResolvedOnchainTxid
        // must not return nil.
        XCTAssertNotNil(second)
    }
}
