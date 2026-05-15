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
    case failed(String)
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

    /// Authenticate using Face ID/Touch ID first, then auto-fallback to passcode.
    /// Never shows a "Use Passcode" button — goes straight to passcode on failure.
    static func authenticate(reason: String) async throws -> Bool {
        let ctx = LAContext()
        ctx.localizedCancelTitle = "Cancel"
        // Hide the fallback button — we auto-fallback instead
        ctx.localizedFallbackTitle = ""

        // Try biometrics first
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
