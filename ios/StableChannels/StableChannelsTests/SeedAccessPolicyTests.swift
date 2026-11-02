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
}
