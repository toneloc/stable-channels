import XCTest
@testable import StableChannels

/// Ports Android's LightningPaymentRecovery / PaymentFailureRecorder tests: outbound payment
/// outcomes consumed by the NSE are committed before the event is acked, and the foreground
/// sweep repairs pending rows whose event was acked elsewhere.
final class LightningPaymentRecoveryTests: XCTestCase {
    private var dataDir: URL!
    private var service: DatabaseService!
    private var nse: SQLitePaymentDatabase!
    private let channelId = String(repeating: "ab", count: 32)
    private let paymentId = String(repeating: "ef", count: 32)

    override func setUpWithError() throws {
        try super.setUpWithError()
        dataDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("LightningPaymentRecoveryTests-\(UUID().uuidString)")
        service = try DatabaseService(dataDir: dataDir)
        SQLitePaymentDatabase.didEnsureColumns = false
        nse = SQLitePaymentDatabase(dbPath: dataDir.appendingPathComponent(DatabaseService.dbFilename).path)
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
    }

    override func tearDown() {
        service = nil
        nse = nil
        try? FileManager.default.removeItem(at: dataDir)
        super.tearDown()
    }

    // MARK: - Failure routing (pure)

    private func target(
        paymentId: String? = nil,
        pending: String? = nil,
        tradeExists: Bool = false,
        unattached: Bool = false,
        amount: UInt64? = nil
    ) -> (LightningPaymentFailureTarget, lookups: Int) {
        var lookups = 0
        let result = LightningPaymentRecovery.failureTarget(
            paymentId: paymentId ?? self.paymentId,
            pendingStabilityPaymentId: pending,
            tradeExists: tradeExists,
            hasUnattachedPreparedTrade: unattached,
            lookupAmountMsat: {
                lookups += 1
                return amount
            }
        )
        return (result, lookups)
    }

    func testFailureTargetPrefersOwnStabilityMarkerOverEverything() {
        let result = target(pending: paymentId, tradeExists: true, unattached: true, amount: 1)
        XCTAssertEqual(result.0, .stabilitySend)
        XCTAssertEqual(result.lookups, 0)
    }

    func testFailureTargetIgnoresAnotherPaymentsOrAnUnresolvedStabilityMarker() {
        XCTAssertEqual(target(pending: "10").0, .payment)
        XCTAssertEqual(target(pending: "").0, .payment)
        XCTAssertEqual(target(paymentId: "", pending: "").0, .payment)
    }

    func testFailureTargetUsesDurableTradeIdBeforeAmountLookup() {
        let result = target(tradeExists: true, unattached: true, amount: 1_000)
        XCTAssertEqual(result.0, .trade)
        XCTAssertEqual(result.lookups, 0)
    }

    func testFailureTargetLooksUpAmountOnlyForAnUnattachedTrade() {
        XCTAssertEqual(target(unattached: true, amount: 1_000).0, .unattachedTrade(amountMsat: 1_000))
        XCTAssertEqual(target(unattached: true, amount: 0).0, .unattachedTrade(amountMsat: 0))
        XCTAssertEqual(target(unattached: true, amount: nil).0, .payment)
        let none = target(unattached: false, amount: 1_000)
        XCTAssertEqual(none.0, .payment)
        XCTAssertEqual(none.lookups, 0)
    }

    // MARK: - Reason codes

    func testReasonCodeMatchesAndroidEnumNames() {
        let expected = [
            "recipientRejected": "RECIPIENT_REJECTED",
            "userAbandoned": "USER_ABANDONED",
            "retriesExhausted": "RETRIES_EXHAUSTED",
            "paymentExpired": "PAYMENT_EXPIRED",
            "routeNotFound": "ROUTE_NOT_FOUND",
            "unexpectedError": "UNEXPECTED_ERROR",
            "unknownRequiredFeatures": "UNKNOWN_REQUIRED_FEATURES",
            "invoiceRequestExpired": "INVOICE_REQUEST_EXPIRED",
            "invoiceRequestRejected": "INVOICE_REQUEST_REJECTED",
            "blindedPathCreationFailed": "BLINDED_PATH_CREATION_FAILED"
        ]
        for (caseName, code) in expected {
            XCTAssertEqual(LightningPaymentRecovery.reasonCode(caseName: caseName), code)
        }
        XCTAssertNil(LightningPaymentRecovery.reasonCode(caseName: nil))
        XCTAssertNil(LightningPaymentRecovery.reasonCode(caseName: ""))
    }

    // MARK: - Foreground sweep

    func testReconcileCountsOnlyResolvedAndPersistedRows() {
        var applied: [String: LightningPaymentResolution] = [:]
        let repaired = LightningPaymentRecovery.reconcilePending(
            paymentIds: ["ok", "pending", "fail", "write-error"],
            lookup: { id in
                switch id {
                case "ok": return .succeeded(feeMsat: 1_234)
                case "pending": return nil
                default: return .failed(reasonCode: nil)
                }
            },
            apply: { id, resolution in
                guard id != "write-error" else { return false }
                applied[id] = resolution
                return true
            }
        )
        XCTAssertEqual(repaired, 2)
        XCTAssertEqual(applied, ["ok": .succeeded(feeMsat: 1_234), "fail": .failed(reasonCode: nil)])
        XCTAssertEqual(
            LightningPaymentRecovery.reconcilePending(paymentIds: [], lookup: { _ in nil }, apply: { _, _ in true }),
            0
        )
    }

    func testPendingOutgoingLightningQuerySelectsOnlyPendingSentLightningOldestFirst() throws {
        try recordPayment("b", type: "bolt12", createdAt: 20)
        try recordPayment("a", type: "lightning", createdAt: 10)
        try recordPayment("c", type: "lightning", createdAt: 30)
        try recordPayment("received", type: "lightning", direction: "received")
        try recordPayment("done", type: "lightning", status: "completed")
        try recordPayment("chain", type: "onchain")
        try recordPayment("stability", type: "stability")
        try recordPayment("", type: "lightning")

        XCTAssertEqual(try service.paymentRepo.pendingOutgoingLightningPaymentIds(), ["a", "b", "c"])
        XCTAssertEqual(try service.paymentRepo.pendingOutgoingLightningPaymentIds(limit: 2), ["a", "b"])
        XCTAssertEqual(try service.paymentRepo.pendingOutgoingLightningPaymentIds(limit: 0), [])
    }

    // MARK: - NSE success before ack

    func testBackgroundSuccessRepairsPendingHistoryAndSurvivesRelaunch() throws {
        try recordPayment(paymentId, type: "lightning")
        XCTAssertTrue(nse.recordOutgoingPaymentSuccess(paymentId: paymentId, feeMsat: 1_234))

        service = try DatabaseService(dataDir: dataDir)
        let row = try XCTUnwrap(service.paymentRepo.payment(paymentId: paymentId))
        XCTAssertEqual(row.status, "completed")
        XCTAssertEqual(row.feeMsat, 1_234)
        XCTAssertTrue(try service.paymentRepo.pendingOutgoingLightningPaymentIds().isEmpty)
        // A replayed event cannot rewrite the terminal row.
        XCTAssertTrue(nse.recordOutgoingPaymentSuccess(paymentId: paymentId, feeMsat: 9))
        XCTAssertEqual(service.paymentRepo.payment(paymentId: paymentId)?.feeMsat, 1_234)
    }

    func testBackgroundTradeFeeSuccessMarksTheDurableTradeFeePaid() throws {
        let tradeDbId = try service.channelRepo.recordPreparedTrade(prepare())
        XCTAssertTrue(try service.channelRepo.attachTradePaymentId(tradeDbId: tradeDbId, paymentId: paymentId))

        XCTAssertTrue(nse.recordOutgoingPaymentSuccess(paymentId: paymentId, feeMsat: 250))
        XCTAssertEqual(try service.channelRepo.unresolvedTradePayments()[paymentId]?.status, "fee_paid")
    }

    func testSuccessWithoutPaymentIdIsAckedWithoutWriting() {
        XCTAssertTrue(nse.recordOutgoingPaymentSuccess(paymentId: "", feeMsat: nil))
    }

    // MARK: - NSE failure before ack

    func testFailedFeeIsTerminalWithItsReasonAndSurvivesRestart() throws {
        let tradeDbId = try service.channelRepo.recordPreparedTrade(prepare())
        XCTAssertTrue(try service.channelRepo.attachTradePaymentId(tradeDbId: tradeDbId, paymentId: paymentId))

        XCTAssertTrue(recordFailure(reason: "RETRIES_EXHAUSTED"))
        service = try DatabaseService(dataDir: dataDir)
        XCTAssertEqual(try tradeRow(), ["send_failed", "RETRIES_EXHAUSTED"])
        XCTAssertNil(try service.channelRepo.unresolvedTradePayments()[paymentId])
        // Replayed failure keeps the original durable reason.
        XCTAssertTrue(recordFailure(reason: "ROUTE_NOT_FOUND"))
        XCTAssertEqual(try tradeRow(), ["send_failed", "RETRIES_EXHAUSTED"])
    }

    func testFailedUnattachedFeeIsRecoveredWithItsReason() throws {
        let trade = prepare()
        _ = try service.channelRepo.recordPreparedTrade(trade)

        XCTAssertTrue(recordFailure(reason: "ROUTE_NOT_FOUND", amount: trade.feeMsat))
        XCTAssertEqual(try tradeRow(), ["send_failed", "ROUTE_NOT_FOUND"])
    }

    func testUnrelatedPaymentCannotFailAnUnattachedTrade() throws {
        let trade = prepare()
        _ = try service.channelRepo.recordPreparedTrade(trade)
        try recordPayment(paymentId, type: "lightning")

        XCTAssertTrue(recordFailure(reason: nil, amount: trade.feeMsat + 1))
        XCTAssertEqual(try tradeRow(), ["prepared", ""])
        XCTAssertEqual(service.paymentRepo.payment(paymentId: paymentId)?.status, "failed")
    }

    func testLateFailureCannotUndoAPaidFee() throws {
        let tradeDbId = try service.channelRepo.recordPreparedTrade(prepare())
        XCTAssertTrue(try service.channelRepo.attachTradePaymentId(tradeDbId: tradeDbId, paymentId: paymentId))
        XCTAssertTrue(try service.channelRepo.markTradeFeePaid(paymentId: paymentId))

        XCTAssertTrue(recordFailure(reason: "RETRIES_EXHAUSTED"))
        XCTAssertEqual(try tradeRow(), ["fee_paid", ""])
    }

    func testStabilityFailureReleasesOnlyItsOwnSendMarker() throws {
        XCTAssertTrue(service.stabilityRepo.claimPendingSend(amountMsat: 10_000, price: 100_000))
        XCTAssertTrue(service.stabilityRepo.setPendingSendPaymentId(paymentId))

        XCTAssertTrue(nse.recordOutgoingPaymentFailure(
            paymentId: String(repeating: "10", count: 32), reasonCode: nil, lookupAmountMsat: { nil }
        ))
        XCTAssertNotNil(service.stabilityRepo.loadPendingSend())
        XCTAssertTrue(recordFailure(reason: nil))
        XCTAssertNil(service.stabilityRepo.loadPendingSend())
        XCTAssertEqual(try service.channelRepo.loadChannel(userChannelId: "7")?.backingSats, 55_000)
    }

    func testOrdinaryFailureMarksOnlyAPendingRowFailed() throws {
        try recordPayment(paymentId, type: "lightning")
        XCTAssertTrue(recordFailure(reason: nil))
        XCTAssertEqual(service.paymentRepo.payment(paymentId: paymentId)?.status, "failed")
    }

    func testPersistenceFailureIsReportedSoTheEventIsNotAcked() throws {
        let tradeDbId = try service.channelRepo.recordPreparedTrade(prepare())
        XCTAssertTrue(try service.channelRepo.attachTradePaymentId(tradeDbId: tradeDbId, paymentId: paymentId))
        try service.rawSQL.execute(
            "CREATE TRIGGER refuse_failure BEFORE UPDATE ON trades BEGIN SELECT RAISE(ABORT, 'test'); END"
        )

        XCTAssertFalse(recordFailure(reason: nil))
        XCTAssertFalse(nse.recordOutgoingPaymentSuccess(paymentId: paymentId, feeMsat: 1))
        XCTAssertEqual(try tradeRow(), ["sent", ""])
    }

    // MARK: - Foreground reason codes

    func testForegroundSendFailedStoresReasonCode() throws {
        let tradeDbId = try service.channelRepo.recordPreparedTrade(prepare())
        XCTAssertTrue(try service.channelRepo.markTradeSendFailed(tradeDbId: tradeDbId, reasonCode: "PAYMENT_EXPIRED"))
        XCTAssertEqual(try tradeRow(), ["send_failed", "PAYMENT_EXPIRED"])
    }

    func testForegroundUnattachedFailureStoresReasonCode() throws {
        let trade = prepare()
        _ = try service.channelRepo.recordPreparedTrade(trade)
        XCTAssertNotNil(try service.channelRepo.failUnattachedPreparedTrade(
            paymentId: paymentId, amountMsat: trade.feeMsat, reasonCode: "ROUTE_NOT_FOUND"
        ))
        XCTAssertEqual(try tradeRow(), ["send_failed", "ROUTE_NOT_FOUND"])
    }

    // MARK: - Helpers

    private func prepare() -> PreparedMobileTrade {
        TradeProtocol.prepare(
            channelId: channelId,
            userChannelId: "7",
            currentExpectedUSD: 50,
            currentBackingSats: 55_000,
            receiverSats: 100_000,
            spendableSats: 100_000,
            action: "sell",
            amountUSD: 10,
            amountBTC: 0.000099,
            feeUSD: 0.1,
            newExpectedUSD: 59.9,
            quotePrice: 100_000,
            tradeId: String(repeating: "cd", count: 32)
        )!
    }

    private func recordFailure(reason: String?, amount: UInt64? = nil) -> Bool {
        nse.recordOutgoingPaymentFailure(paymentId: paymentId, reasonCode: reason, lookupAmountMsat: { amount })
    }

    private func recordPayment(
        _ id: String,
        type: String,
        direction: String = "sent",
        status: String = "pending",
        createdAt: Int64? = nil
    ) throws {
        _ = try service.paymentRepo.recordPayment(
            paymentId: id.isEmpty ? nil : id,
            paymentType: type,
            direction: direction,
            amountMsat: 50_000,
            amountUSD: nil,
            btcPrice: nil,
            counterparty: nil,
            status: status
        )
        if let createdAt {
            try service.rawSQL.execute(
                "UPDATE payments SET created_at = ? WHERE payment_id = ?",
                params: [.integer(createdAt), .text(id)]
            )
        }
    }

    private func tradeRow() throws -> [String] {
        let row = try XCTUnwrap(service.rawSQL.query("SELECT status, reason_code FROM trades").first)
        return [row.string(0), row.string(1)]
    }
}

final class LightningPaymentResolutionRowTests: XCTestCase {
    func testResolutionsWriteTheirRowStatusAndFee() {
        XCTAssertEqual(LightningPaymentResolution.succeeded(feeMsat: 12).rowStatus, "completed")
        XCTAssertEqual(LightningPaymentResolution.succeeded(feeMsat: 12).rowFeeMsat, 12)
        XCTAssertEqual(LightningPaymentResolution.failed(reasonCode: "X").rowStatus, "failed")
        XCTAssertNil(LightningPaymentResolution.failed(reasonCode: "X").rowFeeMsat)
    }
}
