import LocalAuthentication

enum BiometricType {
    case none, touchID, faceID
}

enum BiometricError: Error, LocalizedError {
    case notAvailable
    case notEnrolled
    case cancelled
    case lockout
    case biometryFailed
    case passcodeFailed

    var errorDescription: String? {
        switch self {
        case .notAvailable: return "Biometric authentication is not available on this device."
        case .notEnrolled: return "No biometrics enrolled. Please set up Face ID or Touch ID in Settings."
        case .cancelled: return "Authentication was cancelled."
        case .lockout: return "Biometrics locked. Please use your device passcode."
        case .biometryFailed: return "Biometric authentication failed. Try again or use your passcode."
        case .passcodeFailed: return "Authentication failed. Please try again."
        }
    }
}

enum BiometricService {
    static var biometricType: BiometricType {
        let ctx = LAContext()
        _ = ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
        switch ctx.biometryType {
        case .faceID: return .faceID
        case .touchID: return .touchID
        default: return .none
        }
    }

    static var canUseBiometrics: Bool {
        let ctx = LAContext()
        var error: NSError?
        return ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    }

    static var canUseDevicePasscode: Bool {
        let ctx = LAContext()
        var error: NSError?
        return ctx.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
    }

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

    @MainActor static func authenticate(reason: String) async throws -> Bool {
        let ctx = LAContext()
        ctx.localizedCancelTitle = "Cancel"
        ctx.localizedFallbackTitle = ""

        guard canUseBiometrics else {
            throw BiometricError.notAvailable
        }

        do {
            return try await ctx.evaluatePolicy(
                .deviceOwnerAuthenticationWithBiometrics,
                localizedReason: reason
            )
        } catch {
            throw classifyLAError(error)
        }
    }

    @MainActor static func authenticateWithPasscode(reason: String) async throws -> Bool {
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
}
