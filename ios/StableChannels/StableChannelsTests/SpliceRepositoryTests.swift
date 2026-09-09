import SQLite3
import XCTest
@testable import StableChannels

/// Covers the persist-at-initiation splice flow (issue #279, port of Android
/// PR #252): beginSpliceOut/sweepToChannel write a pending NULL-txid row
/// before the native splice call, and spliceNegotiated stamps the txid onto
/// that row via setPendingSpliceTxid instead of creating it.
final class SpliceRepositoryTests: XCTestCase {
    private var service: DatabaseService!
    private var dataDir: URL!

    override func setUp() {
        super.setUp()
        dataDir = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("SpliceRepositoryTests-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        service = try? DatabaseService(dataDir: dataDir)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dataDir)
        service = nil
        super.tearDown()
    }

    private func recordPendingSplice(type: String = "splice_out") throws {
        _ = try service.paymentRepo.recordPayment(
            paymentId: nil,
            paymentType: type,
            direction: type == "splice_out" ? "sent" : "received",
            amountMsat: 50_000_000,
            amountUSD: 25.0,
            btcPrice: 50_000.0,
            counterparty: nil,
            status: "pending",
            address: type == "splice_out" ? "bc1qtestaddress" : nil
        )
    }

    /// Backdates every NULL-txid splice row past the 600s no-txid expiry
    /// window so the next hasPendingSplice() call runs the sweep against it.
    private func ageOutPendingSplices(by seconds: Int64 = 700) throws {
        try service.rawSQL.execute(
            """
            UPDATE payments
            SET created_at = created_at - ?
            WHERE payment_type IN ('splice_in', 'splice_out') AND txid IS NULL
            """,
            params: [.integer(seconds)]
        )
    }

    private func spliceRows() throws -> [(id: Int64, status: String, txid: String?)] {
        try service.rawSQL.query(
            """
            SELECT id, status, txid FROM payments
            WHERE payment_type IN ('splice_in', 'splice_out')
            ORDER BY id ASC
            """
        ).map { ($0.int64(0), $0.string(1), $0.optString(2)) }
    }

    // MARK: - Persist at initiation

    func testInitiationRowIsPendingWithNilTxid() throws {
        try recordPendingSplice()

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "pending")
        XCTAssertNil(rows[0].txid)
        XCTAssertTrue(try service.spliceRepo.hasPendingSplice())
    }

    // MARK: - Txid stamping

    func testSetPendingSpliceTxidStampsPendingNullTxidRow() throws {
        try recordPendingSplice()

        try service.spliceRepo.setPendingSpliceTxid("txid-abc")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "pending")
        XCTAssertEqual(rows[0].txid, "txid-abc")
    }

    func testSetPendingSpliceTxidStampsSpliceInRowToo() throws {
        try recordPendingSplice(type: "splice_in")

        try service.spliceRepo.setPendingSpliceTxid("txid-in")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].txid, "txid-in")
    }

    func testSetPendingSpliceTxidNeverStampsFailedRow() throws {
        // A failed initiation row (native call threw) followed by a fresh
        // pending initiation row. The stamp must land on the pending row and
        // must never resurrect the failed one.
        try recordPendingSplice()
        service.spliceRepo.failLatestPendingSplice()
        try recordPendingSplice()

        try service.spliceRepo.setPendingSpliceTxid("txid-def")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows[0].status, "failed")
        XCTAssertNil(rows[0].txid)
        XCTAssertEqual(rows[1].status, "pending")
        XCTAssertEqual(rows[1].txid, "txid-def")
    }

    func testSetPendingSpliceTxidOnlyFailedRowsIsNoOp() throws {
        try recordPendingSplice()
        service.spliceRepo.failLatestPendingSplice()

        try service.spliceRepo.setPendingSpliceTxid("txid-ghi")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "failed")
        XCTAssertNil(rows[0].txid)
    }

    func testReplayedTxidIsNotStampedOntoASecondRow() throws {
        try recordPendingSplice()
        try service.spliceRepo.setPendingSpliceTxid("txid-once")
        // A second operation starts, then the old spliceNegotiated replays.
        try recordPendingSplice()

        try service.spliceRepo.setPendingSpliceTxid("txid-once")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows[0].txid, "txid-once")
        XCTAssertNil(rows[1].txid, "replayed txid must not bind to the newer row")
        XCTAssertEqual(rows[1].status, "pending")
    }

    // MARK: - Expiry sweep and late negotiation

    func testExpirySweepMarksRowExpiredNotFailed() throws {
        try recordPendingSplice()
        try ageOutPendingSplices()

        // hasPendingSplice runs the sweep; an expired row is no longer an
        // active pending splice.
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "expired")
        XCTAssertNil(rows[0].txid)
    }

    func testLateNegotiationRecoversExpiredRowThroughToCompletion() throws {
        // Mainnet splice negotiation can outlast the 600s no-txid window: the
        // initiation row is swept to 'expired', then spliceNegotiated finally
        // arrives. The stamp must recover the row, and confirmation must
        // complete it.
        try recordPendingSplice()
        try ageOutPendingSplices()
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())
        XCTAssertEqual(try spliceRows()[0].status, "expired")

        try service.spliceRepo.setPendingSpliceTxid("txid-late")

        var rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "pending", "stamping must recover the expired row")
        XCTAssertEqual(rows[0].txid, "txid-late")
        XCTAssertTrue(try service.spliceRepo.hasPendingSplice())

        XCTAssertTrue(service.spliceRepo.completeSplice(txid: "txid-late"))
        rows = try spliceRows()
        XCTAssertEqual(rows[0].status, "completed")
    }

    func testExplicitlyFailedRowIsNotRecoveredEvenAfterExpiryWindow() throws {
        // Explicit failure (native throw / spliceNegotiationFailed) is
        // terminal, unlike the passive expiry sweep: aging the row changes
        // nothing, and a late stamp must not resurrect it.
        try recordPendingSplice()
        service.spliceRepo.failLatestPendingSplice()
        try ageOutPendingSplices()
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())

        try service.spliceRepo.setPendingSpliceTxid("txid-necro")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "failed")
        XCTAssertNil(rows[0].txid)
    }

    func testExplicitFailureFinalizesExpiredRow() throws {
        // A late spliceNegotiationFailed for an already-expired initiation row
        // must make it terminal: failed, and no longer stampable.
        try recordPendingSplice()
        try ageOutPendingSplices()
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())

        service.spliceRepo.failLatestPendingSplice()
        try service.spliceRepo.setPendingSpliceTxid("txid-after-fail")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "failed")
        XCTAssertNil(rows[0].txid)
    }

    // MARK: - Failing the initiation row

    func testFailLatestPendingSpliceOnlyTouchesNullTxidRows() throws {
        // An older splice already negotiated (txid stamped, awaiting
        // confirmation) must survive a newer operation's pre-negotiation
        // failure.
        try recordPendingSplice()
        try service.spliceRepo.setPendingSpliceTxid("txid-live")
        try recordPendingSplice()

        service.spliceRepo.failLatestPendingSplice()

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows[0].status, "pending")
        XCTAssertEqual(rows[0].txid, "txid-live")
        XCTAssertEqual(rows[1].status, "failed")
        XCTAssertNil(rows[1].txid)
    }

    func testCompleteSpliceDoesNotFindFailedNullTxidRow() throws {
        try recordPendingSplice()
        service.spliceRepo.failLatestPendingSplice()

        XCTAssertFalse(service.spliceRepo.completeSplice(txid: "txid-jkl"))

        let rows = try spliceRows()
        XCTAssertEqual(rows[0].status, "failed")
    }
}
