import Foundation
import LDKNode

/// Represents a validated send destination target.
enum SendDestination: Equatable, Sendable {
    case bolt11(invoice: Bolt11Invoice, raw: String, amountMsat: UInt64?)
    case bolt12(offer: Offer, raw: String)
    case onchain(address: String, amountSats: UInt64?)
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
        case .onchain(_, let amountSats):
            return amountSats == nil || amountSats == 0
        case .bolt12, .lightningAddress, .lnurlPay:
            return true
        }
    }

    var rawDestination: String {
        switch self {
        case .bolt11(_, let raw, _):
            return raw
        case .bolt12(_, let raw):
            return raw
        case .onchain(let address, _):
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
    private static let btcAmountRegex = try? NSRegularExpression(
        pattern: "^[0-9]+(?:\\.[0-9]{1,8})?$"
    )

    static func classify(_ input: String, network: Network? = nil) -> PaymentDestinationClassification {
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
            if let bip21 = parseBIP21(normalized, network: network) {
                return bip21
            }
            normalized = String(normalized.dropFirst("bitcoin:".count))
        }

        // 1. Lightning Address (user@domain) - checked before "ln" branch because handles like lnbits@... start with
        // "ln"
        if normalized.contains("@") && !normalized.contains(" ") {
            if let match = classifyLightningAddress(normalized) {
                return match
            }
        }

        let lower = normalized.lowercased()

        // 2. Bolt11, Bolt12, or LNURL
        if lower.hasPrefix("ln") {
            if lower.hasPrefix("lnbcrt") || lower.hasPrefix("lnbc") || lower.hasPrefix("lntbs") || lower
                .hasPrefix("lntb") {
                if let invoice = try? Bolt11Invoice.fromStr(invoiceStr: normalized) {
                    if let network {
                        let currency = invoice.currency()
                        let matches = (network == .bitcoin && currency == .bitcoin) ||
                            (network == .testnet && currency == .bitcoinTestnet) ||
                            (network == .signet && currency == .signet) ||
                            (network == .regtest && currency == .regtest)
                        guard matches else {
                            return .invalid(reason: "Invoice network does not match active network.")
                        }
                    }
                    let msat = invoice.amountMilliSatoshis()
                    return .valid(.bolt11(invoice: invoice, raw: normalized, amountMsat: msat))
                }
                return .invalid(reason: "Invalid Lightning invoice checksum or format.")
            } else if lower.hasPrefix("lno") {
                if let offer = try? Offer.fromStr(offerStr: normalized) {
                    if let network {
                        let chains = offer.chains()
                        let matches = chains.isEmpty ? (network == .bitcoin) : chains.contains(network)
                        guard matches else {
                            return .invalid(reason: "Offer network does not match active network.")
                        }
                    }
                    return .valid(.bolt12(offer: offer, raw: normalized))
                }
                return .invalid(reason: "Invalid Bolt12 offer string.")
            } else if lower.hasPrefix("lnurl1") {
                if let url = try? Bech32.decodeLNURL(normalized) {
                    return .valid(.lnurlPay(url: url))
                }
                return .invalid(reason: "Invalid LNURL format or checksum.")
            } else {
                return .invalid(reason: "Unsupported or unrecognized Lightning invoice network.")
            }
        }

        // 3. Onchain Address
        if lower.hasPrefix("bc1") || lower.hasPrefix("tb1") || lower.hasPrefix("bcrt1") ||
            lower.hasPrefix("1") || lower.hasPrefix("3") ||
            lower.hasPrefix("m") || lower.hasPrefix("n") || lower.hasPrefix("2") {
            if isValidOnchainAddress(normalized, network: network) {
                let finalAddress = (lower.hasPrefix("bc1") || lower.hasPrefix("tb1") || lower.hasPrefix("bcrt1")) ?
                    normalized.lowercased() : normalized
                return .valid(.onchain(address: finalAddress, amountSats: nil))
            }
            if network != nil && isValidOnchainAddress(normalized, network: nil) {
                return .invalid(reason: "Address network does not match active network.")
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

        var components = URLComponents()
        components.scheme = "https"
        components.host = domain
        let encodedHandle = handle.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? handle
        components.path = "/.well-known/lnurlp/\(encodedHandle)"

        guard let url = components.url else {
            return .invalid(reason: "Could not create URL for Lightning Address.")
        }

        return .valid(.lightningAddress(handle: handle, domain: domain, url: url))
    }

    private static func parseBIP21(_ uri: String, network: Network? = nil) -> PaymentDestinationClassification? {
        var raw = uri
        if raw.lowercased().hasPrefix("bitcoin://") {
            raw = String(raw.dropFirst("bitcoin://".count))
        } else if raw.lowercased().hasPrefix("bitcoin:") {
            raw = String(raw.dropFirst("bitcoin:".count))
        }

        let addressPart: String
        let queryPart: String?
        if let queryIndex = raw.firstIndex(of: "?") {
            addressPart = String(raw[..<queryIndex])
            queryPart = String(raw[raw.index(after: queryIndex)...])
        } else {
            addressPart = raw
            queryPart = nil
        }

        let address = addressPart.trimmingCharacters(in: .whitespacesAndNewlines)

        var amountSats: UInt64?
        var lightningParam: String?
        var bolt12Param: String?
        var seenKeys = Set<String>()
        var duplicateDetected = false
        var unhandledRequiredParam: String?

        if let query = queryPart {
            var dummyComponents = URLComponents()
            dummyComponents.query = query
            if let queryItems = dummyComponents.queryItems {
                for item in queryItems {
                    let name = item.name.lowercased()
                    if name.hasPrefix("req-") {
                        unhandledRequiredParam = name
                    } else if name == "pop" {
                        unhandledRequiredParam = "pop"
                    }

                    if ["amount", "lightning", "lno"].contains(name) {
                        if seenKeys.contains(name) {
                            duplicateDetected = true
                        }
                        seenKeys.insert(name)
                    }

                    if name == "amount", let val = item.value {
                        amountSats = parseBTCAmountToSats(val)
                    } else if name == "lightning", let val = item.value, !val.isEmpty {
                        lightningParam = val
                    } else if name == "lno", let val = item.value, !val.isEmpty {
                        bolt12Param = val
                    }
                }
            }
        }

        if let unhandled = unhandledRequiredParam {
            return .invalid(reason: "Unhandled required BIP21 parameter: \(unhandled)")
        }
        if duplicateDetected {
            return .invalid(reason: "Duplicate BIP21 parameter detected.")
        }

        // Explicit precedence: If lightning fallback invoice or offer is valid, prefer Lightning
        if let lightning = lightningParam {
            let classified = classify(lightning, network: network)
            if case .valid(let dest) = classified {
                switch dest {
                case .bolt11, .bolt12:
                    return .valid(dest)
                default:
                    break
                }
            }
        }

        if let lno = bolt12Param {
            let classified = classify(lno, network: network)
            if case .valid(let dest) = classified {
                switch dest {
                case .bolt12:
                    return .valid(dest)
                default:
                    break
                }
            }
        }

        // Otherwise require a valid onchain address
        guard !address.isEmpty else {
            return nil
        }

        if isValidOnchainAddress(address, network: network) {
            let lowerAddr = address.lowercased()
            let finalAddr = (lowerAddr.hasPrefix("bc1") || lowerAddr.hasPrefix("tb1") || lowerAddr.hasPrefix("bcrt1")) ?
                address.lowercased() : address
            return .valid(.onchain(address: finalAddr, amountSats: amountSats))
        }

        if network != nil && isValidOnchainAddress(address, network: nil) {
            return .invalid(reason: "Address network does not match active network.")
        }

        return nil
    }

    private static func parseBTCAmountToSats(_ btcString: String) -> UInt64? {
        guard let regex = btcAmountRegex else { return nil }
        let range = NSRange(location: 0, length: btcString.utf16.count)
        guard regex.firstMatch(in: btcString, options: [], range: range) != nil else { return nil }
        guard let decimal = Decimal(string: btcString), decimal > 0 else { return nil }
        let satsDecimal = decimal * 100_000_000
        let nsDecimal = satsDecimal as NSDecimalNumber
        guard nsDecimal.doubleValue <= 2_100_000_000_000_000 else { return nil }
        let sats = nsDecimal.uint64Value
        guard satsDecimal == Decimal(sats) else { return nil }
        return sats
    }

    private static func isValidOnchainAddress(_ address: String, network: Network? = nil) -> Bool {
        let count = address.count
        guard count >= 26 && count <= 90 else { return false }
        let lower = address.lowercased()

        // Bech32 / Bech32m addresses (Native Segwit & Taproot)
        if lower.hasPrefix("bc1") {
            guard network == nil || network == .bitcoin else { return false }
            return Bech32.verifySegwitAddress(address, expectedHrp: "bc")
        }
        if lower.hasPrefix("tb1") {
            guard network == nil || network == .testnet || network == .signet else { return false }
            return Bech32.verifySegwitAddress(address, expectedHrp: "tb")
        }
        if lower.hasPrefix("bcrt1") {
            guard network == nil || network == .regtest else { return false }
            return Bech32.verifySegwitAddress(address, expectedHrp: "bcrt")
        }

        // Base58 Legacy / Nested Segwit addresses (1, 3, m, n, 2)
        if address.hasPrefix("1") || address.hasPrefix("3") {
            guard network == nil || network == .bitcoin else { return false }
            guard count >= 26 && count <= 35 else { return false }
            return Base58Check.verify(address)
        }
        if address.hasPrefix("m") || address.hasPrefix("n") || address.hasPrefix("2") {
            guard network == nil || network == .testnet || network == .signet || network == .regtest
            else { return false }
            guard count >= 26 && count <= 35 else { return false }
            return Base58Check.verify(address)
        }

        return false
    }
}
