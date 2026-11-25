@testable import StableChannels
import XCTest

final class StabilitySurfaceTests: XCTestCase {
    func testWithoutStabilityHomeShowsNoSliderOrTradeButtons() {
        let surface = StabilitySurface(stabilityEnabled: false)

        XCTAssertFalse(surface.showsBalanceBar)
        XCTAssertFalse(surface.showsTradeButtons)
        XCTAssertFalse(surface.showsStablePosition)
    }

    func testWithStabilityEverythingShows() {
        let surface = StabilitySurface(stabilityEnabled: true)

        XCTAssertTrue(surface.showsBalanceBar)
        XCTAssertTrue(surface.showsTradeButtons)
        XCTAssertTrue(surface.showsStablePosition)
    }

    func testTheNormalBuildShowsEveryStabilitySurface() {
        XCTAssertEqual(StabilitySurface.current, StabilitySurface(stabilityEnabled: true))
    }

    func testTradeActionsExistOnlyWithStability() {
        var calls = 0
        let action = StabilitySurface(stabilityEnabled: true).tradeAction { calls += 1 }
        XCTAssertNotNil(action)
        action?()
        XCTAssertEqual(calls, 1)
        XCTAssertNil(StabilitySurface(stabilityEnabled: false).tradeAction { calls += 1 })
    }

    func testWithStabilityThePushCopyIsUnchanged() {
        let surface = StabilitySurface(stabilityEnabled: true)
        XCTAssertEqual(surface.pushTitle, "Background payments and stability updates")
        XCTAssertEqual(
            surface.pushDetail,
            "Your device maintains a persistent connection to receive incoming payments and stability updates."
        )
    }

    func testWithoutStabilityThePushCopyNeverMentionsIt() {
        let surface = StabilitySurface(stabilityEnabled: false)
        XCTAssertEqual(surface.pushTitle, "Background payments")
        XCTAssertEqual(
            surface.pushDetail,
            "Your device maintains a persistent connection to receive incoming payments."
        )
        for text in [surface.pushTitle, surface.pushDetail] {
            XCTAssertFalse(text.localizedCaseInsensitiveContains("stabil"), "Unexpected stability wording: \(text)")
        }
    }

    func testTheNotificationPromptsAreNeutralInEveryBuild() {
        // Both keys already read the same, with no stability wording, in all 11 catalog languages.
        let banner = String(localized: "notifications_disabled_subtitle")
        let required = String(localized: "info_notifications_needed")
        XCTAssertEqual(banner, "Enable notifications to receive payment alerts")
        XCTAssertEqual(required, "Enable notifications to receive payment alerts")
    }

    func testWithoutStabilityHistoryHasNoOrders() {
        XCTAssertFalse(StabilitySurface(stabilityEnabled: false).showsOrders)
        XCTAssertTrue(StabilitySurface(stabilityEnabled: true).showsOrders)
    }

    func testHistoryOpensOnPaymentsWithoutStability() {
        XCTAssertEqual(StabilitySurface(stabilityEnabled: false).initialHistorySegment, .payments)
        XCTAssertEqual(StabilitySurface(stabilityEnabled: true).initialHistorySegment, .orders)
    }
}
