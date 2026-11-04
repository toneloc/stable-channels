@testable import StableChannels
import XCTest

final class SeedAccessPolicyTests: XCTestCase {
    func testNoAuthWhenBothTogglesAreOff() {
        XCTAssertFalse(SeedAccessPolicy.requiresAuth(appUnlockEnabled: false, paymentConfirmationEnabled: false))
    }

    func testAuthWhenEitherToggleIsOn() {
        XCTAssertTrue(SeedAccessPolicy.requiresAuth(appUnlockEnabled: true, paymentConfirmationEnabled: false))
        XCTAssertTrue(SeedAccessPolicy.requiresAuth(appUnlockEnabled: false, paymentConfirmationEnabled: true))
        XCTAssertTrue(SeedAccessPolicy.requiresAuth(appUnlockEnabled: true, paymentConfirmationEnabled: true))
    }

    func testOnlyBackgroundClosesSeedScreens() {
        XCTAssertTrue(SeedAccessPolicy.shouldCloseSeedScreens(for: .background))
        XCTAssertFalse(SeedAccessPolicy.shouldCloseSeedScreens(for: .inactive))
        XCTAssertFalse(SeedAccessPolicy.shouldCloseSeedScreens(for: .active))
    }

    func testLateAuthReplyOpensSeedScreenOnlyOutsideBackground() {
        XCTAssertTrue(SeedAccessPolicy.canOpenSeedScreen(after: .active))
        XCTAssertTrue(SeedAccessPolicy.canOpenSeedScreen(after: .inactive))
        XCTAssertFalse(SeedAccessPolicy.canOpenSeedScreen(after: .background))
    }
}
