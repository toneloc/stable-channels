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
}
