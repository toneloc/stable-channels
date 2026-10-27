import LDKNode
import XCTest
@testable import StableChannels

/// Port of Android SpliceEventRecoveryTest (#319): operation identity is saved with the pending
/// row, both writers commit splice events before acking, and only the captured operation can
/// ever receive a txid.
final class SpliceEventRecorderTests: XCTestCase {
    private var dataDir: URL!
    private var service: DatabaseService!
    private let ucid = "42"
    private let cid = "chan-a"
    private let oldFunding = String(repeating: "0a", count: 32)
    private let newFunding = String(repeating: "0b", count: 32)

    override func setUpWithError() throws {
        SQLitePaymentDatabase.didEnsureColumns = false
        dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("SpliceEventRecorderTests-\(UUID().uuidString)")
        service = try DatabaseService(dataDir: dataDir)
    }

    override func tearDownWithError() throws {
        service = nil
        try? FileManager.default.removeItem(at: dataDir)
    }

    // MARK: - Helpers

    @discardableResult
    private func begin(
        _ type: String = "splice_out",
        userChannelId: String? = nil,
        channelId: String? = nil,
        previousFunding: String?? = .none
    ) throws -> Int64 {
        try service.spliceRepo.recordPendingSplice(
            paymentType: type,
            amountMsat: 50_000_000,
            amountUSD: 25,
            btcPrice: 50000,
            address: type == "splice_out" ? "bc1qtest" : nil,
            channel: SpliceChannelIdentity(
                userChannelId: userChannelId ?? ucid,
                channelId: channelId ?? cid,
                previousFundingTxid: previousFunding ?? oldFunding
            )
        )
    }

    private func negotiated(_ txid: String, userChannelId: String? = nil, channelId: String? = nil)
        -> SpliceEventReceipt {
        SpliceEventReceipt(
            kind: .negotiated, userChannelId: userChannelId ?? ucid, channelId: channelId ?? cid,
            identityTxid: txid, funding: OutPoint(txid: txid, vout: 1)
        )
    }

    private func ready(eventFunding: String?, liveFunding: String? = nil) -> SpliceEventReceipt {
        let funding = (eventFunding ?? liveFunding).map { OutPoint(txid: $0, vout: 0) }
        return SpliceEventReceipt(
            kind: .ready, userChannelId: ucid, channelId: cid, identityTxid: eventFunding, funding: funding
        )
    }

    private func row(_ id: Int64) throws -> (status: String, txid: String?) {
        let rows = try service.rawSQL.query("SELECT status, txid FROM payments WHERE id = ?", params: [.integer(id)])
        return (rows[0].string(0), rows[0].optString(1))
    }

    private func eventCount() throws -> Int {
        try service.rawSQL.query("SELECT COUNT(*) FROM splice_events").first.map { Int($0.int64(0)) } ?? 0
    }

    private func ageOut(_ id: Int64) throws {
        try service.rawSQL.execute(
            "UPDATE payments SET created_at = created_at - 700 WHERE id = ?",
            params: [.integer(id)]
        )
    }

    // MARK: - Negotiation

    func testNegotiatedEventsAssignTheirOperationForBothDirectionsAndReplayIsIdempotent() throws {
        let out = try begin("splice_out")
        try service.spliceRepo.recordEvent(negotiated(newFunding))
        try service.spliceRepo.recordEvent(negotiated(newFunding))
        XCTAssertEqual(try row(out).txid, newFunding)
        XCTAssertEqual(try eventCount(), 1)

        XCTAssertTrue(service.spliceRepo.completeSplice(txid: newFunding))
        let into = try begin("splice_in", previousFunding: newFunding)
        let next = String(repeating: "0c", count: 32)
        try service.spliceRepo.recordEvent(negotiated(next))
        XCTAssertEqual(try row(into).txid, next)
        XCTAssertTrue(try service.spliceRepo.isPendingSplice(txid: next))
    }

    func testDelayedNegotiationRecoversItsExpiredOperation() throws {
        let id = try begin()
        try ageOut(id)
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())
        XCTAssertEqual(try row(id).status, "expired")

        try service.spliceRepo.recordEvent(negotiated(newFunding))

        XCTAssertEqual(try row(id).status, "pending")
        XCTAssertEqual(try row(id).txid, newFunding)
        XCTAssertTrue(try service.spliceRepo.hasPendingSplice())
    }

    func testUnrelatedChannelAndAmbiguousOperationsStayUnassigned() throws {
        let first = try begin()
        try service.spliceRepo.recordEvent(negotiated(newFunding, channelId: "other-channel"))
        XCTAssertNil(try row(first).txid)

        let second = try begin()
        try service.spliceRepo.recordEvent(negotiated(String(repeating: "0d", count: 32)))
        XCTAssertNil(try row(first).txid)
        XCTAssertNil(try row(second).txid)

        // A later operation never inherits an event saved before it existed.
        try service.spliceRepo.recoverAssignments()
        XCTAssertNil(try row(second).txid)
    }

    func testFailedOrCompletedOldEventsCannotClaimANewOperation() throws {
        let old = try begin()
        try service.spliceRepo.recordEvent(negotiated(newFunding))
        XCTAssertTrue(service.spliceRepo.completeSplice(txid: newFunding))

        let failed = try begin(previousFunding: newFunding)
        service.spliceRepo.failLatestPendingSplice()
        let fresh = try begin(previousFunding: newFunding)
        try service.spliceRepo.recordEvent(negotiated(newFunding))

        XCTAssertEqual(try row(old).status, "completed")
        XCTAssertEqual(try row(failed).status, "failed")
        XCTAssertNil(try row(fresh).txid)
    }

    func testNegotiationForTheExistingFundingCannotClaimANewSplice() throws {
        let id = try begin()
        try service.spliceRepo.recordEvent(negotiated(oldFunding))
        XCTAssertNil(try row(id).txid)
    }

    func testTxidAlreadyOnAnotherRowIsNeverAssignedTwice() throws {
        // A row the assignment never reclaims (only on-chain received rows are) keeps the txid.
        _ = try service.paymentRepo.recordPayment(
            paymentId: "send", paymentType: "onchain", direction: "sent", amountMsat: 1000,
            amountUSD: nil, btcPrice: nil, counterparty: nil, status: "pending", txid: newFunding
        )
        let id = try begin()
        try service.spliceRepo.recordEvent(negotiated(newFunding))
        XCTAssertNil(try row(id).txid)
    }

    // MARK: - Retry after an overdue attempt

    func testRetriedSpliceAfterAnExpiredAttemptGetsItsTxid() throws {
        let first = try begin()
        try ageOut(first)
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())
        let retry = try begin()

        try service.spliceRepo.recordEvent(negotiated(newFunding))

        XCTAssertEqual(try row(retry).txid, newFunding, "the pending retry is preferred over the expired attempt")
        XCTAssertEqual(try row(first).status, "expired")
        XCTAssertNil(try row(first).txid)
    }

    func testTwoExpiredAttemptsStayAmbiguous() throws {
        let first = try begin()
        let second = try begin()
        try ageOut(first)
        try ageOut(second)
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())

        try service.spliceRepo.recordEvent(negotiated(newFunding))

        XCTAssertNil(try row(first).txid)
        XCTAssertNil(try row(second).txid)
    }

    func testAcceptedSpliceRetiresOnlyOlderExpiredAttemptsOnItsChannel() throws {
        let stale = try begin()
        let otherChannel = try begin(userChannelId: "43", channelId: "chan-b")
        try ageOut(stale)
        try ageOut(otherChannel)
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())
        let accepted = try begin()

        try service.spliceRepo.retireSupersededSplices(byRowId: accepted)

        XCTAssertEqual(try row(stale).status, "failed")
        XCTAssertEqual(try row(otherChannel).status, "expired")
        XCTAssertEqual(try row(accepted).status, "pending")
    }

    func testSelfSendIsReadBackFromTheOperation() throws {
        let identity = SpliceChannelIdentity(userChannelId: ucid, channelId: cid, previousFundingTxid: oldFunding)
        let own = try service.spliceRepo.recordPendingSplice(
            paymentType: "splice_out", amountMsat: 1000, amountUSD: nil, btcPrice: nil,
            address: "bc1qown", selfSend: true, channel: identity
        )
        try service.spliceRepo.recordEvent(negotiated(newFunding))
        XCTAssertEqual(try row(own).txid, newFunding)
        XCTAssertTrue(try service.spliceRepo.isSelfSendSplice(txid: newFunding))
        XCTAssertFalse(try service.spliceRepo.isSelfSendSplice(txid: oldFunding))
    }

    // MARK: - ChannelReady

    func testReadyAloneRecoversWithUnchangedChannelIdUsingTheEventFunding() throws {
        let id = try begin()
        try service.spliceRepo.recordEvent(ready(eventFunding: newFunding))
        XCTAssertEqual(try row(id).txid, newFunding)
    }

    func testInitialChannelReadyCannotClaimAPendingSplice() throws {
        let id = try begin()
        try service.spliceRepo.recordEvent(ready(eventFunding: oldFunding))
        XCTAssertNil(try row(id).txid)
    }

    func testReadyWithoutKnownPreviousFundingCannotClaim() throws {
        let id = try begin(previousFunding: .some(nil))
        try service.spliceRepo.recordEvent(ready(eventFunding: newFunding))
        XCTAssertNil(try row(id).txid)
    }

    func testLegacyReadyWithoutFundingIsSavedThenResolvedAgainstTheSameChannel() throws {
        let id = try begin()
        try service.spliceRepo.recordEvent(ready(eventFunding: nil))
        XCTAssertNil(try row(id).txid)
        XCTAssertEqual(try eventCount(), 1)

        try service.spliceRepo.recoverReadyEvents { userChannelId, channelId in
            userChannelId == self.ucid && channelId == self.cid ? (self.newFunding, 0) : nil
        }
        XCTAssertEqual(try row(id).txid, newFunding)
    }

    func testLegacyReadyForTheOriginalFundingReleasesItsCapture() throws {
        let id = try begin()
        try service.spliceRepo.recordEvent(ready(eventFunding: nil))
        try service.spliceRepo.recoverReadyEvents { _, _ in (self.oldFunding, 0) }

        XCTAssertNil(try row(id).txid)
        let captured = try service.rawSQL.query("SELECT payment_row_id FROM splice_events").first?[0]
        XCTAssertNil(captured ?? nil)
        try ageOut(id)
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice(), "an abandoned operation must still expire")
    }

    func testLiveFundingLookupRequiresExactlyOneMatchingChannel() {
        XCTAssertNil(SpliceEventReceipt.liveFunding(in: [], userChannelId: ucid, channelId: cid))
    }

    func testEventKeyPrefersTheEventTxidThenTheResolvedFunding() {
        XCTAssertEqual(
            ready(eventFunding: oldFunding, liveFunding: newFunding).eventKey,
            "ready:42:chan-a:\(oldFunding)"
        )
        XCTAssertEqual(ready(eventFunding: nil, liveFunding: newFunding).eventKey, "ready:42:chan-a:\(newFunding)")
        XCTAssertEqual(ready(eventFunding: nil).eventKey, "ready:42:chan-a:unknown")
        XCTAssertEqual(negotiated(newFunding).eventKey, "negotiated:42:chan-a:\(newFunding)")
    }

    func testOutpointLessReadyAfterTheOpenStillReachesTheSplice() throws {
        try service.spliceRepo.recordEvent(ready(eventFunding: nil, liveFunding: oldFunding))
        let id = try begin()
        try service.spliceRepo.recordEvent(ready(eventFunding: nil, liveFunding: newFunding))

        XCTAssertEqual(try eventCount(), 2)
        XCTAssertEqual(try row(id).txid, newFunding)
    }

    // MARK: - Legacy rows and identity

    func testLegacyRowIsMatchedOnlyWhenItIsTheSingleCandidateOnTheSavedChannel() throws {
        func legacyRow() throws {
            _ = try service.paymentRepo.recordPayment(
                paymentId: nil, paymentType: "splice_out", direction: "sent", amountMsat: 1000,
                amountUSD: nil, btcPrice: nil, counterparty: nil, status: "pending"
            )
        }
        try legacyRow()
        try service.spliceRepo.recordEvent(negotiated(newFunding))
        XCTAssertNil(try service.spliceRepo.getPendingSpliceTxid(), "no saved channel row")

        try service.rawSQL.execute(
            "INSERT INTO channels (channel_id, user_channel_id) VALUES (?, ?)", params: [.text(cid), .text(ucid)]
        )
        let txid = String(repeating: "0e", count: 32)
        try service.spliceRepo.recordEvent(negotiated(txid))
        XCTAssertEqual(try service.spliceRepo.getPendingSpliceTxid(), txid)

        try legacyRow()
        try legacyRow()
        try service.spliceRepo.recordEvent(negotiated(String(repeating: "0f", count: 32)))
        XCTAssertEqual(try service.spliceRepo.getPendingSpliceTxid(), txid, "ambiguous legacy rows stay unassigned")
    }

    func testIncompleteIdentityRecordsNothing() throws {
        XCTAssertThrowsError(try begin(channelId: ""))
        XCTAssertFalse(try service.spliceRepo.hasPendingSplice())
    }

    func testOperationIdentityFailureRollsBackThePaymentToo() throws {
        try service.rawSQL.execute("DROP TABLE splice_operations")
        XCTAssertThrowsError(try begin())
        XCTAssertTrue(try service.rawSQL.query("SELECT 1 FROM payments").isEmpty)
    }

    func testDuplicateTxidRowsAreNeverReportedPending() throws {
        let id = try begin()
        try service.spliceRepo.recordEvent(negotiated(newFunding))
        try service.rawSQL.execute(
            "INSERT INTO payments (payment_type, direction, amount_msat, status, txid) VALUES ('splice_in', 'received', 1, 'pending', ?)",
            params: [.text(newFunding)]
        )
        XCTAssertEqual(try row(id).txid, newFunding)
        XCTAssertFalse(try service.spliceRepo.isPendingSplice(txid: newFunding))
    }

    // MARK: - Background writer

    private var backgroundDB: SQLitePaymentDatabase {
        SQLitePaymentDatabase(dbPath: dataDir.appendingPathComponent(DatabaseService.dbFilename).path)
    }

    func testBackgroundCommitsNegotiatedAndReadyBeforeAcknowledging() throws {
        let id = try begin()
        XCTAssertTrue(backgroundDB.recordSpliceEvent(ready(eventFunding: nil)))
        XCTAssertTrue(backgroundDB.recordSpliceEvent(negotiated(newFunding)))
        XCTAssertTrue(backgroundDB.recordSpliceEvent(negotiated(newFunding)))

        XCTAssertEqual(try row(id).txid, newFunding)
        XCTAssertEqual(try eventCount(), 2)
    }

    func testBackgroundReclaimsASelfSendDepositForTheSpliceRow() throws {
        // The websocket recorded a splice-out to our own address as a deposit first (#316).
        _ = try service.paymentRepo.recordPayment(
            paymentId: "deposit", paymentType: "onchain", direction: "received", amountMsat: 1000,
            amountUSD: nil, btcPrice: nil, counterparty: nil, status: "pending", txid: newFunding
        )
        let id = try begin()
        XCTAssertTrue(backgroundDB.recordSpliceEvent(negotiated(newFunding)))

        XCTAssertEqual(try row(id).txid, newFunding)
        XCTAssertEqual(
            try service.rawSQL.query("SELECT 1 FROM payments WHERE txid = ?", params: [.text(newFunding)]).count,
            1
        )
    }

    func testBackgroundDatabaseFailureLeavesTheEventUnacknowledged() {
        let missing = SQLitePaymentDatabase(dbPath: dataDir.appendingPathComponent("missing/none.db").path)
        XCTAssertFalse(missing.recordSpliceEvent(negotiated(newFunding)))
        XCTAssertNil(missing.hasPendingSplice())
    }

    func testBackgroundLeavesASpliceFailureForTheForegroundOnlyWhileAnOperationIsPending() throws {
        let failure = Event.spliceNegotiationFailed(channelId: cid, userChannelId: ucid, counterpartyNodeId: "peer")
        func mayAcknowledge() -> Bool {
            SpliceEventAcknowledgement.backgroundMayAcknowledge(failure, db: backgroundDB, channels: { [] })
        }
        XCTAssertTrue(mayAcknowledge())
        try begin()
        XCTAssertFalse(mayAcknowledge(), "the foreground owns a failure while a splice is pending")
    }
}
