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
