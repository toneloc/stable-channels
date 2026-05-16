import LocalAuthentication

enum BiometricType {
    case none, touchID, faceID
}

enum BiometricError: Error {
    case notAvailable, notEnrolled, cancelled, lockout
}

enum BiometricService {
    /// Guards ContentView scenePhase lock during active biometric auth.
    static var isAuthenticating: Bool = false

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

    static func authenticate(reason: String) async throws -> Bool {
        let ctx = LAContext()
        ctx.localizedCancelTitle = "Cancel"
        ctx.localizedFallbackTitle = ""

        guard canUseBiometrics else {
            throw BiometricError.notAvailable
        }

        isAuthenticating = true
        defer { isAuthenticating = false }

        return try await ctx.evaluatePolicy(
            .deviceOwnerAuthenticationWithBiometrics,
            localizedReason: reason
        )
    }

    static func authenticateWithPasscode(reason: String) async throws -> Bool {
        let ctx = LAContext()
        ctx.localizedCancelTitle = "Cancel"

        isAuthenticating = true
        defer { isAuthenticating = false }

        return try await ctx.evaluatePolicy(
            .deviceOwnerAuthentication,
            localizedReason: reason
        )
    }
}
