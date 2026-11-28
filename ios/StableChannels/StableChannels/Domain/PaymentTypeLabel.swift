import Foundation

/// The label History and Payment Detail show for a payment's stored type. Without stability in the build, a
/// stability payment (a Lightning keysend) is labelled Lightning.
enum PaymentTypeLabel {
    static func text(for paymentType: String, stabilityEnabled: Bool = BuildFeatures.stability) -> String {
        switch paymentType {
        case "stability" where stabilityEnabled:
            return String(localized: "payment_type_stability", defaultValue: "Stability")
        case "stability", "lightning":
            return String(localized: "payment_type_lightning", defaultValue: "Lightning")
        case SplicePaymentType.spliceIn.rawValue:
            return String(localized: "payment_type_splice_in", defaultValue: "Splice In")
        case SplicePaymentType.spliceOut.rawValue:
            return String(localized: "payment_type_splice_out", defaultValue: "Splice Out")
        case "onchain":
            return String(localized: "payment_type_on_chain", defaultValue: "Onchain")
        case "channel_close":
            return String(localized: "payment_type_channel_close", defaultValue: "Channel Close")
        case "bolt12":
            return String(localized: "payment_type_bolt12", defaultValue: "Bolt12")
        default:
            return paymentType
        }
    }
}
