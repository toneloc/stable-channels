import Foundation
import LocalAuthentication

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

    func enableToggle(_ key: String, reason: String) async -> Bool {
        guard !isEnabling else { return false }
        isEnabling = true
        defer { isEnabling = false }

        let ctx = LAContext()
        var error: NSError?
        _ = ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
        if let err = error, err.code == LAError.biometryNotAvailable.rawValue {
            requiresSettingsRedirect = true
            return false
        }

        do {
            let success = try await auth.authenticate(reason: reason)
            if success {
                UserDefaults.standard.set(true, forKey: key)
            }
            return success
        } catch {
            return false
        }
    }

    // MARK: - Disable Flow

    func disableToggle(_ key: String, reason: String) async -> Bool {
        do {
            let success = try await auth.authenticate(reason: reason)
            if success {
                UserDefaults.standard.set(false, forKey: key)
                return true
            }
        } catch let error as BiometricError where error == .cancelled {
            return false
        } catch {}
        return false
    }
}
