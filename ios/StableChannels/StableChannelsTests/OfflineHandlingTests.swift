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

    func testEffectiveTradePriceFallback() {
        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        // When accounting price is 0, effectiveTradePrice falls back to btcPrice
        XCTAssertEqual(appState.accountingBTCPrice, 0.0)
        XCTAssertGreaterThan(appState.btcPrice, 0.0)
        XCTAssertEqual(appState.effectiveTradePrice, appState.btcPrice)

        #if DEBUG
            // When accounting price is trusted and positive, effectiveTradePrice uses accounting price
            appState.priceService.setPriceForTesting(68_500)
            XCTAssertEqual(appState.accountingBTCPrice, 68_500)
            XCTAssertEqual(appState.effectiveTradePrice, 68_500)
        #endif
    }

    func testOfflineCachedNodeIdAvailableWhenNodeNotRunning() {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        let expectedNodeId = "02710b5069e90a44deadbeef"
        ud?.set(expectedNodeId, forKey: "node_id")

        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertFalse(appState.nodeService.isRunning)
        let cachedNodeId = UserDefaults(suiteName: suite)?.string(forKey: "node_id")
        XCTAssertEqual(cachedNodeId, expectedNodeId)
    }

    func testHasActiveChannelWhenNodeOfflineWithCachedChannelReady() {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(true, forKey: "cached_has_ready_channel")
        ud?.set(Int64(0), forKey: "cached_lightning_sats")
        ud?.removeObject(forKey: "funding_txid")

        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertFalse(appState.nodeService.isRunning)
        XCTAssertTrue(appState.nodeService.channels.isEmpty)
        XCTAssertTrue(
            appState.hasActiveChannel,
            "hasActiveChannel must remain true when channel is cached ready even if node is offline"
        )
    }

    func testHasActiveChannelWhenNodeOfflineWithCachedLightningBalance() {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(false, forKey: "cached_has_ready_channel")
        ud?.set(Int64(25_000), forKey: "cached_lightning_sats")
        ud?.removeObject(forKey: "funding_txid")

        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertFalse(appState.nodeService.isRunning)
        XCTAssertTrue(appState.nodeService.channels.isEmpty)
        XCTAssertTrue(appState.hasActiveChannel, "hasActiveChannel must remain true when lightning balance is cached")
    }

    func testHasActiveChannelWhenNodeOfflineWithFundingTxid() {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(false, forKey: "cached_has_ready_channel")
        ud?.set(Int64(0), forKey: "cached_lightning_sats")
        ud?.set("11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff", forKey: "funding_txid")

        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertFalse(appState.nodeService.isRunning)
        XCTAssertTrue(appState.nodeService.channels.isEmpty)
        XCTAssertTrue(appState.hasActiveChannel, "hasActiveChannel must remain true when fundingTxid is cached")
    }

    func testHasActiveChannelFalseWhenCleanWithoutChannel() {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(false, forKey: "cached_has_ready_channel")
        ud?.set(Int64(0), forKey: "cached_lightning_sats")
        ud?.removeObject(forKey: "funding_txid")

        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)

        XCTAssertFalse(appState.nodeService.isRunning)
        XCTAssertTrue(appState.nodeService.channels.isEmpty)
        XCTAssertFalse(
            appState.hasActiveChannel,
            "hasActiveChannel must be false when no channels or cached metrics exist"
        )
    }

    func testSwitchLSPRejectedWhenActiveChannelExists() async {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(true, forKey: "cached_has_ready_channel")

        let mockMonitor = MockNetworkMonitor(initialStatus: .online)
        let appState = AppState(networkMonitor: mockMonitor)
        XCTAssertTrue(appState.hasActiveChannel)

        let initialLSP = appState.activeLSP
        let customLSP = LSPConfig(
            alias: "Custom Test LSP",
            pubkey: "02" + String(repeating: "1", count: 64),
            address: "127.0.0.1:9735",
            token: nil
        )

        let result = await appState.switchLSP(to: customLSP)
        XCTAssertFalse(result, "switchLSP must be rejected when active channel exists")
        XCTAssertEqual(appState.activeLSP, initialLSP, "Active LSP must remain unchanged after rejected switch")
    }

    func testSwitchLSPRejectedWhenOffline() async {
        let suite = Constants.appGroupIdentifier
        let ud = UserDefaults(suiteName: suite)
        ud?.set(false, forKey: "cached_has_ready_channel")
        ud?.set(Int64(0), forKey: "cached_lightning_sats")
        ud?.removeObject(forKey: "funding_txid")

        let mockMonitor = MockNetworkMonitor(initialStatus: .offline)
        let appState = AppState(networkMonitor: mockMonitor)
        XCTAssertFalse(appState.hasActiveChannel)
        XCTAssertFalse(appState.isOnline)

        let initialLSP = appState.activeLSP
        let customLSP = LSPConfig(
            alias: "Custom Test LSP",
            pubkey: "02" + String(repeating: "2", count: 64),
            address: "127.0.0.1:9735",
            token: nil
        )

        let result = await appState.switchLSP(to: customLSP)
        XCTAssertFalse(result, "switchLSP must be rejected when offline")
        XCTAssertEqual(appState.activeLSP, initialLSP, "Active LSP must remain unchanged after rejected switch")
    }
}
