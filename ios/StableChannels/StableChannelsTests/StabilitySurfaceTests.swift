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

    func testCurrentFollowsTheBuild() {
        XCTAssertEqual(
            StabilitySurface.current,
            StabilitySurface(stabilityEnabled: BuildFeatures.stability)
        )
    }

    func testWithStabilityTheCopyIsUnchanged() {
        let surface = StabilitySurface(stabilityEnabled: true)

        // Existing `en` entries in Localizable.xcstrings override the Swift defaultValue for these two keys.
        XCTAssertEqual(surface.notificationBanner, "Enable notifications to receive payment alerts")
        XCTAssertEqual(surface.notificationsRequired, "Enable notifications to receive payment alerts")
        XCTAssertEqual(surface.pushTitle, "Background payments and stability updates")
        XCTAssertEqual(
            surface.pushDetail,
            "Your device maintains a persistent connection to receive incoming payments and stability updates."
        )
    }

    func testWithoutStabilityNoCopyMentionsIt() {
        let surface = StabilitySurface(stabilityEnabled: false)
        let strings = [
            surface.notificationBanner,
            surface.notificationsRequired,
            surface.pushTitle,
            surface.pushDetail
        ]

        XCTAssertEqual(
            surface.notificationBanner,
            "Enable notifications to receive payments while the app is closed"
        )
        XCTAssertEqual(
            surface.notificationsRequired,
            "Notifications are required to receive payments while the app is closed."
        )
        XCTAssertEqual(surface.pushTitle, "Background payments")
        XCTAssertEqual(
            surface.pushDetail,
            "Your device maintains a persistent connection to receive incoming payments."
        )
        for text in strings {
            XCTAssertFalse(text.localizedCaseInsensitiveContains("stabil"), "Unexpected 'stabil' in: \(text)")
        }
    }
}
