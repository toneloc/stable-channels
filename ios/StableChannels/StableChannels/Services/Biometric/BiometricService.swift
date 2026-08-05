import LocalAuthentication

final class BiometricService: BiometricCapabilityChecking, BiometricAuthenticating {
    static let shared = BiometricService()

    // MARK: - BiometricCapabilityChecking

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

        // Pre-flight check to request/check biometric permissions, ensuring Face ID is tried first if available
        var error: NSError?
        _ = ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)

        do {
            return try await ctx.evaluatePolicy(
                .deviceOwnerAuthentication,
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
