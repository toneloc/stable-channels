import Foundation
import LDKNode

// MARK: - Protocol

protocol LNURLServiceProtocol: Sendable {
    func fetchPayParams(from url: URL) async throws -> LNURLPayParams
    func fetchInvoice(
        callback: String,
        amountMsat: UInt64,
        comment: String?,
        expectedMetadataHashHex: String?
    ) async throws -> LNURLPayInvoiceResponse
}

extension LNURLServiceProtocol {
    func fetchInvoice(
        callback: String,
        amountMsat: UInt64,
        comment: String?
    ) async throws -> LNURLPayInvoiceResponse {
        try await fetchInvoice(
            callback: callback,
            amountMsat: amountMsat,
            comment: comment,
            expectedMetadataHashHex: nil
        )
    }
}

// MARK: - Service Implementation

final class LNURLService: LNURLServiceProtocol {
    private let urlSession: URLSession

    init(urlSession: URLSession? = nil) {
        if let session = urlSession {
            self.urlSession = session
        } else {
            let config = URLSessionConfiguration.default
            config.timeoutIntervalForRequest = 15.0
            config.timeoutIntervalForResource = 30.0
            self.urlSession = URLSession(configuration: config)
        }
    }

    /// Resolves an input destination into an actionable HTTPS LNURL endpoint URL.
    /// Handles Lightning addresses (LUD-16), Bech32 LNURL strings (LUD-01), and raw HTTPS URLs.
    static func resolveEndpoint(from input: String) throws -> URL {
        var clean = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if clean.lowercased().hasPrefix("lightning:") {
            clean = String(clean.dropFirst("lightning:".count))
        }

        // LUD-16: Lightning Address (name@domain.com)
        if clean.contains("@") {
            guard clean.filter({ $0 == "@" }).count == 1 else {
                throw LNURLError.invalidTarget
            }
            let parts = clean.split(separator: "@", omittingEmptySubsequences: true)
            guard parts.count == 2 else { throw LNURLError.invalidTarget }
            let username = String(parts[0]).trimmingCharacters(in: .whitespacesAndNewlines)
            let domain = String(parts[1]).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            guard !username.isEmpty, !domain.isEmpty else { throw LNURLError.invalidTarget }

            let allowedUserChars = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: ".-_+"))
            guard username.unicodeScalars.allSatisfy({ allowedUserChars.contains($0) }) else {
                throw LNURLError.invalidTarget
            }

            guard domain.contains("."), !domain.hasPrefix("."), !domain.hasSuffix(".") else {
                throw LNURLError.invalidTarget
            }
            guard !domain.hasSuffix(".onion") else {
                throw LNURLError.insecureEndpoint
            }
            let allowedDomainChars = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: ".-"))
            guard domain.unicodeScalars.allSatisfy({ allowedDomainChars.contains($0) }) else {
                throw LNURLError.invalidTarget
            }

            guard let url = URL(string: "https://\(domain)/.well-known/lnurlp/\(username)") else {
                throw LNURLError.invalidTarget
            }
            return url
        }

        // LUD-01: Bech32 encoded LNURL
        if clean.lowercased().hasPrefix("lnurl1") {
            return try Bech32.decodeLNURL(clean)
        }

        // Direct HTTPS URL
        if let url = URL(string: clean), url.scheme?.lowercased() == "https" {
            return url
        }

        throw LNURLError.invalidTarget
    }

    func fetchPayParams(from url: URL) async throws -> LNURLPayParams {
        guard url.scheme?.lowercased() == "https" else {
            throw LNURLError.insecureEndpoint
        }

        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await urlSession.data(for: request)
        } catch {
            throw LNURLError.networkError(error.localizedDescription)
        }

        try parseErrorResponseIfPresent(data: data)

        guard let httpResponse = response as? HTTPURLResponse, (200...299).contains(httpResponse.statusCode) else {
            if let http = response as? HTTPURLResponse, http.statusCode == 404 {
                throw LNURLError.errorResponse(reason: "Recipient address not found.")
            }
            throw LNURLError.invalidResponse
        }

        let decoder = JSONDecoder()
        guard let params = try? decoder.decode(LNURLPayParams.self, from: data) else {
            throw LNURLError.invalidResponse
        }

        guard params.tag.lowercased() == "payrequest" else {
            throw LNURLError.unsupportedTag(tag: params.tag)
        }

        guard params.hasValidBounds else {
            throw LNURLError.amountOutOfBounds(minSats: params.minSats, maxSats: params.maxSats)
        }

        return params
    }

    func fetchInvoice(
        callback: String,
        amountMsat: UInt64,
        comment: String?,
        expectedMetadataHashHex: String? = nil
    ) async throws -> LNURLPayInvoiceResponse {
        guard let initialUrl = URL(string: callback), initialUrl.scheme?.lowercased() == "https" else {
            throw LNURLError.insecureEndpoint
        }

        guard var components = URLComponents(url: initialUrl, resolvingAgainstBaseURL: false) else {
            throw LNURLError.invalidResponse
        }

        var queryItems = components.queryItems ?? []
        queryItems.append(URLQueryItem(name: "amount", value: "\(amountMsat)"))
        if let comment {
            let trimmed = comment.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty {
                queryItems.append(URLQueryItem(name: "comment", value: trimmed))
            }
        }
        components.queryItems = queryItems

        guard let url = components.url else {
            throw LNURLError.invalidResponse
        }

        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await urlSession.data(for: request)
        } catch {
            throw LNURLError.networkError(error.localizedDescription)
        }

        try parseErrorResponseIfPresent(data: data)

        guard let httpResponse = response as? HTTPURLResponse, (200...299).contains(httpResponse.statusCode) else {
            throw LNURLError.invalidResponse
        }

        let decoder = JSONDecoder()
        guard let invoiceResponse = try? decoder.decode(LNURLPayInvoiceResponse.self, from: data) else {
            throw LNURLError.invalidResponse
        }

        guard !invoiceResponse.isError else {
            let reason = invoiceResponse.reason ?? "The recipient service reported an error."
            throw LNURLError.errorResponse(reason: reason)
        }

        guard !invoiceResponse.pr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw LNURLError.invalidResponse
        }

        // Verify BOLT11 invoice amount, expiration, and metadata hash
        let bolt11 = try Bolt11Invoice.fromStr(invoiceStr: invoiceResponse.pr)
        guard let invoiceMsat = bolt11.amountMilliSatoshis() else {
            throw LNURLError.errorResponse(reason: "Amountless invoices are not permitted for LNURL pay.")
        }
        guard invoiceMsat == amountMsat else {
            throw LNURLError.invoiceAmountMismatch(expectedMsat: amountMsat, actualMsat: invoiceMsat)
        }
        guard !bolt11.isExpired() else {
            throw LNURLError.errorResponse(reason: "The invoice returned by the LNURL service has expired.")
        }

        if let expectedMetadataHashHex {
            switch bolt11.invoiceDescription() {
            case .hash(let hash):
                guard hash.lowercased() == expectedMetadataHashHex.lowercased() else {
                    throw LNURLError.errorResponse(reason: "Invoice description hash does not match payee metadata.")
                }
            case .direct:
                throw LNURLError
                    .errorResponse(
                        reason: "Invoice uses direct description instead of required description hash (h tag)."
                    )
            }
        }

        return invoiceResponse
    }

    private func parseErrorResponseIfPresent(data: Data) throws {
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }

        if let status = json["status"] as? String, status.uppercased() == "ERROR" {
            let reason = (json["reason"] as? String) ?? "The recipient service reported an error."
            throw LNURLError.errorResponse(reason: reason)
        }

        if let reason = json["reason"] as? String, !reason.isEmpty, json["tag"] == nil, json["pr"] == nil {
            throw LNURLError.errorResponse(reason: reason)
        }
    }
}
