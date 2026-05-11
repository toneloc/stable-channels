@testable import StableChannels
import XCTest

final class PendingPushPaymentTests: XCTestCase {
    private let key = "pending_push_payment"

    private func makeDefaults(_ suffix: String) -> UserDefaults {
        let suite = "PendingPushPaymentTests.\(suffix).\(UUID().uuidString)"
        let ud = UserDefaults(suiteName: suite)!
        ud.removePersistentDomain(forName: suite)
        return ud
    }

    func testLegacyBehaviorDropsPendingFlagOnReconnectFailure() {
        let ud = makeDefaults("legacy")
        ud.set(true, forKey: key)

        // Legacy flow in AppState before fix:
        // shared?.set(false, forKey: "pending_push_payment") before reconnect
        ud.set(false, forKey: key)

        XCTAssertFalse(
            ud.bool(forKey: key),
            "Legacy behavior drops pending flag before reconnect outcome is known"
        )
    }

    func testKeepsPendingFlagWhenReconnectFails() {
        let ud = makeDefaults("failure")
        ud.set(true, forKey: key)

        AppState.updatePendingPushPaymentFlag(ud, reconnectSucceeded: false)

        XCTAssertTrue(
            ud.bool(forKey: key),
            "Pending flag must remain true so foreground/startup can retry"
        )
    }

    func testClearsPendingFlagWhenReconnectSucceeds() {
        let ud = makeDefaults("success")
        ud.set(true, forKey: key)

        AppState.updatePendingPushPaymentFlag(ud, reconnectSucceeded: true)

        XCTAssertFalse(
            ud.bool(forKey: key),
            "Pending flag should clear only after successful reconnect"
        )
    }
}
