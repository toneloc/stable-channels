import LDKNode
import SQLite3
import XCTest
@testable import StableChannels

/// Covers the persist-at-initiation splice flow (issue #279, port of Android
/// PR #252): beginSpliceOut/sweepToChannel write a pending NULL-txid row
/// before the native splice call, and spliceNegotiated stamps the txid onto
/// that row via the splice event recorder instead of creating it.
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
        try service.spliceRepo.recordPendingSplice(
            paymentType: type,
            amountMsat: 50_000_000,
            amountUSD: 25.0,
            btcPrice: 50_000.0,
            address: type == "splice_out" ? "bc1qtestaddress" : nil,
            channel: SpliceChannelIdentity(userChannelId: "7", channelId: "channel", previousFundingTxid: nil)
        )
    }

    private func negotiate(_ txid: String) throws {
        try service.spliceRepo.recordEvent(SpliceEventReceipt(
            kind: .negotiated, userChannelId: "7", channelId: "channel",
            identityTxid: txid, funding: OutPoint(txid: txid, vout: 0)
        ))
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

    func testNegotiationStampsPendingNullTxidRow() throws {
        try recordPendingSplice()

        try negotiate("txid-abc")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "pending")
        XCTAssertEqual(rows[0].txid, "txid-abc")
    }

    func testNegotiationStampsSpliceInRowToo() throws {
        try recordPendingSplice(type: "splice_in")

        try negotiate("txid-in")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].txid, "txid-in")
    }

    func testNegotiationNeverStampsFailedRow() throws {
        // A failed initiation row (native call threw) followed by a fresh
        // pending initiation row. The stamp must land on the pending row and
        // must never resurrect the failed one.
        try recordPendingSplice()
        service.spliceRepo.failLatestPendingSplice()
        try recordPendingSplice()

        try negotiate("txid-def")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows[0].status, "failed")
        XCTAssertNil(rows[0].txid)
        XCTAssertEqual(rows[1].status, "pending")
        XCTAssertEqual(rows[1].txid, "txid-def")
    }

    func testNegotiationOnlyFailedRowsIsNoOp() throws {
        try recordPendingSplice()
        service.spliceRepo.failLatestPendingSplice()

        try negotiate("txid-ghi")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].status, "failed")
        XCTAssertNil(rows[0].txid)
    }

    func testReplayedTxidIsNotStampedOntoASecondRow() throws {
        try recordPendingSplice()
        try negotiate("txid-once")
        // A second operation starts, then the old spliceNegotiated replays.
        try recordPendingSplice()

        try negotiate("txid-once")

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

        try negotiate("txid-late")

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

        try negotiate("txid-necro")

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
        try negotiate("txid-after-fail")

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
        try negotiate("txid-live")
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

    // MARK: - Self-send reclaim (Android #316 review)

    private func recordWebSocketReceive(txid: String, address: String = "bc1qourownaddress") throws {
        _ = try service.paymentRepo.recordPayment(
            paymentId: "onchain_receive_\(txid)",
            paymentType: "onchain",
            direction: "received",
            amountMsat: 50_000_000,
            amountUSD: nil,
            btcPrice: nil,
            counterparty: nil,
            status: "pending",
            txid: txid,
            address: address
        )
    }

    private func rowCount(txid: String) throws -> Int {
        try service.rawSQL.query("SELECT 1 FROM payments WHERE txid = ?", params: [.text(txid)]).count
    }

    func testSelfSendReceiveRowIsReclaimedForTheSpliceRow() throws {
        // The websocket receive handler recorded a splice-out to our own address as a deposit
        // before the negotiated event could stamp the txid.
        try recordWebSocketReceive(txid: "self-send-tx")
        try recordPendingSplice()

        try negotiate("self-send-tx")

        let rows = try spliceRows()
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].txid, "self-send-tx")
        XCTAssertEqual(try rowCount(txid: "self-send-tx"), 1, "the duplicate deposit row is dropped")
        XCTAssertTrue(service.spliceRepo.completeSplice(txid: "self-send-tx"))
    }

    func testCompletedReceiveRowSharingTheSpliceTxidIsAlsoReclaimed() throws {
        try recordWebSocketReceive(txid: "self-send-tx")
        try service.rawSQL.execute("UPDATE payments SET status = 'completed' WHERE txid = 'self-send-tx'")
        try recordPendingSplice()

        try negotiate("self-send-tx")
        XCTAssertEqual(try spliceRows()[0].txid, "self-send-tx")
        XCTAssertEqual(try rowCount(txid: "self-send-tx"), 1)
    }

    func testReceiveRowSurvivesWhenNoSpliceRowCanTakeTheTxid() throws {
        try recordWebSocketReceive(txid: "plain-deposit-tx")

        try negotiate("plain-deposit-tx")
        XCTAssertEqual(try rowCount(txid: "plain-deposit-tx"), 1)
    }

    func testTxidOwnedByANonDepositRowIsNeverReclaimed() throws {
        _ = try service.paymentRepo.recordPayment(
            paymentId: "close-1",
            paymentType: "channel_close",
            direction: "received",
            amountMsat: 50_000_000,
            amountUSD: nil,
            btcPrice: nil,
            counterparty: nil,
            status: "pending",
            txid: "close-tx"
        )
        try recordPendingSplice()

        try negotiate("close-tx")
        XCTAssertNil(try spliceRows()[0].txid)
        XCTAssertEqual(try rowCount(txid: "close-tx"), 1)
    }

    func testTxidOnASpliceRowAndADepositRowIsNotReassigned() throws {
        try recordPendingSplice()
        try negotiate("txid-once")
        try recordWebSocketReceive(txid: "txid-once")
        try recordPendingSplice()

        try negotiate("txid-once")
        XCTAssertNil(try spliceRows()[1].txid)
        XCTAssertEqual(try rowCount(txid: "txid-once"), 2, "neither owner is deleted")
    }

    func testIsKnownReceiveAddressIsTrueOnceWeHaveReceivedThere() throws {
        try recordWebSocketReceive(txid: "older-tx", address: "bc1qourownaddress")

        XCTAssertTrue(try service.paymentRepo.isKnownReceiveAddress("bc1qourownaddress"))
    }

    func testIsKnownReceiveAddressIsFalseForAnAddressWeHaveNeverReceivedTo() throws {
        // A splice-out destination is recorded on the splice row, which must not count.
        try recordPendingSplice()

        XCTAssertFalse(try service.paymentRepo.isKnownReceiveAddress("bc1qtestaddress"))
        XCTAssertFalse(try service.paymentRepo.isKnownReceiveAddress("bc1qsomeoneelsesaddress"))
    }
}

private struct RecoverySpliceChecker: SpliceBroadcastChecking {
    let result: TxBroadcastStatus
    func checkStatus(txid _: String, endpointURLs _: [String]) async -> TxBroadcastStatus { result }
}

private struct CallbackSpliceChecker: SpliceBroadcastChecking {
    let check: @MainActor @Sendable () -> TxBroadcastStatus
    func checkStatus(txid _: String, endpointURLs _: [String]) async -> TxBroadcastStatus { await check() }
}

@MainActor
final class SpliceFailureRecoveryTests: XCTestCase {
    private var dataDir: URL!
    private var db: DatabaseService!
    private var app: AppState!
    private let txid = String(repeating: "ab", count: 32)
    private let channel = SpliceChannelIdentity(userChannelId: "7", channelId: "channel", previousFundingTxid: nil)

    override func setUp() async throws {
        dataDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        db = try DatabaseService(dataDir: dataDir)
        app = makeApp(.inconclusive)
    }

    override func tearDown() async throws {
        app = nil
        db = nil
        try FileManager.default.removeItem(at: dataDir)
    }

    private func makeApp(_ status: TxBroadcastStatus) -> AppState {
        let value = AppState(spliceBroadcastChecker: RecoverySpliceChecker(result: status))
        value.databaseService = db
        return value
    }

    private func beginNegotiatedSplice() throws {
        try app.beginSpliceOut(amountSats: 50_000, address: "bc1qtest", channel: channel)
        try db.spliceRepo.recordEvent(SpliceEventReceipt(
            kind: .negotiated, userChannelId: "7", channelId: "channel",
            identityTxid: txid, funding: OutPoint(txid: txid, vout: 0)
        ))
        app.spliceTxid = txid
    }

    private func queueFailure() throws {
        try db.spliceRepo.deferFailureCheck(txid: txid, channelId: "channel", userChannelId: "7")
    }

    private func status(_ txid: String) throws -> String? {
        try db.rawSQL.query("SELECT status FROM payments WHERE txid = ?", params: [.text(txid)]).first?.string(0)
    }

    func testFailureEventIsDurableBeforeAcknowledgement() throws {
        try beginNegotiatedSplice()
        let ack = EventAckToken()
        app.handleSpliceNegotiationFailed(channelId: "channel", userChannelId: "7", ackToken: ack)
        XCTAssertTrue(ack.shouldAck)
        XCTAssertEqual(try db.spliceRepo.pendingFailureChecks().map(\.txid), [txid])
    }

    func testQueueFailureKeepsLdkEventUnacknowledged() throws {
        try beginNegotiatedSplice()
        try db.rawSQL.execute("""
            CREATE TRIGGER fail_splice_inbox BEFORE INSERT ON pending_splice_failure_checks
            BEGIN SELECT RAISE(ABORT, 'injected disk failure'); END
        """)
        let ack = EventAckToken()
        app.handleSpliceNegotiationFailed(channelId: "channel", userChannelId: "7", ackToken: ack)
        XCTAssertFalse(ack.shouldAck)
        XCTAssertTrue(app.isSweeping)
        XCTAssertTrue(try db.spliceRepo.pendingFailureChecks().isEmpty)
    }

    func testInconclusiveCheckSurvivesRestartAndFinalizesExactRow() async throws {
        try beginNegotiatedSplice()
        try queueFailure()
        await app.retryPendingSpliceFailureChecks()
        XCTAssertTrue(app.isSweeping)
        XCTAssertEqual(try status(txid), "pending")
        XCTAssertEqual(try db.spliceRepo.pendingFailureChecks().count, 1)
        app = nil
        db = nil
        db = try DatabaseService(dataDir: dataDir)
        app = makeApp(.notFound)
        app.spliceTxid = txid
        await app.retryPendingSpliceFailureChecks()
        XCTAssertEqual(try status(txid), "failed")
        XCTAssertTrue(try db.spliceRepo.pendingFailureChecks().isEmpty)
        XCTAssertFalse(try db.spliceRepo.hasPendingSplice())
        XCTAssertFalse(app.isSweeping)
        XCTAssertNoThrow(try app.beginSpliceOut(amountSats: 10_000, address: "bc1qnext", channel: channel))
    }

    func testBroadcastTransactionKeepsConfirmationOwnership() async throws {
        app = makeApp(.exists)
        try beginNegotiatedSplice()
        try queueFailure()
        await app.retryPendingSpliceFailureChecks()
        XCTAssertTrue(app.isSweeping)
        XCTAssertEqual(try status(txid), "pending")
        XCTAssertTrue(try db.spliceRepo.pendingFailureChecks().isEmpty)
    }

    func testFinalizationFailureRetainsRecoveryObligation() async throws {
        app = makeApp(.notFound)
        try beginNegotiatedSplice()
        try queueFailure()
        try db.rawSQL.execute("""
            CREATE TRIGGER fail_splice_update BEFORE UPDATE OF status ON payments
            BEGIN SELECT RAISE(ABORT, 'injected disk failure'); END
        """)
        await app.retryPendingSpliceFailureChecks()
        XCTAssertEqual(try status(txid), "pending")
        XCTAssertEqual(try db.spliceRepo.pendingFailureChecks().count, 1)
        XCTAssertTrue(app.isSweeping)
        try db.rawSQL.execute("DROP TRIGGER fail_splice_update")
        await app.retryPendingSpliceFailureChecks()
        XCTAssertEqual(try status(txid), "failed")
        XCTAssertFalse(app.isSweeping)
    }

    func testNewSpliceDuringCheckIsNotFailedOrTornDown() async throws {
        app = AppState(spliceBroadcastChecker: CallbackSpliceChecker { [unowned self] in
            self.app.spliceTxid = nil
            self.app.cancelPendingSpliceStart()
            try! self.app.beginSpliceOut(amountSats: 10_000, address: "bc1qnew", channel: channel)
            return .notFound
        })
        app.databaseService = db
        try beginNegotiatedSplice()
        try queueFailure()
        await app.retryPendingSpliceFailureChecks()
        XCTAssertTrue(app.isSweeping)
        XCTAssertEqual(try status(txid), "pending")
        XCTAssertEqual(try db.spliceRepo.pendingFailureChecks().count, 1)
        let latest = try db.rawSQL.query("SELECT status, txid FROM payments ORDER BY id DESC LIMIT 1").first
        XCTAssertEqual(latest?.string(0), "pending")
        XCTAssertNil(latest?.optString(1))
    }

    func testExactTxidFinalizationLeavesNewInitiationAndCompletedRowsAlone() throws {
        try beginNegotiatedSplice()
        try queueFailure()
        app.spliceTxid = nil
        app.cancelPendingSpliceStart()
        try app.beginSpliceOut(amountSats: 10_000, address: "bc1qnew", channel: channel)
        XCTAssertTrue(try db.spliceRepo.failUnbroadcastSplice(txid: txid))
        XCTAssertTrue(try db.spliceRepo.hasPendingSplice(), "The newer NULL-txid splice is still pending")
        XCTAssertTrue(db.spliceRepo.completeSplice(txid: txid))
        try queueFailure()
        XCTAssertFalse(try db.spliceRepo.failUnbroadcastSplice(txid: txid))
        XCTAssertEqual(try status(txid), "completed")
        XCTAssertTrue(try db.spliceRepo.pendingFailureChecks().isEmpty)
    }

    // MARK: - Stale splice lock (Android c577ccb1 / 5fb6c459)

    private func expirePendingSplices() throws {
        try db.rawSQL.execute(
            "UPDATE payments SET created_at = created_at - 700 WHERE payment_type IN ('splice_in', 'splice_out')"
        )
    }

    func testStabilityTickReleasesALockWhosePreNegotiationRowExpired() throws {
        try app.beginSpliceOut(amountSats: 50_000, address: "bc1qtest", channel: channel)
        XCTAssertTrue(app.isSweeping)
        try expirePendingSplices()

        app.runStabilityCheck()

        XCTAssertFalse(app.isSweeping)
        XCTAssertNil(app.pendingSplice)
    }

    func testStabilityTickKeepsTheLockWhileThePreNegotiationRowIsPending() throws {
        try app.beginSpliceOut(amountSats: 50_000, address: "bc1qtest", channel: channel)

        app.runStabilityCheck()

        XCTAssertTrue(app.isSweeping)
        XCTAssertNotNil(app.pendingSplice)
    }

    func testStabilityTickKeepsTheLockOnceATxidWasNegotiated() throws {
        try app.beginSpliceOut(amountSats: 50_000, address: "bc1qtest", channel: channel)
        try expirePendingSplices()
        app.spliceTxid = txid

        app.runStabilityCheck()

        XCTAssertTrue(app.isSweeping)
    }

    func testFailingStaleLockCheckKeepsTheLock() throws {
        try app.beginSpliceOut(amountSats: 50_000, address: "bc1qtest", channel: channel)
        try expirePendingSplices()
        try db.rawSQL.execute(
            "CREATE TRIGGER refuse_expiry BEFORE UPDATE OF status ON payments BEGIN SELECT RAISE(ABORT, 'test'); END"
        )

        app.runStabilityCheck()
        XCTAssertTrue(app.isSweeping)

        try db.rawSQL.execute("DROP TRIGGER refuse_expiry")
        app.runStabilityCheck()
        XCTAssertFalse(app.isSweeping)
    }
}
