import Foundation

/// When showing or replacing the recovery phrase asks for Face ID or the passcode.
/// The user's App Access choices decide. Payment Confirmation counts too: a copied phrase
/// could otherwise be spent from another device without it.
enum SeedAccessPolicy {
    static func requiresAuth(appUnlockEnabled: Bool, paymentConfirmationEnabled: Bool) -> Bool {
        appUnlockEnabled || paymentConfirmationEnabled
    }
}
