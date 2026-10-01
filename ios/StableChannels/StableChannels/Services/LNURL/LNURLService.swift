import Foundation
import LDKNode

// MARK: - LNURL Service Implementation

final class LNURLService: LNURLServiceProtocol {
    private let transport: SecureHTTPTransporting
    private let hostResolver: HostIPResolving
    private let expectedNetwork: Network

    init(
        expectedNetwork: Network,
        hostResolver: HostIPResolving = SystemHostIPResolver(),
        onionTransport: SecureHTTPTransporting? = nil,
        transport: SecureHTTPTransporting? = nil
    ) {
        self.expectedNetwork = expectedNetwork
        self.hostResolver = hostResolver
        self.transport = transport ?? NWConnectionTransport(
            hostResolver: hostResolver,
            onionTransport: onionTransport
        )
    }

    #if DEBUG
        convenience init(
            urlSession: URLSession,
            hostResolver: HostIPResolving = SystemHostIPResolver(),
            expectedNetwork: Network = .regtest,
            onionTransport: SecureHTTPTransporting? = nil
        ) {
            self.init(
                expectedNetwork: expectedNetwork,
                hostResolver: hostResolver,
                onionTransport: onionTransport,
                transport: URLSessionTransport(urlSession: urlSession, hostResolver: hostResolver)
            )
        }

        convenience init(
            transport: SecureHTTPTransporting,
            hostResolver: HostIPResolving = SystemHostIPResolver(),
            expectedNetwork: Network = .regtest
        ) {
            self.init(
                expectedNetwork: expectedNetwork,
                hostResolver: hostResolver,
                onionTransport: nil,
                transport: transport
            )
        }
    #endif

    /// Checks if a given host is a private, loopback, or link-local address to prevent SSRF.
    static func isPrivateOrLoopbackHost(_ host: String) -> Bool {
        SecureEndpointValidator.isPrivateOrLoopbackHost(host)
    }

    /// Validates transport security: strict HTTPS for clearnet, HTTP or HTTPS for Tor (.onion) hidden services.
    static func isSecureEndpoint(url: URL, hostResolver: HostIPResolving = SystemHostIPResolver()) -> Bool {
        SecureEndpointValidator.isSecureEndpoint(url: url, hostResolver: hostResolver)
    }

    /// Resolves an input destination into an actionable HTTPS LNURL endpoint URL.
    static func resolveEndpoint(
        from input: String,
        hostResolver: HostIPResolving = SystemHostIPResolver()
    ) throws -> URL {
        var clean = input.trimmingCharacters(in: .whitespacesAndNewlines)
        for prefix in ["lightning://", "lightning:"] where clean.lowercased().hasPrefix(prefix) {
            clean = String(clean.dropFirst(prefix.count)).trimmingCharacters(in: .whitespacesAndNewlines)
            break
        }

        let resolvedURL: URL
        if clean.contains("@") {
            let parts = clean.split(separator: "@", omittingEmptySubsequences: true)
            guard clean.filter({ $0 == "@" }).count == 1, parts.count == 2 else {
                throw LNURLError.invalidTarget
            }
            let username = String(parts[0]).trimmingCharacters(in: .whitespacesAndNewlines)
            let domain = String(parts[1]).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            guard !username.isEmpty, !domain.isEmpty else {
                throw LNURLError.invalidTarget
            }

            let forbidden = CharacterSet(charactersIn: "/?#@: \\\"%<>{}|^`[]")
            let allowedDomain = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyz0123456789.-")
            guard username.rangeOfCharacter(from: forbidden) == nil,
                  username.rangeOfCharacter(from: .controlCharacters) == nil,
                  domain.contains("."), !domain.hasPrefix("."), !domain.hasSuffix("."),
                  domain.unicodeScalars.allSatisfy({ allowedDomain.contains($0) }),
                  !isPrivateOrLoopbackHost(domain) else {
                throw LNURLError.invalidTarget
            }

            let scheme = domain.hasSuffix(".onion") ? "http" : "https"
            guard let url = URL(string: "\(scheme)://\(domain)/.well-known/lnurlp/\(username)") else {
                throw LNURLError.invalidTarget
            }
            resolvedURL = url
        } else if clean.lowercased().hasPrefix("lnurl1") {
            resolvedURL = try Bech32.decodeLNURL(clean)
        } else if let directURL = URL(string: clean) {
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
        try await transport.executeGet(url: url)
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
            throw LNURLError.invalidComment
        }
        return try await fetchInvoiceRaw(
            callback: params.callback,
            amountMsat: amountMsat,
            comment: comment,
            expectedMetadataHashHex: params.metadataHashHex
        )
    }

    /// Pure function for assembling the invoice callback URL with escaped query parameters.
    static func buildInvoiceCallbackURL(
        callback: String,
        amountMsat: UInt64,
        comment: String?
    ) throws -> URL {
        guard let initialUrl = URL(string: callback),
              var components = URLComponents(url: initialUrl, resolvingAgainstBaseURL: false),
              initialUrl.scheme != nil,
              initialUrl.host != nil else {
            throw LNURLError.invalidTarget
        }

        var queryItems = components.percentEncodedQueryItems ?? []
        queryItems.append(URLQueryItem(name: "amount", value: "\(amountMsat)"))
        if let comment {
            let trimmed = comment.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty {
                var allowed = CharacterSet.urlQueryAllowed
                allowed.remove(charactersIn: "+&?#/=;")
                let encoded = trimmed.addingPercentEncoding(withAllowedCharacters: allowed) ?? trimmed
                queryItems.append(URLQueryItem(name: "comment", value: encoded))
            }
        }
        components.percentEncodedQueryItems = queryItems
        guard let url = components.url else {
            throw LNURLError.invalidResponse
        }
        return url
    }

    /// Pure domain validation function verifying BOLT11 invoice parameters against request expectations.
    static func validateBolt11Invoice(
        pr: String,
        amountMsat: UInt64,
        expectedNetwork: Network,
        expectedMetadataHashHex: String
    ) throws {
        guard !pr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw LNURLError.invalidResponse
        }

        guard let bolt11 = try? Bolt11Invoice.fromStr(invoiceStr: pr) else {
            throw LNURLError.invalidResponse
        }

        let currency = bolt11.currency()
        let matches = (expectedNetwork == .bitcoin && currency == .bitcoin) ||
            (expectedNetwork == .testnet && currency == .bitcoinTestnet) ||
            (expectedNetwork == .signet && currency == .signet) ||
            (expectedNetwork == .regtest && currency == .regtest)
        guard matches else {
            throw LNURLError.errorResponse(reason: "Invoice currency does not match wallet network.")
        }

        guard let invoiceMsat = bolt11.amountMilliSatoshis() else {
            throw LNURLError.errorResponse(reason: "Amountless invoices are not permitted for LNURL pay.")
        }

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
    }

    private func fetchInvoiceRaw(
        callback: String,
        amountMsat: UInt64,
        comment: String?,
        expectedMetadataHashHex: String
    ) async throws -> LNURLPayInvoiceResponse {
        let url = try Self.buildInvoiceCallbackURL(callback: callback, amountMsat: amountMsat, comment: comment)
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

        try Self.validateBolt11Invoice(
            pr: invoiceResponse.pr,
            amountMsat: amountMsat,
            expectedNetwork: expectedNetwork,
            expectedMetadataHashHex: expectedMetadataHashHex
        )

        return invoiceResponse
    }

    private func throwIfErrorResponse(data: Data) throws {
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return
        }

        if let status = json["status"] as? String, status.uppercased() == "ERROR" {
            let reason = (json["reason"] as? String) ?? "The recipient service reported an error."
            throw LNURLError.errorResponse(reason: reason)
        }

        if let reason = json["reason"] as? String, !reason.isEmpty, json["tag"] == nil, json["pr"] == nil {
            throw LNURLError.errorResponse(reason: reason)
        }
    }
}
