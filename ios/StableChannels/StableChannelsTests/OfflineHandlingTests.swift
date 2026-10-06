@testable import StableChannels
import XCTest

@MainActor
final class OfflineHandlingTests: XCTestCase {
    func testAppStateOnlineReflectsNetworkMonitor() {
        let mockMonitor = MockNetworkMonitor(initialStatus: .online)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertTrue(appState.isOnline)
        XCTAssertFalse(appState.showOfflineNotice)

        mockMonitor.setStatus(.offline)
        XCTAssertFalse(appState.isOnline)
    }

    func testOfflineNoticeCanBePresentedAndDismissed() {
        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertFalse(appState.showOfflineNotice)

        appState.showOfflineNotice = true
        XCTAssertTrue(appState.showOfflineNotice)

        appState.showOfflineNotice = false
        XCTAssertFalse(appState.showOfflineNotice)
    }

    func testRetryConnectionWhenStillOfflineMaintainsNotice() async {
        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)
        appState.showOfflineNotice = true

        await appState.retryConnection()

        XCTAssertTrue(appState.showOfflineNotice, "Offline notice should stay presented if still offline")
        XCTAssertFalse(appState.isRetryingConnection, "Retrying flag should be reset after attempt")
    }

    func testRetryConnectionWhenOnlineDismissesNotice() async {
        let mockMonitor = MockNetworkMonitor(initialStatus: .online)
        let appState = AppState(networkMonitor: mockMonitor)
        XCTAssertFalse(appState.hasCompletedInitialSync)
        appState.showOfflineNotice = true

        await appState.retryConnection()

        XCTAssertFalse(appState.showOfflineNotice, "Offline notice should be dismissed when connection succeeds")
        XCTAssertFalse(appState.isRetryingConnection)
        XCTAssertTrue(appState.hasCompletedInitialSync)
        XCTAssertEqual(appState.phase, .wallet)
    }

    func testNetworkMonitorStartsOnAppStartupInvocation() async {
        let mockMonitor = MockNetworkMonitor(initialStatus: .online)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertFalse(mockMonitor.didStart)

        // Starting app starts the network monitor
        Task {
            await appState.start()
        }

        // Give a short window for prologue execution
        try? await Task.sleep(nanoseconds: 50_000_000)

        XCTAssertTrue(mockMonitor.didStart, "Network monitor start() must be invoked on AppState.start()")
    }

    func testNetworkStatusChangeAutomaticallyReplacesOfflinePhaseWhenOnline() {
        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let expectation = expectation(description: "Status change handler called")

        mockMonitor.onStatusChange = { status in
            if status == .online {
                expectation.fulfill()
            }
        }

        mockMonitor.setStatus(.online)
        wait(for: [expectation], timeout: 1.0)
    }

    func testStartupWhenOfflineTransitionsToOfflinePhase() async {
        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertEqual(appState.phase, .loading)
        await appState.start()
        XCTAssertEqual(appState.phase, .offline)
        XCTAssertTrue(appState.isOfflineBlocked)
    }

    func testCachedBalancesAndPricePreservedWhenOffline() {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(Int64(10_000), forKey: "cached_lightning_sats")
        ud?.set(Int64(5_000), forKey: "cached_onchain_sats")
        ud?.set(true, forKey: "cached_has_ready_channel")
        PriceOracleAnchorStore.save(price: 90_000, suiteName: suite)

        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertEqual(appState.lightningBalanceSats, 10_000)
        XCTAssertEqual(appState.onchainBalanceSats, 5_000)
        XCTAssertTrue(appState.hasReadyChannel)
        XCTAssertEqual(appState.totalBalanceSats, 15_000)
        XCTAssertGreaterThan(appState.btcPrice, 0)
        XCTAssertGreaterThan(appState.totalBalanceUSD, 0)
    }

    func testBalanceBarAllocationRemainsValidWhenOffline() {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(Int64(20_000), forKey: "cached_lightning_sats")
        ud?.set(Int64(0), forKey: "cached_onchain_sats")
        ud?.set(true, forKey: "cached_has_ready_channel")
        PriceOracleAnchorStore.save(price: 80_000, suiteName: suite)

        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        let allocation = ChannelAllocation(
            stableUSD: appState.stableUSD,
            lightningBalanceSats: appState.lightningBalanceSats,
            btcPrice: appState.btcPrice,
            backingSatsOverride: appState.stableChannel.backingSats
        )

        XCTAssertFalse(allocation.isEmpty)
        XCTAssertGreaterThan(allocation.btcPrice, 0)
        XCTAssertEqual(allocation.stableFraction, 0.0, accuracy: 0.01)
    }

    func testTradeServiceMaxSellCentsUsesCachedChannelWhenNodeNotRunning() throws {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(Int64(50_000), forKey: "cached_lightning_sats")
        ud?.set(true, forKey: "cached_has_ready_channel")

        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let db = try DatabaseService(dataDir: tempDir)
        let tradeService = TradeService(nodeService: NodeService(), databaseService: db)
        let sc = StableChannel.default

        let maxSell = tradeService.maxSellCents(sc: sc, price: 60_000)
        XCTAssertGreaterThan(maxSell, 0, "maxSellCents should compute positive limit from cached channel capacity")
    }

    func testNetworkStatusChangeRetriesConnectionWhileInWalletPhase() async {
        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)
        appState.phase = .wallet

        let expectation = expectation(description: "Status change handler triggers retry")
        mockMonitor.onStatusChange = { status in
            if status == .online {
                expectation.fulfill()
            }
        }

        mockMonitor.setStatus(.online)
        await fulfillment(of: [expectation], timeout: 1.0)
    }
}
