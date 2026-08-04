import LocalAuthentication

/// Concrete implementation of biometric capability checking and authentication.
///
/// Conforms to both `BiometricCapabilityChecking` and `BiometricAuthenticating`
/// so callers can depend on whichever slice they need (Interface Segregation).
/// Instantiated as a singleton via `shared`; injected through protocols for testability.
final class BiometricService: BiometricCapabilityChecking, BiometricAuthenticating {
    // MARK: - Singleton

    static let shared = BiometricService()

    // MARK: - BiometricCapabilityChecking

    /// Returns the device's biometric hardware type.
    /// Uses `.deviceOwnerAuthentication` to populate `biometryType` even when
    /// the user has revoked Face ID permission in iOS Settings — that policy
    /// succeeds whenever a passcode is set and still reports the hardware.
    var biometricType: BiometricType {
        let ctx = LAContext()
        _ = ctx.canEvaluatePolicy(.deviceOwnerAuthentication, error: nil)
        switch ctx.biometryType {
        case .faceID: return .faceID
        case .touchID: return .touchID
        default: return .none
        }
    }

    var canUseBiometrics: Bool {
        let ctx = LAContext()
        var error: NSError?
        return ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    }

    var canUseDevicePasscode: Bool {
        let ctx = LAContext()
        var error: NSError?
        return ctx.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
    }

    // MARK: - BiometricAuthenticating

    @MainActor
    func authenticate(reason: String) async throws -> Bool {
        let ctx = LAContext()
        ctx.localizedCancelTitle = "Cancel"
        ctx.localizedFallbackTitle = ""

        // No canUseBiometrics guard — let evaluatePolicy run so iOS can
        // show the "Allow Face ID" permission dialog when the user has
        // previously denied or revoked permission in Settings.
        do {
            return try await ctx.evaluatePolicy(
                .deviceOwnerAuthenticationWithBiometrics,
                localizedReason: reason
            )
        } catch {
            throw Self.classifyLAError(error)
        }
    }

    @MainActor
    func authenticateWithPasscode(reason: String) async throws -> Bool {
        let ctx = LAContext()
        ctx.localizedCancelTitle = "Cancel"

        do {
            return try await ctx.evaluatePolicy(
                .deviceOwnerAuthentication,
                localizedReason: reason
            )
        } catch {
            if let laError = error as? LAError, laError.code == .userCancel {
                throw BiometricError.cancelled
            }
            throw BiometricError.passcodeFailed
        }
    }

    // MARK: - Private

    /// Classifies LAError code into BiometricError for user-facing feedback.
    private static func classifyLAError(_ error: Error) -> BiometricError {
        guard let laError = error as? LAError else {
            return .biometryFailed
        }
        switch laError.code {
        case .biometryNotAvailable: return .notAvailable
        case .biometryNotEnrolled: return .notEnrolled
        case .biometryLockout: return .lockout
        case .userCancel, .systemCancel, .appCancel: return .cancelled
        default: return .biometryFailed
        }
    }
}
