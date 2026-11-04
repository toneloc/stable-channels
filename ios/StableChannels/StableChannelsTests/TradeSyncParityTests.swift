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
        try db.channelRepo.deferTradeResponse(
            paymentHash: paymentHash,
            signedRecord: envelope,
            counterparty: "original-peer"
        )
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
        for _ in 0..<25 {
            XCTAssertFalse(receive(envelope).shouldAck)
        }
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

    // MARK: - BuyAmountPolicy Tests

    func testBuyAmountPolicyMaximumUsdTruncatesDown() {
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(10.25), 10.25, accuracy: 0.0001)
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(10.259), 10.25, accuracy: 0.0001)
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(10.299), 10.29, accuracy: 0.0001)
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(0.29), 0.29, accuracy: 0.0001)
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(0.2900000001), 0.29, accuracy: 0.0001)
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(0.0), 0.0, accuracy: 0.0001)
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(-5.0), 0.0, accuracy: 0.0001)
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(.nan), 0.0, accuracy: 0.0001)
        XCTAssertEqual(BuyAmountPolicy.maximumUsd(.infinity), 0.0, accuracy: 0.0001)
    }

    func testBuyAmountPolicyAccepts() {
        XCTAssertTrue(BuyAmountPolicy.accepts(amountUsd: 10.0, balanceUsd: 10.0))
        XCTAssertTrue(BuyAmountPolicy.accepts(amountUsd: 5.50, balanceUsd: 10.0))
        XCTAssertTrue(BuyAmountPolicy.accepts(amountUsd: 0.29, balanceUsd: 0.29))

        XCTAssertFalse(BuyAmountPolicy.accepts(amountUsd: 10.01, balanceUsd: 10.0))
        XCTAssertFalse(BuyAmountPolicy.accepts(amountUsd: 10.30, balanceUsd: 10.299))
        XCTAssertFalse(BuyAmountPolicy.accepts(amountUsd: 0.0, balanceUsd: 10.0))
        XCTAssertFalse(BuyAmountPolicy.accepts(amountUsd: -1.0, balanceUsd: 10.0))
        XCTAssertFalse(BuyAmountPolicy.accepts(amountUsd: .nan, balanceUsd: 10.0))
        XCTAssertFalse(BuyAmountPolicy.accepts(amountUsd: 10.0, balanceUsd: 0.0))
    }
}

// MARK: - Balance correction SYNC (#318) and zero-target SYNC (#322)

extension TradeSyncParityTests {
    private static let correctionChannelId = String(repeating: "ab", count: 32)
    private static let correctionUserChannelId = "7"
    private static let correctionPrice = 100_000.0

    private func saveBooks(expected: Double, backing: UInt64, receiver: UInt64) throws {
        try service.channelRepo.saveChannel(
            channelId: Self.correctionChannelId,
            userChannelId: Self.correctionUserChannelId,
            expectedUSD: expected,
            backingSats: backing,
            note: nil,
            receiverSats: receiver,
            latestPrice: Self.correctionPrice
        )
    }

    private func correctionSync(
        _ expected: Double,
        version: UInt64 = 1,
        backing: UInt64 = 999_999
    ) -> TradeControlMessage.Sync {
        TradeControlMessage.Sync(
            channelId: Self.correctionChannelId,
            userChannelId: "lsp-has-a-different-id",
            expectedUSD: expected,
            backingSats: backing,
            syncVersion: version,
            correlation: nil
        )
    }

    private func assertBooks(
        _ expected: Double,
        _ backing: UInt64,
        _ version: UInt64,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let row = try XCTUnwrap(service.channelRepo.loadChannel(userChannelId: Self.correctionUserChannelId))
        XCTAssertEqual(row.expectedUSD, expected, accuracy: 0.000000001, file: file, line: line)
        XCTAssertEqual(row.backingSats, backing, file: file, line: line)
        XCTAssertEqual(row.syncVersion, version, file: file, line: line)
    }

    func testSyncBackingAfterDeltaClampsToCapacityInsteadOfFailing() {
        XCTAssertNil(TradeProtocol.tradeBackingAfterDelta(
            receiverSats: 58_000, currentBackingSats: 55_000,
            currentExpectedUSD: 50, newExpectedUSD: 60, price: Self.correctionPrice
        ))
        XCTAssertEqual(TradeProtocol.syncBackingAfterDelta(
            receiverSats: 58_000, currentBackingSats: 55_000,
            currentExpectedUSD: 50, newExpectedUSD: 60, price: Self.correctionPrice
        ), 58_000)
        // Negative delta clamps into [0, receiver]; a zero result is nil.
        XCTAssertEqual(TradeProtocol.syncBackingAfterDelta(
            receiverSats: 30_000, currentBackingSats: 55_000,
            currentExpectedUSD: 50, newExpectedUSD: 40, price: Self.correctionPrice
        ), 30_000)
        XCTAssertNil(TradeProtocol.syncBackingAfterDelta(
            receiverSats: 100_000, currentBackingSats: 1_000,
            currentExpectedUSD: 50, newExpectedUSD: 40, price: Self.correctionPrice
        ))
        // Zero targets are the caller's job; invalid inputs never derive a backing.
        XCTAssertNil(TradeProtocol.syncBackingAfterDelta(
            receiverSats: 100_000, currentBackingSats: 1_000,
            currentExpectedUSD: 50, newExpectedUSD: 0, price: Self.correctionPrice
        ))
        for price in [0.0, -1.0, Double.nan, Double.infinity] {
            XCTAssertNil(TradeProtocol.syncBackingAfterDelta(
                receiverSats: 100_000, currentBackingSats: 1_000,
                currentExpectedUSD: 50, newExpectedUSD: 60, price: price
            ))
        }
    }

    func testUncorrelatedSyncBackingAtZeroTargetKeepsOnlyWhatTheLspBooks() {
        let backing: (UInt64, UInt64, UInt64) -> UInt64? = { current, signed, receiver in
            TradeProtocol.uncorrelatedSyncBacking(
                receiverSats: receiver,
                currentBackingSats: current,
                currentExpectedUSD: 60,
                sync: self.correctionSync(0, backing: signed),
                price: Self.correctionPrice
            )
        }
        XCTAssertEqual(backing(20_000, 20_000, 15_000), 15_000)
        XCTAssertEqual(backing(20_000, 5_000, 15_000), 5_000)
        XCTAssertEqual(backing(60_000, 0, 100_000), 0)
        XCTAssertEqual(backing(0, 5_000, 100_000), 0)
    }

    func testCorrectionClampsAnOverCapacityAllocationAndSurvivesReplay() throws {
        try saveBooks(expected: 50, backing: 55_000, receiver: 58_000)
        let result = service.channelRepo.applyUncorrelatedSyncIfNewer(
            correctionSync(60),
            trustedPrice: Self.correctionPrice
        )
        XCTAssertEqual(result.status, .applied)
        XCTAssertEqual(result.localBackingSats, 58_000)
        XCTAssertEqual(result.peerBackingSats, 999_999)
        try assertBooks(60, 58_000, 1)

        service = try DatabaseService(dataDir: dataDir)
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(correctionSync(60), trustedPrice: Self.correctionPrice)
                .status,
            .duplicate
        )
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(
                correctionSync(70, version: 1),
                trustedPrice: Self.correctionPrice
            ).status,
            .duplicate
        )
        try assertBooks(60, 58_000, 1)
    }

    func testCorrectionRestoresATargetJustAboveTheWalletsCapacity() throws {
        try saveBooks(expected: 0, backing: 0, receiver: 100_000)
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(
                correctionSync(100.001),
                trustedPrice: Self.correctionPrice
            )
            .status,
            .applied
        )
        try assertBooks(100.001, 100_000, 1)
    }

    func testCorrectionPreservesLocalDriftAndUsesCumulativeFloorsForTheDelta() throws {
        try saveBooks(expected: 70, backing: 65_000, receiver: 100_000)
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(correctionSync(60), trustedPrice: Self.correctionPrice)
                .status,
            .applied
        )
        // At this floating-point price, floor(60 / price * satsPerBtc) is 59,999.
        try assertBooks(60, 54_999, 1)
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(correctionSync(60, version: 2), trustedPrice: 90_000)
                .status,
            .applied
        )
        try assertBooks(60, 54_999, 2)
    }

    func testCorrectionStillRetriesWhenNoPositiveBackingCanBeDerived() throws {
        try saveBooks(expected: 50, backing: 1_000, receiver: 100_000)
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(correctionSync(40), trustedPrice: Self.correctionPrice)
                .status,
            .retry
        )
        try assertBooks(50, 1_000, 0)
        try saveBooks(expected: 0, backing: 0, receiver: 0)
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(correctionSync(40), trustedPrice: Self.correctionPrice)
                .status,
            .retry
        )
        try assertBooks(0, 0, 0)
    }

    func testCorrectionWithUnusablePricesNeverAdvancesTheVersion() throws {
        try saveBooks(expected: 50, backing: 55_000, receiver: 58_000)
        for badPrice in [0.0, -1.0, Double.nan, Double.infinity] {
            XCTAssertEqual(
                service.channelRepo.applyUncorrelatedSyncIfNewer(correctionSync(60), trustedPrice: badPrice).status,
                .invalid
            )
            try assertBooks(50, 55_000, 0)
        }
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(
                correctionSync(60),
                trustedPrice: Double.leastNonzeroMagnitude
            ).status,
            .retry
        )
        try assertBooks(50, 55_000, 0)
        // Adding the derived delta would overflow before it is clamped.
        let max = UInt64(Int64.max)
        try saveBooks(expected: 0, backing: max - 2, receiver: max - 1)
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(correctionSync(10), trustedPrice: Self.correctionPrice)
                .status,
            .applied
        )
        try assertBooks(10, max - 1, 1)
    }

    func testUncorrelatedSyncAtZeroTargetPreservesTheResidueTheLspStillBooks() throws {
        try saveBooks(expected: 10, backing: 20_000, receiver: 15_000)
        let result = service.channelRepo.applyUncorrelatedSyncIfNewer(
            correctionSync(0, backing: 20_000),
            trustedPrice: Self.correctionPrice
        )
        XCTAssertEqual(result.status, .applied)
        XCTAssertEqual(result.localBackingSats, 15_000)
        try assertBooks(0, 15_000, 1)
    }

    func testUncorrelatedSyncAtZeroTargetReleasesWhatTheLspBookedAsClosed() throws {
        try saveBooks(expected: 60, backing: 60_000, receiver: 100_000)
        let result = service.channelRepo.applyUncorrelatedSyncIfNewer(
            correctionSync(0, backing: 0),
            trustedPrice: Self.correctionPrice
        )
        XCTAssertEqual(result.status, .applied)
        XCTAssertEqual(result.localBackingSats, 0)
        try assertBooks(0, 0, 1)
    }

    // MARK: - Splice reconcile against the stored row (#318)

    func testOutgoingReconcileUsesCommittedCorrectionNotAnOlderSnapshot() throws {
        try saveBooks(expected: 10, backing: 10_000, receiver: 9_000)
        XCTAssertEqual(
            service.channelRepo.applyUncorrelatedSyncIfNewer(correctionSync(8), trustedPrice: Self.correctionPrice)
                .status,
            .applied
        )
        // Memory may still hold 10.0 / 10_000; the row already fits the confirmed balance.
        XCTAssertNil(try service.channelRepo.reconcileOutgoingBacking(
            userChannelId: Self.correctionUserChannelId,
            receiverSats: 9_000,
            price: Self.correctionPrice
        ).repair)
        try assertBooks(8, 8_000, 1)
    }

    func testOutgoingReconcileDeductsOverflowOnceAndRetriesAreIdempotent() throws {
        try saveBooks(expected: 10, backing: 10_000, receiver: 10_000)
        let books = try service.channelRepo.reconcileOutgoingBacking(
            userChannelId: Self.correctionUserChannelId,
            receiverSats: 9_000,
            price: Self.correctionPrice
        )
        XCTAssertEqual(books.expectedUSD, 9, accuracy: 0.000001)
        XCTAssertEqual(books.backingSats, 9_000)
        let repair = try XCTUnwrap(books.repair)
        XCTAssertEqual(repair.usdDeducted, 1, accuracy: 0.000001)
        try assertBooks(9, 9_000, 0)
        XCTAssertNil(try service.channelRepo.reconcileOutgoingBacking(
            userChannelId: Self.correctionUserChannelId,
            receiverSats: 9_000,
            price: Self.correctionPrice
        ).repair)
        try assertBooks(9, 9_000, 0)
    }

    func testOutgoingReconcileWithMissingRowThrowsDedicatedError() {
        XCTAssertThrowsError(try service.channelRepo.reconcileOutgoingBacking(
            userChannelId: "missing",
            receiverSats: 9_000,
            price: Self.correctionPrice
        )) { error in
            guard case DatabaseError.missingChannelRow("missing") = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
    }
}
