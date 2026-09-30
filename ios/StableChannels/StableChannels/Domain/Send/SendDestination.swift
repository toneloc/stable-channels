import Foundation
import LDKNode

/// Represents a validated send destination target.
enum SendDestination: Equatable, Sendable {
    case bolt11(invoice: Bolt11Invoice, raw: String, amountMsat: UInt64?)
    case bolt12(offer: Offer, raw: String)
    case onchain(address: String)
    case lightningAddress(handle: String, domain: String, url: URL)
    case lnurlPay(url: URL)

    var displayTitle: String {
        switch self {
        case .bolt11:
            return "Lightning (BOLT11) Invoice"
        case .bolt12:
            return "Lightning (BOLT12) Offer"
        case .onchain:
            return "Bitcoin Address"
        case .lightningAddress(let handle, let domain, _):
            return "\(handle)@\(domain)"
        case .lnurlPay(let url):
            return url.host ?? "LNURL-Pay"
        }
    }

    var requiresManualAmount: Bool {
        switch self {
        case .bolt11(_, _, let msat):
            return msat == nil || msat == 0
        case .bolt12, .onchain, .lightningAddress, .lnurlPay:
            return true
        }
    }

    var rawDestination: String {
        switch self {
        case .bolt11(_, let raw, _):
            return raw
        case .bolt12(_, let raw):
            return raw
        case .onchain(let address):
            return address
        case .lightningAddress(let handle, let domain, _):
            return "\(handle)@\(domain)"
        case .lnurlPay(let url):
            return url.absoluteString
        }
    }
}

/// Result of evaluating a user-entered payment destination.
enum PaymentDestinationClassification: Equatable, Sendable {
    case valid(SendDestination)
    case invalid(reason: String)
    case empty
}

/// Pure classifier for parsing and validating Bitcoin and Lightning destinations.
enum PaymentDestinationClassifier {
    private static let emailRegex = try? NSRegularExpression(
        pattern: "^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\\.[a-zA-Z0-9-.]+$",
        options: .caseInsensitive
    )

    static func classify(_ input: String) -> PaymentDestinationClassification {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return .empty
        }

        var normalized = trimmed
        let lowerTrimmed = normalized.lowercased()
        if lowerTrimmed.hasPrefix("lightning://") {
            normalized = String(normalized.dropFirst("lightning://".count))
        } else if lowerTrimmed.hasPrefix("lightning:") {
            normalized = String(normalized.dropFirst("lightning:".count))
        }

        if normalized.lowercased().hasPrefix("bitcoin:") {
            if let bip21 = parseBIP21(normalized) {
                return bip21
            }
            normalized = String(normalized.dropFirst("bitcoin:".count))
        }

        let lower = normalized.lowercased()

        // 1. Bolt11 Invoice
        if lower.hasPrefix("lnbc") || lower.hasPrefix("lntb") || lower.hasPrefix("lnts") {
            if let invoice = try? Bolt11Invoice.fromStr(invoiceStr: normalized) {
                let msat = invoice.amountMilliSatoshis()
                return .valid(.bolt11(invoice: invoice, raw: normalized, amountMsat: msat))
            }
            return .invalid(reason: "Invalid Lightning invoice checksum or format.")
        }

        // 2. Bolt12 Offer
        if lower.hasPrefix("lno") {
            if let offer = try? Offer.fromStr(offerStr: normalized) {
                return .valid(.bolt12(offer: offer, raw: normalized))
            }
            return .invalid(reason: "Invalid Bolt12 offer string.")
        }

        // 3. LNURL-pay bech32
        if lower.hasPrefix("lnurl1") {
            if let url = try? Bech32.decodeLNURL(normalized) {
                return .valid(.lnurlPay(url: url))
            }
            return .invalid(reason: "Invalid LNURL format or checksum.")
        }

        // 4. Lightning Address (user@domain)
        if normalized.contains("@") && !normalized.contains(" ") {
            if let match = classifyLightningAddress(normalized) {
                return match
            }
        }

        // 5. Onchain Address
        if lower.hasPrefix("bc1") || lower.hasPrefix("tb1") || lower.hasPrefix("bcrt1") ||
            lower.hasPrefix("1") || lower.hasPrefix("3") ||
            lower.hasPrefix("m") || lower.hasPrefix("n") || lower.hasPrefix("2") {
            if isValidOnchainAddress(normalized) {
                return .valid(.onchain(address: normalized))
            }
            return .invalid(reason: "Invalid onchain address length or characters.")
        }

        return .invalid(reason: "Unrecognized payment format.")
    }

    private static func classifyLightningAddress(_ input: String) -> PaymentDestinationClassification? {
        let parts = input.split(separator: "@", maxSplits: 1, omittingEmptySubsequences: true)
        guard parts.count == 2 else { return nil }
        let handle = String(parts[0])
        let domain = String(parts[1])

        guard let regex = emailRegex else { return nil }
        let range = NSRange(location: 0, length: input.utf16.count)
        guard regex.firstMatch(in: input, options: [], range: range) != nil else {
            return .invalid(reason: "Invalid Lightning Address format.")
        }

        guard let url = URL(string: "https://\(domain)/.well-known/lnurlp/\(handle)") else {
            return .invalid(reason: "Could not create URL for Lightning Address.")
        }

        return .valid(.lightningAddress(handle: handle, domain: domain, url: url))
    }

    private static func parseBIP21(_ uri: String) -> PaymentDestinationClassification? {
        guard let components = URLComponents(string: uri) else { return nil }
        if let lightningQuery = components.queryItems?.first(where: { $0.name.lowercased() == "lightning" })?.value,
           !lightningQuery.isEmpty {
            return classify(lightningQuery)
        }
        let address = components.path.isEmpty ? (components.host ?? "") : components.path
        if !address.isEmpty && isValidOnchainAddress(address) {
            return .valid(.onchain(address: address))
        }
        return nil
    }

    private static func isValidOnchainAddress(_ address: String) -> Bool {
        let count = address.count
        guard count >= 26 && count <= 90 else { return false }
        let lower = address.lowercased()

        // Bech32 / Bech32m addresses (Native Segwit & Taproot)
        if lower.hasPrefix("bc1") {
            return Bech32.verifySegwitAddress(address, expectedHrp: "bc")
        }
        if lower.hasPrefix("tb1") {
            return Bech32.verifySegwitAddress(address, expectedHrp: "tb")
        }
        if lower.hasPrefix("bcrt1") {
            return Bech32.verifySegwitAddress(address, expectedHrp: "bcrt")
        }

        // Base58 Legacy / Nested Segwit addresses (1, 3, m, n, 2)
        if address.hasPrefix("1") || address.hasPrefix("3") || address.hasPrefix("m") || address
            .hasPrefix("n") || address.hasPrefix("2") {
            guard count >= 26 && count <= 35 else { return false }
            return Base58Check.verify(address)
        }

        return false
    }
}
