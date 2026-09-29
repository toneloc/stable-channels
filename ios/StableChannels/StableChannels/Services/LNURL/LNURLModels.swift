import CryptoKit
import Foundation

// MARK: - Pay Parameters Model (LUD-06)

/// LNURL-pay parameters returned from LUD-06 first-step GET request.
struct LNURLPayParams: Codable, Equatable, Sendable {
    static let maxSupplyMsat: UInt64 = 21_000_000 * 100_000_000 * 1000

    let tag: String
    let callback: String
    let minSendable: UInt64 // millisatoshis
    let maxSendable: UInt64 // millisatoshis
    let metadata: String
    let commentAllowed: Int?

    var minSats: UInt64 {
        let quotient = minSendable / 1000
        let remainder = minSendable % 1000
        return remainder > 0 ? (quotient + 1) : quotient
    }

    var maxSats: UInt64 {
        maxSendable / 1000
    }

    var hasCustomSendBounds: Bool {
        minSendable > 1000 || maxSendable < Self.maxSupplyMsat
    }

    /// Verifies that payment bounds conform to protocol invariants and supply caps.
    var hasValidBounds: Bool {
        minSendable > 0 && minSendable <= maxSendable && maxSendable <= Self.maxSupplyMsat
    }

    /// Validates payment amount in millisatoshis against protocol limits.
    func isAmountValid(msat: UInt64) -> Bool {
        msat >= minSendable && msat <= maxSendable
    }

    /// Computes the SHA-256 hash hex string of the UTF-8 encoded metadata.
    /// LUD-06 requires this hash to match the BOLT11 invoice description_hash (h tag).
    var metadataHashHex: String {
        let digest = SHA256.hash(data: Data(metadata.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    /// Extracts the first plain text description from metadata JSON array per LUD-06.
    var plainTextDescription: String? {
        guard let data = metadata.data(using: .utf8),
              let jsonArray = try? JSONSerialization.jsonObject(with: data) as? [[String]] else {
            return nil
        }
        for item in jsonArray where item.count >= 2 && item[0] == "text/plain" {
            return item[1]
        }
        return nil
    }

    /// Extracts an optional base64 image (PNG or JPEG) payload from metadata.
    var imageDescription: (mimeType: String, base64Data: String)? {
        guard let data = metadata.data(using: .utf8),
              let jsonArray = try? JSONSerialization.jsonObject(with: data) as? [[String]] else {
            return nil
        }
        for item in jsonArray where item.count >= 2 {
            let mime = item[0]
            if mime == "image/png;base64" || mime == "image/jpeg;base64" {
                return (mimeType: mime, base64Data: item[1])
            }
        }
        return nil
    }

    /// Validates whether a comment satisfies the payee comment character limit per LUD-12.
    func isCommentValid(_ comment: String?) -> Bool {
        guard let comment, !comment.isEmpty else { return true }
        guard let allowed = commentAllowed, allowed > 0 else { return false }
        return comment.count <= allowed
    }
}

// MARK: - Success Action Model (LUD-09 / LUD-10)

/// Success action returned after LNURL payment completion.
struct LNURLSuccessAction: Codable, Equatable, Sendable {
    let tag: String
    let description: String?
    let url: String?
    let message: String?
    let ciphertext: String?
    let iv: String?

    enum ActionType: Equatable {
        case message(String)
        case url(description: String, url: URL)
        case aes(description: String, ciphertext: String, iv: String)
        case unknown(tag: String)
    }

    var actionType: ActionType {
        switch tag.lowercased() {
        case "message":
            return .message(message ?? "")
        case "url":
            if let desc = description,
               let urlStr = url,
               let parsedURL = URL(string: urlStr),
               let scheme = parsedURL.scheme?.lowercased() {
                let isSecure = scheme == "https" ||
                    (scheme == "http" && parsedURL.host?.lowercased().hasSuffix(".onion") == true)
                if isSecure {
                    return .url(description: desc, url: parsedURL)
                }
            }
            return .unknown(tag: tag)
        case "aes":
            if let desc = description, let cipher = ciphertext, let initVector = iv {
                return .aes(description: desc, ciphertext: cipher, iv: initVector)
            }
            return .unknown(tag: tag)
        default:
            return .unknown(tag: tag)
        }
    }

    /// Checks if action URL shares the host or subdomain of the callback endpoint.
    func isSameHostOrSubdomain(callbackURL: URL) -> Bool {
        guard let urlStr = url, let actionURL = URL(string: urlStr),
              let actionHost = actionURL.host?.lowercased(),
              let callbackHost = callbackURL.host?.lowercased() else {
            return false
        }
        return actionHost == callbackHost || actionHost.hasSuffix("." + callbackHost)
    }
}

// MARK: - Invoice Response Model

/// Invoice response returned from LNURL-pay second-step GET request.
struct LNURLPayInvoiceResponse: Codable, Equatable, Sendable {
    let pr: String
    let successAction: LNURLSuccessAction?
    let status: String?
    let reason: String?

    var isError: Bool {
        status?.uppercased() == "ERROR"
    }
}

// MARK: - Errors

enum LNURLError: Swift.Error, LocalizedError, Equatable {
    case invalidTarget
    case invalidResponse
    case errorResponse(reason: String)
    case unsupportedTag(tag: String)
    case amountOutOfBounds(minSats: UInt64, maxSats: UInt64)
    case invoiceAmountMismatch(expectedMsat: UInt64, actualMsat: UInt64)
    case invalidMetadata
    case insecureEndpoint
    case networkError(String)

    var errorDescription: String? {
        switch self {
        case .invalidTarget:
            return "The provided address or LNURL is invalid."
        case .invalidResponse:
            return "Received an invalid or malformed response from the LNURL server."
        case let .errorResponse(reason):
            return reason
        case let .unsupportedTag(tag):
            return "Unsupported LNURL tag: \(tag). Only LNURL-pay is supported."
        case let .amountOutOfBounds(minSats, maxSats):
            return "Amount must be between \(minSats) and \(maxSats) sats."
        case let .invoiceAmountMismatch(expected, actual):
            return "Invoice amount (\(actual / 1000) sats) does not match requested amount (\(expected / 1000) sats)."
        case .invalidMetadata:
            return "LNURL metadata is invalid or missing required description."
        case .insecureEndpoint:
            return "LNURL endpoint must use HTTPS."
        case let .networkError(msg):
            return "LNURL network request failed: \(msg)"
        }
    }
}
