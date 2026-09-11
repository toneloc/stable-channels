import XCTest
import LDKNode
@testable import StableChannels

final class TradeSyncParityTests: XCTestCase {
    private var service: DatabaseService!
    private var dataDir: URL!

    override func setUp() {
        super.setUp()
        dataDir = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("TradeSyncParityTests-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        service = try? DatabaseService(dataDir: dataDir)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dataDir)
        service = nil
        super.tearDown()
    }

    // MARK: - Generation Bump Skip on Resume

    func testShouldSkipGenerationBumpOnResume() {
        // Active monitor with matching txid -> skip bump
        XCTAssertTrue(
            AppState.shouldSkipGenerationBumpOnResume(
                monitorActive: true,
                monitoredTxid: "txid123",
                resumedTxid: "txid123"
            )
        )

        // Matching txid with leading/trailing whitespace -> skip bump
        XCTAssertTrue(
            AppState.shouldSkipGenerationBumpOnResume(
                monitorActive: true,
                monitoredTxid: "txid123",
                resumedTxid: "  txid123\n"
            )
        )

        // Monitor not active -> do not skip bump
        XCTAssertFalse(
            AppState.shouldSkipGenerationBumpOnResume(
                monitorActive: false,
                monitoredTxid: "txid123",
                resumedTxid: "txid123"
            )
        )

        // Monitored txid differs from resumed -> do not skip bump
        XCTAssertFalse(
            AppState.shouldSkipGenerationBumpOnResume(
                monitorActive: true,
                monitoredTxid: "txid123",
                resumedTxid: "txid456"
            )
        )

        // Monitored txid nil -> do not skip bump
        XCTAssertFalse(
            AppState.shouldSkipGenerationBumpOnResume(
                monitorActive: true,
                monitoredTxid: nil,
                resumedTxid: "txid123"
            )
        )
    }

    // MARK: - isChannelClosed

    func testIsChannelClosedDetectsPaymentsAndPendingOps() throws {
        let channelId = String(repeating: "ab", count: 32)
        let userChannelId = "user-close-test-1"

        // Initially not closed
        XCTAssertFalse(service.channelRepo.isChannelClosed(channelId: channelId, userChannelId: userChannelId))

        // Record a channel_close payment by userChannelId opId
        _ = try service.paymentRepo.recordPayment(
            paymentId: "close-\(userChannelId)",
            paymentType: "channel_close",
            direction: "received",
            amountMsat: 50_000_000,
            amountUSD: 50.0,
            btcPrice: 100_000,
            counterparty: nil,
            status: "completed",
            txid: String(repeating: "01", count: 32)
        )

        // Now returns closed
        XCTAssertTrue(service.channelRepo.isChannelClosed(channelId: channelId, userChannelId: userChannelId))
    }

    func testIsChannelClosedDetectsPendingOperations() {
        let channelId = String(repeating: "ab", count: 32)
        let userChannelId = "user-close-test-2"

        XCTAssertFalse(service.channelRepo.isChannelClosed(channelId: channelId, userChannelId: userChannelId))

        // Record pending_operation for channel_close
        service.pendingOpRepo.insertPendingOperation(
            opId: "close-\(userChannelId)",
            opType: "channel_close",
            fundingOutpointTxid: String(repeating: "02", count: 32),
            fundingOutpointVout: 0
        )

        XCTAssertTrue(service.channelRepo.isChannelClosed(channelId: channelId, userChannelId: userChannelId))
    }

    // MARK: - applyCorrelatedTradeAcceptance

    func testCorrelatedTradeAcceptanceSucceedsWithDriftedUserChannelId() throws {
        let channelId = String(repeating: "ab", count: 32)
        let tradeId = String(repeating: "cd", count: 32)
        let paymentId = String(repeating: "ef", count: 32)
        let channelDriftedUserChannelId = "channel-row-user-id-drifted"
        let tradeUserChannelId = "trade-user-channel-id"

        // Save channel with drifted user_channel_id in channels table
        try service.channelRepo.saveChannel(
            channelId: channelId,
            userChannelId: channelDriftedUserChannelId,
            expectedUSD: 50.0,
            backingSats: 55_000,
            nativeSats: 45_000,
            note: nil,
            receiverSats: 100_000,
            latestPrice: 100_000
        )

        // Prepare trade using tradeUserChannelId
        let prepared = try XCTUnwrap(TradeProtocol.prepare(
            channelId: channelId,
            userChannelId: tradeUserChannelId,
            currentExpectedUSD: 50.0,
            currentBackingSats: 55_000,
            receiverSats: 100_000,
            spendableSats: 100_000,
            action: "sell",
            amountUSD: 10.0,
            amountBTC: 0.000099,
            feeUSD: 0.1,
            newExpectedUSD: 59.9,
            quotePrice: 100_000,
            now: 1_786_310_000,
            tradeId: tradeId
        ))
        _ = try service.channelRepo.recordPreparedTrade(prepared)
        _ = try service.channelRepo.adoptUnattachedPreparedTrade(
            paymentId: paymentId,
            amountMsat: prepared.feeMsat,
            now: 1_786_310_001
        )

        // Incoming sync has tradeUserChannelId matching trade, while channels table has drifted
        let sync = TradeControlMessage.Sync(
            channelId: channelId,
            userChannelId: tradeUserChannelId,
            expectedUSD: prepared.newExpectedUSD,
            backingSats: prepared.newBackingSats,
            syncVersion: 1,
            correlation: TradeCorrelation(
                tradeId: prepared.tradeId,
                tradePaymentId: paymentId,
                requestHash: prepared.requestHash
            )
        )

        // Looking up channel by channel_id succeeds even though channels.user_channel_id drifted
        let result = service.channelRepo.applyCorrelatedTradeAcceptance(sync)
        XCTAssertEqual(result.status, TradeControlApplyStatus.applied)
    }

    func testCorrelatedTradeAcceptanceReturnsInvalidIfChannelMissingAndClosed() throws {
        let channelId = String(repeating: "ab", count: 32)
        let tradeId = String(repeating: "cd", count: 32)
        let paymentId = String(repeating: "ef", count: 32)
        let userChannelId = "user-channel-closed-1"

        // Save channel and prepare trade
        try service.channelRepo.saveChannel(
            channelId: channelId,
            userChannelId: userChannelId,
            expectedUSD: 50.0,
            backingSats: 55_000,
            nativeSats: 45_000,
            note: nil,
            receiverSats: 100_000,
            latestPrice: 100_000
        )
        let prepared = try XCTUnwrap(TradeProtocol.prepare(
            channelId: channelId,
            userChannelId: userChannelId,
            currentExpectedUSD: 50.0,
            currentBackingSats: 55_000,
            receiverSats: 100_000,
            spendableSats: 100_000,
            action: "sell",
            amountUSD: 10.0,
            amountBTC: 0.000099,
            feeUSD: 0.1,
            newExpectedUSD: 59.9,
            quotePrice: 100_000,
            now: 1_786_310_000,
            tradeId: tradeId
        ))
        _ = try service.channelRepo.recordPreparedTrade(prepared)
        _ = try service.channelRepo.adoptUnattachedPreparedTrade(
            paymentId: paymentId,
            amountMsat: prepared.feeMsat,
            now: 1_786_310_001
        )

        // Delete channel to simulate it having been closed
        try service.channelRepo.deleteChannel(userChannelId: userChannelId)

        // Mark channel as closed in payments
        _ = try service.paymentRepo.recordPayment(
            paymentId: "close-\(userChannelId)",
            paymentType: "channel_close",
            direction: "received",
            amountMsat: 10_000_000,
            amountUSD: 10.0,
            btcPrice: 100_000,
            counterparty: nil,
            status: "completed",
            txid: String(repeating: "03", count: 32)
        )

        let sync = TradeControlMessage.Sync(
            channelId: channelId,
            userChannelId: userChannelId,
            expectedUSD: prepared.newExpectedUSD,
            backingSats: prepared.newBackingSats,
            syncVersion: 1,
            correlation: TradeCorrelation(
                tradeId: tradeId,
                tradePaymentId: paymentId,
                requestHash: prepared.requestHash
            )
        )

        // Missing channel row + closed -> returns invalid (so event can be ACKed and dropped)
        let result = service.channelRepo.applyCorrelatedTradeAcceptance(sync)
        XCTAssertEqual(result.status, TradeControlApplyStatus.invalid)
    }

    // MARK: - applyUncorrelatedSyncIfNewer

    func testUncorrelatedSyncSucceedsWithDriftedUserChannelId() throws {
        let channelId = String(repeating: "ab", count: 32)
        let storedUserChannelId = "user-chan-stored-2"
        let incomingUserChannelId = "user-chan-incoming-drifted-2"

        try service.channelRepo.saveChannel(
            channelId: channelId,
            userChannelId: storedUserChannelId,
            expectedUSD: 10.0,
            backingSats: 10_000,
            nativeSats: 90_000,
            note: nil,
            receiverSats: 100_000,
            latestPrice: 100_000
        )

        let sync = TradeControlMessage.Sync(
            channelId: channelId,
            userChannelId: incomingUserChannelId,
            expectedUSD: 15.0,
            backingSats: 15_000,
            syncVersion: 2,
            correlation: nil
        )

        let result = service.channelRepo.applyUncorrelatedSyncIfNewer(sync, trustedPrice: 100_000)
        XCTAssertEqual(result.status, .applied)
    }

    func testUncorrelatedSyncReturnsInvalidIfChannelMissingAndClosed() throws {
        let channelId = String(repeating: "ab", count: 32)
        let userChannelId = "user-uncorr-closed-1"

        // Mark channel as closed
        _ = try service.paymentRepo.recordPayment(
            paymentId: "close-\(userChannelId)",
            paymentType: "channel_close",
            direction: "received",
            amountMsat: 10_000_000,
            amountUSD: 10.0,
            btcPrice: 100_000,
            counterparty: nil,
            status: "completed",
            txid: String(repeating: "04", count: 32)
        )

        let sync = TradeControlMessage.Sync(
            channelId: channelId,
            userChannelId: userChannelId,
            expectedUSD: 15.0,
            backingSats: 15_000,
            syncVersion: 2,
            correlation: nil
        )

        // Missing channel row + closed -> returns invalid
        let result = service.channelRepo.applyUncorrelatedSyncIfNewer(sync, trustedPrice: 100_000)
        XCTAssertEqual(result.status, .invalid)
    }
}

@MainActor
final class TradeResponseRecoveryTests: XCTestCase {
    private let channelId = String(repeating: "ab", count: 32)
    private let paymentId = String(repeating: "ef", count: 32)
    private let paymentHash = String(repeating: "12", count: 32)
    private var dataDir: URL!
    private var db: DatabaseService!
    private var app: AppState!

    override func setUp() async throws {
        dataDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        db = try DatabaseService(dataDir: dataDir)
        app = makeApp()
    }

    override func tearDown() async throws {
        app = nil
        db = nil
        try FileManager.default.removeItem(at: dataDir)
    }

    private func makeApp() -> AppState {
        let result = AppState(verifyTradeSignature: { _, signature, peer in
            signature == "test-signature" && peer == "original-peer"
        })
        result.databaseService = db
        result.stableChannel.channelId = channelId
        result.stableChannel.userChannelId = "7"
        result.stableChannel.counterparty = "original-peer"
        return result
    }

    private func saveChannel() throws {
        try db.channelRepo.saveChannel(
            channelId: channelId, userChannelId: "7", expectedUSD: 50,
            backingSats: 55_000, nativeSats: 45_000, note: nil,
            receiverSats: 100_000, latestPrice: 100_000
        )
    }

    private func prepareAcceptance() throws -> (PreparedMobileTrade, Data) {
        try saveChannel()
        let trade = try XCTUnwrap(TradeProtocol.prepare(
            channelId: channelId, userChannelId: "7", currentExpectedUSD: 50,
            currentBackingSats: 55_000, receiverSats: 100_000, spendableSats: 100_000,
            action: "sell", amountUSD: 10, amountBTC: 0.000099, feeUSD: 0.1,
            newExpectedUSD: 59.9, quotePrice: 100_000, now: 1_786_310_000,
            tradeId: String(repeating: "cd", count: 32)
        ))
        _ = try db.channelRepo.recordPreparedTrade(trade)
        let payload = try JSONSerialization.data(withJSONObject: [
            "type": "SYNC_V1", "channel_id": channelId, "user_channel_id": "7",
            "expected_usd": trade.newExpectedUSD, "backing_sats": trade.newBackingSats,
            "sync_version": 1, "trade_id": trade.tradeId,
            "trade_payment_id": paymentId, "request_hash": trade.requestHash
        ])
        let envelope = try JSONSerialization.data(withJSONObject: [
            "payload": String(decoding: payload, as: UTF8.self), "signature": "test-signature"
        ])
        return (trade, envelope)
    }

    private func receive(_ data: Data) -> EventAckToken {
        let token = EventAckToken()
        app.handlePaymentReceived(
            paymentId: paymentId, amountMsat: TradeProtocol.resultControlAmountMsat,
            paymentHash: paymentHash,
            customRecords: [CustomTlvRecord(typeNum: Constants.stableChannelTLVType, value: data)],
            ackToken: token
        )
        return token
    }

    func testDeferredAcceptanceSurvivesRestartAndUnblocksNextTrade() throws {
        let (trade, envelope) = try prepareAcceptance()
        try db.rawSQL.execute("DELETE FROM channels")
        XCTAssertTrue(receive(envelope).shouldAck, "A durable response can release the LDK queue immediately")
        XCTAssertTrue(receive(envelope).shouldAck, "Redelivery must not duplicate the inbox entry")
        XCTAssertEqual(try db.channelRepo.deferredTradeResponses().count, 1)
        XCTAssertThrowsError(try db.channelRepo.recordPreparedTrade(trade))
        app = nil
        db = nil
        db = try DatabaseService(dataDir: dataDir)
        app = makeApp()
        // Revalidate with the peer authenticated at arrival, even if settings changed.
        app.stableChannel.counterparty = "changed-peer"
        app.retryDeferredTradeResponses()
        XCTAssertEqual(try db.channelRepo.deferredTradeResponses().count, 1)
        try saveChannel()
        app.retryDeferredTradeResponses()
        XCTAssertTrue(try db.channelRepo.deferredTradeResponses().isEmpty)
        XCTAssertEqual(try db.channelRepo.terminalTradeOutcome(paymentId: paymentId)?.accepted, true)
        XCTAssertEqual(try db.channelRepo.loadChannel(userChannelId: "7")?.expectedUSD, trade.newExpectedUSD)
        let nextTrade = try XCTUnwrap(TradeProtocol.prepare(
            channelId: channelId, userChannelId: "7", currentExpectedUSD: trade.newExpectedUSD,
            currentBackingSats: trade.newBackingSats, receiverSats: 100_000, spendableSats: 100_000,
            action: "sell", amountUSD: 5, amountBTC: 0.0000495, feeUSD: 0.05,
            newExpectedUSD: 64.85, quotePrice: 100_000, now: 1_786_311_000,
            tradeId: String(repeating: "dd", count: 32)
        ))
        XCTAssertNoThrow(try db.channelRepo.recordPreparedTrade(nextTrade))
        // Crash after apply but before inbox deletion: replay must not allocate twice.
        try db.channelRepo.deferTradeResponse(paymentHash: paymentHash, signedRecord: envelope, counterparty: "original-peer")
        app.retryDeferredTradeResponses()
        XCTAssertTrue(try db.channelRepo.deferredTradeResponses().isEmpty)
        XCTAssertEqual(try db.channelRepo.loadChannel(userChannelId: "7")?.backingSats, trade.newBackingSats)
    }

    func testQueueWriteFailureNeverAcknowledgesEvenAfterOldRetryLimit() throws {
        let (_, envelope) = try prepareAcceptance()
        try db.rawSQL.execute("DELETE FROM channels")
        try db.rawSQL.execute("""
            CREATE TRIGGER fail_inbox BEFORE INSERT ON deferred_trade_responses
            BEGIN SELECT RAISE(ABORT, 'injected disk failure'); END
        """)
        for _ in 0..<25 { XCTAssertFalse(receive(envelope).shouldAck) }
        XCTAssertTrue(try db.channelRepo.deferredTradeResponses().isEmpty)
        try db.rawSQL.execute("DROP TRIGGER fail_inbox")
        XCTAssertTrue(receive(envelope).shouldAck)
        XCTAssertEqual(try db.channelRepo.deferredTradeResponses().count, 1)
    }

    func testDatabaseUnavailableKeepsEventUntilInboxCanPersist() throws {
        let (_, envelope) = try prepareAcceptance()
        app.databaseService = nil
        XCTAssertFalse(receive(envelope).shouldAck)
        app.databaseService = db
        XCTAssertTrue(receive(envelope).shouldAck)
        XCTAssertEqual(try db.channelRepo.terminalTradeOutcome(paymentId: paymentId)?.accepted, true)
    }

    func testReceiverBalanceRecoveryAppliesDeferredAcceptance() throws {
        let (trade, envelope) = try prepareAcceptance()
        try db.channelRepo.updateReceiverBalance(channelId: channelId, receiverSats: 20_000)
        XCTAssertTrue(receive(envelope).shouldAck)
        XCTAssertEqual(try db.channelRepo.deferredTradeResponses().count, 1)
        try db.channelRepo.updateReceiverBalance(channelId: channelId, receiverSats: 100_000)
        XCTAssertEqual(try db.channelRepo.loadChannel(userChannelId: "7")?.backingSats, 55_000)
        app.retryDeferredTradeResponses()
        XCTAssertTrue(try db.channelRepo.deferredTradeResponses().isEmpty)
        XCTAssertEqual(try db.channelRepo.loadChannel(userChannelId: "7")?.backingSats, trade.newBackingSats)
    }

    func testInvalidSignatureIsNotSavedForReplay() throws {
        let (_, envelope) = try prepareAcceptance()
        var invalid = try XCTUnwrap(JSONSerialization.jsonObject(with: envelope) as? [String: Any])
        invalid["signature"] = "bad-signature"
        XCTAssertTrue(receive(try JSONSerialization.data(withJSONObject: invalid)).shouldAck)
        XCTAssertTrue(try db.channelRepo.deferredTradeResponses().isEmpty)
        XCTAssertNil(try db.channelRepo.terminalTradeOutcome(paymentId: paymentId))
    }

    func testClosedChannelRetiresDeferredResponse() throws {
        let (_, envelope) = try prepareAcceptance()
        try db.rawSQL.execute("DELETE FROM channels")
        XCTAssertTrue(receive(envelope).shouldAck)
        _ = try db.paymentRepo.recordPayment(
            paymentId: "close-7", paymentType: "channel_close", direction: "received",
            amountMsat: 100_000, amountUSD: nil, btcPrice: nil, counterparty: nil, status: "completed"
        )
        app.retryDeferredTradeResponses()
        XCTAssertTrue(try db.channelRepo.deferredTradeResponses().isEmpty)
    }
}
