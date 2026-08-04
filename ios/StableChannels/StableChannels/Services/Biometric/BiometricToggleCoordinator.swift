import Foundation

@MainActor @Observable
final class BiometricToggleCoordinator {
    // MARK: - Dependencies

    private let auth: BiometricAuthenticating

    // MARK: - Observable State

    private(set) var isEnabling = false
    var requiresSettingsRedirect = false

    // MARK: - Init

    init(auth: BiometricAuthenticating = BiometricService.shared) {
        self.auth = auth
    }

    // MARK: - Enable Flow

    /// Authenticates with biometrics before enabling a toggle.
    /// Redirects user to iOS system settings if Face ID permission is revoked.
    func enableToggle(_ key: String, reason: String) async -> Bool {
        guard !isEnabling else { return false }
        isEnabling = true
        defer { isEnabling = false }

        do {
            let success = try await auth.authenticateWithBiometrics(reason: reason)
            if success {
                UserDefaults.standard.set(true, forKey: key)
            }
            return success
        } catch let error as BiometricError where error == .notAvailable {
            requiresSettingsRedirect = true
            return false
        } catch {
            print("[Bio] Enable auth failed: \(error.localizedDescription)")
            return false
        }
    }

    // MARK: - Disable Flow

    /// Authenticates before disabling a toggle, falling back to passcode unless biometrics are cancelled.
    func disableToggle(_ key: String, reason: String) async -> Bool {
        print("[Bio] BiometricToggleCoordinator.disableToggle() start")
        do {
            let success = try await auth.authenticate(reason: reason)
            print("[Bio] BiometricToggleCoordinator.disableToggle() biometric auth success: \(success)")
            if success {
                UserDefaults.standard.set(false, forKey: key)
                return true
            }
        } catch let error as BiometricError where error == .cancelled {
            print("[Bio] BiometricToggleCoordinator.disableToggle() biometric auth cancelled by user")
            return false
        } catch {
            print(
                "[Bio] BiometricToggleCoordinator.disableToggle() biometric auth error: \(error), trying passcode fallback"
            )
            try? await Task.sleep(nanoseconds: 300_000_000)
            let passcodeOk = await (try? auth.authenticateWithPasscode(reason: reason)) ?? false
            print("[Bio] BiometricToggleCoordinator.disableToggle() passcode auth result: \(passcodeOk)")
            if passcodeOk {
                UserDefaults.standard.set(false, forKey: key)
                return true
            }
        }
        return false
    }
}
