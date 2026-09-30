import Foundation
import LDKNode

// MARK: - LNURL Service Implementation

final class LNURLService: LNURLServiceProtocol {
    private let urlSession: URLSession
    private let hostResolver: HostIPResolving

    init(urlSession: URLSession? = nil, hostResolver: HostIPResolving = SystemHostIPResolver()) {
        self.hostResolver = hostResolver
        if let session = urlSession {
            self.urlSession = session
        } else {
            let config = URLSessionConfiguration.default
            config.timeoutIntervalForRequest = 15.0
            config.timeoutIntervalForResource = 30.0
            self.urlSession = URLSession(configuration: config)
        }
    }

    /// Checks if a given host is a private, loopback, or link-local address to prevent SSRF.
    static func isPrivateOrLoopbackHost(_ host: String) -> Bool {
        SecureEndpointValidator.isPrivateOrLoopbackHost(host)
    }

    /// Validates transport security: strict HTTPS for clearnet, HTTP or HTTPS for Tor (.onion) hidden services.
    /// Also rejects any loopback, private, or link-local hosts, as well as DNS names resolving to restricted IPs.
    static func isSecureEndpoint(url: URL, hostResolver: HostIPResolving = SystemHostIPResolver()) -> Bool {
        SecureEndpointValidator.isSecureEndpoint(url: url, hostResolver: hostResolver)
    }

    /// Resolves an input destination into an actionable HTTPS LNURL endpoint URL.
    /// Handles Lightning addresses (LUD-16), Bech32 LNURL strings (LUD-01), and raw HTTPS URLs.
    static func resolveEndpoint(
        from input: String,
        hostResolver: HostIPResolving = SystemHostIPResolver()
    ) throws -> URL {
        var clean = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if let range = clean.range(of: "lightning://", options: [.caseInsensitive, .anchored]) {
            clean.removeSubrange(range)
        } else if let range = clean.range(of: "lightning:", options: [.caseInsensitive, .anchored]) {
            clean.removeSubrange(range)
        }
        clean = clean.trimmingCharacters(in: .whitespacesAndNewlines)

        let resolvedURL: URL
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

            // Prevent path traversal, query injection, or delimiter manipulation in username
            let forbiddenUserChars = CharacterSet(charactersIn: "/?#@: \\\"%<>{}|^`[]")
            guard username.rangeOfCharacter(from: forbiddenUserChars) == nil,
                  username.rangeOfCharacter(from: .controlCharacters) == nil else {
                throw LNURLError.invalidTarget
            }

            guard domain.contains("."), !domain.hasPrefix("."), !domain.hasSuffix(".") else {
                throw LNURLError.invalidTarget
            }
            let allowedDomainChars = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: ".-"))
            guard domain.unicodeScalars.allSatisfy({ allowedDomainChars.contains($0) }) else {
                throw LNURLError.invalidTarget
            }

            guard !isPrivateOrLoopbackHost(domain) else {
                throw LNURLError.invalidTarget
            }

            let scheme = domain.hasSuffix(".onion") ? "http" : "https"
            guard let url = URL(string: "\(scheme)://\(domain)/.well-known/lnurlp/\(username)") else {
                throw LNURLError.invalidTarget
            }
            resolvedURL = url
        } else if clean.lowercased().hasPrefix("lnurl1") {
            // LUD-01: Bech32 encoded LNURL
            resolvedURL = try Bech32.decodeLNURL(clean)
        } else if let directURL = URL(string: clean) {
            // Direct URL (HTTPS for clearnet, HTTP/HTTPS for Tor .onion)
            resolvedURL = directURL
        } else {
            throw LNURLError.invalidTarget
        }

        guard isSecureEndpoint(url: resolvedURL, hostResolver: hostResolver) else {
            throw LNURLError.invalidTarget
        }

        return resolvedURL
    }

    private func executeSecureGet(url: URL) async throws -> (Data, HTTPURLResponse) {
        guard let pinned = SecureEndpointValidator.preparePinnedRequest(from: url, hostResolver: hostResolver) else {
            throw LNURLError.insecureEndpoint
        }

        let redirectDelegate = SecureRedirectDelegate(
            hostResolver: hostResolver,
            initialExpectedHost: pinned.expectedHost
        )
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await urlSession.data(for: pinned.request, delegate: redirectDelegate)
        } catch {
            throw LNURLError.networkError(error.localizedDescription)
        }

        if redirectDelegate.encounteredInsecureRedirect {
            throw LNURLError.insecureEndpoint
        }

        guard let httpResponse = response as? HTTPURLResponse,
              let finalURL = httpResponse.url else {
            throw LNURLError.invalidResponse
        }

        guard Self.isSecureEndpoint(url: finalURL, hostResolver: hostResolver) else {
            throw LNURLError.insecureEndpoint
        }

        return (data, httpResponse)
    }

    func fetchPayParams(from url: URL) async throws -> LNURLPayParams {
        let (data, httpResponse) = try await executeSecureGet(url: url)

        guard (200...299).contains(httpResponse.statusCode) else {
            if httpResponse.statusCode == 404 {
                throw LNURLError.errorResponse(reason: "Recipient address not found.")
            }
            throw LNURLError.invalidResponse
        }

        try throwIfErrorResponse(data: data)

        let decoder = JSONDecoder()
        guard let params = try? decoder.decode(LNURLPayParams.self, from: data) else {
            throw LNURLError.invalidResponse
        }

        guard params.tag.lowercased() == "payrequest" else {
            throw LNURLError.unsupportedTag(tag: params.tag)
        }

        // Validate metadata has at least one valid text/plain entry per LUD-06
        guard params.plainTextDescription != nil else {
            throw LNURLError.invalidMetadata
        }

        guard params.hasValidBounds else {
            throw LNURLError.amountOutOfBounds(minSats: params.minSats, maxSats: params.maxSats)
        }

        return params
    }

    func fetchInvoice(
        params: LNURLPayParams,
        amountMsat: UInt64,
        comment: String? = nil
    ) async throws -> LNURLPayInvoiceResponse {
        guard params.isAmountValid(msat: amountMsat) else {
            throw LNURLError.amountOutOfBounds(minSats: params.minSats, maxSats: params.maxSats)
        }
        guard params.isCommentValid(comment) else {
            throw LNURLError.invalidResponse
        }
        return try await fetchInvoiceRaw(
            callback: params.callback,
            amountMsat: amountMsat,
            comment: comment,
            expectedMetadataHashHex: params.metadataHashHex
        )
    }

    private func fetchInvoiceRaw(
        callback: String,
        amountMsat: UInt64,
        comment: String?,
        expectedMetadataHashHex: String
    ) async throws -> LNURLPayInvoiceResponse {
        guard let initialUrl = URL(string: callback) else {
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

        let (data, httpResponse) = try await executeSecureGet(url: url)

        guard (200...299).contains(httpResponse.statusCode) else {
            throw LNURLError.invalidResponse
        }

        try throwIfErrorResponse(data: data)

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

        // LNURL-pay callback requests an exact amount in millisatoshis.
        // Reject invoices whose encoded amount differs from the requested msat.
        guard invoiceMsat == amountMsat else {
            throw LNURLError.invoiceAmountMismatch(expectedMsat: amountMsat, actualMsat: invoiceMsat)
        }

        guard !bolt11.isExpired() else {
            throw LNURLError.errorResponse(reason: "The invoice returned by the LNURL service has expired.")
        }

        switch bolt11.invoiceDescription() {
        case .hash(let hash):
            guard hash.lowercased() == expectedMetadataHashHex.lowercased() else {
                throw LNURLError.errorResponse(reason: "Invoice description hash does not match payee metadata.")
            }
        case .direct:
            throw LNURLError.errorResponse(
                reason: "Invoice uses direct description instead of required description hash (h tag)."
            )
        }

        return invoiceResponse
    }

    private func throwIfErrorResponse(data: Data) throws {
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
