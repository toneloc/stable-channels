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

    // Locked means App Access is on and this session has not passed it yet; without App Access nothing locks.
    @MainActor
    func testAppIsLockedOnlyWhenAppAccessIsOnAndNotYetPassed() async throws {
        let suiteName = "SeedAccessPolicyTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let appState = AppState(seedAccessDefaults: defaults)
        appState.biometricAuth = AllowingAuth()

        XCTAssertFalse(appState.isAppLocked)
        defaults.set(true, forKey: Constants.appUnlockEnabledKey)
        XCTAssertTrue(appState.isAppLocked)
        _ = await appState.authenticate()
        XCTAssertFalse(appState.isAppLocked)
    }
}

/// Stands in for biometrics and always lets the user in.
@MainActor
private final class AllowingAuth: BiometricAuthenticating {
    func authenticate(reason _: String, allowPasscodeFallback _: Bool) async throws -> Bool {
        true
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
