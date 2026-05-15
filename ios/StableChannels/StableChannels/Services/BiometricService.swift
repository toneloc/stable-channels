import LocalAuthentication

enum BiometricType {
    case none
    case touchID
    case faceID
}

enum BiometricError: Error {
    case notAvailable
    case notEnrolled
    case cancelled
    case lockout
}

enum BiometricService {
    /// Returns the available biometric type on this device.
    static var biometricType: BiometricType {
        let ctx = LAContext()
        _ = ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
        switch ctx.biometryType {
        case .faceID: return .faceID
        case .touchID: return .touchID
        default: return .none
        }
    }

    /// Checks if biometric authentication is available on this device.
    static var canUseBiometrics: Bool {
        let ctx = LAContext()
        var error: NSError?
        return ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    }

    /// Checks if device passcode is set (used for fallback).
    static var canUseDevicePasscode: Bool {
        let ctx = LAContext()
        var error: NSError?
        return ctx.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
    }

    /// Authenticates user with biometrics first, then falls back to device passcode.
    /// No "Use Passcode" button is shown — fallback is automatic on failure.
    static func authenticate(reason: String) async throws -> Bool {
        let ctx = LAContext()
        ctx.localizedCancelTitle = "Cancel"
        ctx.localizedFallbackTitle = "" // Hide button — we auto-fallback instead

        // Attempt biometric auth
        if canUseBiometrics {
            do {
                return try await ctx.evaluatePolicy(
                    .deviceOwnerAuthenticationWithBiometrics,
                    localizedReason: reason
                )
            } catch let laError as LAError {
                switch laError.code {
                case .userCancel, .appCancel:
                    throw BiometricError.cancelled
                case .biometryLockout:
                    // Biometrics locked — fall through to passcode
                    break
                default:
                    // Any biometric failure — fall through to passcode
                    break
                }
            } catch {
                // Fall through to passcode on any error
            }
        }

        // Auto-fallback to device passcode
        if canUseDevicePasscode {
            return try await ctx.evaluatePolicy(
                .deviceOwnerAuthentication,
                localizedReason: reason
            )
        }

        throw BiometricError.notAvailable
    }
}
