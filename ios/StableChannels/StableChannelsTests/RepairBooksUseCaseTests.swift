import XCTest
@testable import StableChannels

private final class CountingStatusProvider: PaymentStatusProviding, SpliceStatusProviding,
    StabilitySendStatusProviding {
    var paymentCalls = 0
    var spliceCalls = 0
    var stabilitySendCalls = 0

    var pendingPayment = false
    var pendingSplice = false
    var pendingStabilitySend = false

    func hasPendingOutgoingPayments() -> Bool { paymentCalls += 1; return pendingPayment }
    func hasPendingSplice() -> Bool { spliceCalls += 1; return pendingSplice }
    func hasPendingStabilitySend() -> Bool { stabilitySendCalls += 1; return pendingStabilitySend }
}

final class RepairBooksUseCaseTests: XCTestCase {
    func testExecuteHealsOverbackedChannelWhenPolicyPermits() {
        let provider = CountingStatusProvider()
        let useCase = RepairBooksUseCase(
            paymentStatusProvider: provider,
            spliceStatusProvider: provider,
            stabilitySendStatusProvider: provider
        )
        var channel = StableChannel.default
        channel.isStableReceiver = true
        channel.userChannelId = "test-channel"
        channel.expectedUSD = USD(amount: 100.0)
        channel.backingSats = 100_000
        channel.stableReceiverBTC = Bitcoin(sats: 60_000)

        let context = RepairBooksUseCase.Context(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )

        let result = useCase.execute(channel: &channel, context: context)

        XCTAssertNotNil(result)
        XCTAssertEqual(result?.overflowSats, 40_000)
        XCTAssertEqual(result?.usdDeducted, 40.0)
        XCTAssertEqual(channel.expectedUSD.amount, 60.0)
        XCTAssertEqual(channel.backingSats, 60_000)
        XCTAssertEqual(provider.spliceCalls, 1)
        XCTAssertEqual(provider.stabilitySendCalls, 1)
        XCTAssertEqual(provider.paymentCalls, 1)
    }

    func testExecuteShortCircuitsWhenChannelNotOverbacked() {
        let provider = CountingStatusProvider()
        let useCase = RepairBooksUseCase(
            paymentStatusProvider: provider,
            spliceStatusProvider: provider,
            stabilitySendStatusProvider: provider
        )

        var channel = StableChannel.default
        channel.isStableReceiver = true
        channel.userChannelId = "test-channel"
        channel.expectedUSD = USD(amount: 50.0)
        channel.backingSats = 50_000
        channel.stableReceiverBTC = Bitcoin(sats: 60_000)

        let context = RepairBooksUseCase.Context(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )

        let result = useCase.execute(channel: &channel, context: context)

        XCTAssertNil(result)
        // Zero provider queries executed when channel is not overbacked
        XCTAssertEqual(provider.spliceCalls, 0)
        XCTAssertEqual(provider.stabilitySendCalls, 0)
        XCTAssertEqual(provider.paymentCalls, 0)
    }

    func testExecuteShortCircuitsWhenChannelNotReady() {
        let provider = CountingStatusProvider()
        let useCase = RepairBooksUseCase(
            paymentStatusProvider: provider,
            spliceStatusProvider: provider,
            stabilitySendStatusProvider: provider
        )

        var channel = StableChannel.default
        channel.isStableReceiver = true
        channel.userChannelId = "test-channel"
        channel.backingSats = 100_000
        channel.stableReceiverBTC = Bitcoin(sats: 60_000)

        let context = RepairBooksUseCase.Context(
            hasUserChannelId: true,
            hasReadyChannel: false,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )

        let result = useCase.execute(channel: &channel, context: context)

        XCTAssertNil(result)
        XCTAssertEqual(provider.spliceCalls, 0)
        XCTAssertEqual(provider.stabilitySendCalls, 0)
        XCTAssertEqual(provider.paymentCalls, 0)
    }

    func testExecuteShortCircuitsOnFirstFailingProvider() {
        let provider = CountingStatusProvider()
        provider.pendingSplice = true

        let useCase = RepairBooksUseCase(
            paymentStatusProvider: provider,
            spliceStatusProvider: provider,
            stabilitySendStatusProvider: provider
        )

        var channel = StableChannel.default
        channel.isStableReceiver = true
        channel.userChannelId = "test-channel"
        channel.backingSats = 100_000
        channel.stableReceiverBTC = Bitcoin(sats: 60_000)

        let context = RepairBooksUseCase.Context(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )

        let initialBacking = channel.backingSats
        let initialUSD = channel.expectedUSD.amount
        let result = useCase.execute(channel: &channel, context: context)

        XCTAssertNil(result)
        XCTAssertEqual(channel.backingSats, initialBacking)
        XCTAssertEqual(channel.expectedUSD.amount, initialUSD)
        XCTAssertEqual(provider.spliceCalls, 1)
        // Subsequent checks skipped
        XCTAssertEqual(provider.stabilitySendCalls, 0)
        XCTAssertEqual(provider.paymentCalls, 0)
    }

    @MainActor
    func testAppStateUsesInjectedCustomRepairBooksUseCase() throws {
        let provider = CountingStatusProvider()
        let mockUseCase = RepairBooksUseCase(
            paymentStatusProvider: provider,
            spliceStatusProvider: provider,
            stabilitySendStatusProvider: provider
        )

        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let appState = AppState(repairBooksUseCase: mockUseCase)
        appState.databaseService = try DatabaseService(dataDir: tempDir)
        appState.priceService.setPriceForTesting(100_000.0)
        appState.stableChannel.isStableReceiver = true
        appState.stableChannel.userChannelId = "chan-1"
        appState.hasReadyChannel = true
        appState.stableChannel.expectedUSD = USD(amount: 100.0)
        appState.stableChannel.backingSats = 100_000
        appState.stableChannel.stableReceiverBTC = Bitcoin(sats: 60_000)

        appState.repairBooksAboveLiveBalance()

        XCTAssertEqual(appState.stableChannel.expectedUSD.amount, 60.0)
        XCTAssertEqual(appState.stableChannel.backingSats, 60_000)
        XCTAssertEqual(provider.paymentCalls, 1)
    }
}
