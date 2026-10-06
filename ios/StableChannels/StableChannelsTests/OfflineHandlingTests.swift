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
}
