import Foundation

/// Payment direction from push payload
enum PaymentDirection: String {
    case lspToUser = "lsp_to_user" // LSP owes user (price dropped)
    case userToLsp = "user_to_lsp" // User owes LSP (price rose)
    case incomingPayment = "incoming_payment" // Wake to receive pending

    init(from userInfo: [AnyHashable: Any]) {
        if let stability = userInfo["stability"] as? [String: Any],
           let dir = stability["direction"] as? String {
            self = PaymentDirection(rawValue: dir) ?? .lspToUser
        } else if let stabilityStr = userInfo["stability"] as? String,
                  let data = stabilityStr.data(using: .utf8),
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let dir = json["direction"] as? String {
            self = PaymentDirection(rawValue: dir) ?? .lspToUser
        } else {
            self = .lspToUser
        }
    }

    var notificationType: String {
        switch self {
        case .userToLsp: return "payment_sent"
        case .incomingPayment, .lspToUser: return "payment_received"
        }
    }
}
