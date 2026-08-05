import LocalAuthentication

// MARK: - Shared Types

enum BiometricType {
    case none, touchID, faceID
}

enum BiometricError: Error, LocalizedError, Equatable {
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

// MARK: - Capability Checking (Interface Segregation)

protocol BiometricCapabilityChecking: Sendable {
    var biometricType: BiometricType { get }
    var canUseBiometrics: Bool { get }
    var canUseDevicePasscode: Bool { get }
}

// MARK: - Authentication (Interface Segregation)

protocol BiometricAuthenticating: Sendable {
    @MainActor func authenticate(reason: String) async throws -> Bool
}
