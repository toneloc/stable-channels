import Foundation

/// When showing or replacing the recovery phrase asks for Face ID or the passcode.
/// The user's App Access choices decide. Payment Confirmation counts too: a copied phrase
/// could otherwise be spent from another device without it.
enum SeedAccessPolicy {
    static func requiresAuth(appUnlockEnabled: Bool, paymentConfirmationEnabled: Bool) -> Bool {
        appUnlockEnabled || paymentConfirmationEnabled
    }

    /// Leaving the app closes every screen showing the phrase or backup secrets, because sheets sit
    /// above the app lock overlay. Not `.inactive`: Face ID and system prompts raise it.
    static func shouldCloseSeedScreens(for phase: AppLifecyclePhase) -> Bool {
        phase == .background
    }
}

/// The app lifecycle states seed screens react to, kept free of SwiftUI so the rule stays testable.
enum AppLifecyclePhase {
    case active
    case inactive
    case background
}
