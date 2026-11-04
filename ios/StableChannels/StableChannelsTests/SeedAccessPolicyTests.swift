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

    @MainActor
    func testAppStatePromptsForSeedAccessOnlyWhenAToggleIsOn() async throws {
        let suiteName = "SeedAccessPolicyTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let probe = SeedAuthProbe()
        let appState = AppState(seedAccessDefaults: defaults)
        appState.biometricAuth = probe

        let allowedWithBothOff = await appState.authorizeSeedAccess(reason: "test")
        XCTAssertTrue(allowedWithBothOff)
        XCTAssertEqual(probe.calls, 0)

        for key in [Constants.appUnlockEnabledKey, Constants.transactionAuthEnabledKey] {
            defaults.removePersistentDomain(forName: suiteName)
            defaults.set(true, forKey: key)
            let allowed = await appState.authorizeSeedAccess(reason: "test")
            XCTAssertFalse(allowed, "the probe denies, so a prompt must decide")
        }
        XCTAssertEqual(probe.calls, 2)
    }
}

/// Stands in for biometrics: counts prompts and always denies.
@MainActor
private final class SeedAuthProbe: BiometricAuthenticating {
    private(set) var calls = 0

    func authenticate(reason _: String, allowPasscodeFallback _: Bool) async throws -> Bool {
        calls += 1
        return false
    }
}
