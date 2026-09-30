import Foundation

// MARK: - Host IP Resolver Protocol

protocol HostIPResolving: Sendable {
    func resolveHostIPs(_ host: String) -> [String]
}

// MARK: - System Host IP Resolver

final class SystemHostIPResolver: HostIPResolving {
    func resolveHostIPs(_ host: String) -> [String] {
        var cleanHost = host.trimmingCharacters(in: .whitespacesAndNewlines)
        if cleanHost.hasPrefix("[") && cleanHost.hasSuffix("]") {
            cleanHost = String(cleanHost.dropFirst().dropLast())
        }
        guard !cleanHost.isEmpty, !cleanHost.hasSuffix(".onion") else { return [] }

        var hints = addrinfo()
        hints.ai_flags = 0
        hints.ai_family = AF_UNSPEC
        hints.ai_socktype = SOCK_STREAM

        var res: UnsafeMutablePointer<addrinfo>?
        let status = getaddrinfo(cleanHost, nil, &hints, &res)
        guard status == 0, let first = res else { return [] }
        defer { freeaddrinfo(res) }

        var results: [String] = []
        var ptr: UnsafeMutablePointer<addrinfo>? = first
        while let current = ptr {
            var hostBuffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(
                current.pointee.ai_addr,
                current.pointee.ai_addrlen,
                &hostBuffer,
                socklen_t(hostBuffer.count),
                nil,
                0,
                NI_NUMERICHOST
            ) == 0 {
                let ipStr = String(cString: hostBuffer)
                if !results.contains(ipStr) {
                    results.append(ipStr)
                }
            }
            ptr = current.pointee.ai_next
        }
        return results
    }
}

// MARK: - Secure Redirect Delegate

final class SecureRedirectDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var _encounteredInsecureRedirect = false
    private let hostResolver: HostIPResolving

    var encounteredInsecureRedirect: Bool {
        lock.withLock { _encounteredInsecureRedirect }
    }

    init(hostResolver: HostIPResolving = SystemHostIPResolver()) {
        self.hostResolver = hostResolver
    }

    func urlSession(
        _: URLSession,
        task _: URLSessionTask,
        willPerformHTTPRedirection _: HTTPURLResponse,
        newRequest: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        guard let targetURL = newRequest.url,
              SecureEndpointValidator.isSecureEndpoint(url: targetURL, hostResolver: hostResolver)
        else {
            lock.withLock { _encounteredInsecureRedirect = true }
            completionHandler(nil)
            return
        }
        completionHandler(newRequest)
    }
}

// MARK: - Secure Endpoint Validator

enum SecureEndpointValidator {
    /// Validates whether an IPv4 address belongs to private, loopback, multicast, or reserved ranges.
    static func isPrivateIPv4(_ ip: UInt32) -> Bool {
        let top8 = ip >> 24
        if top8 == 0 || top8 == 10 || top8 == 127 { return true }
        if ip >= 0x6440_0000 && ip <= 0x647F_FFFF { return true } // CGNAT 100.64.0.0/10
        if (ip >> 16) == 0xA9FE { return true } // Link Local 169.254.0.0/16
        if ip >= 0xAC10_0000 && ip <= 0xAC1F_FFFF { return true } // RFC 1918 172.16.0.0/12
        if (ip >> 8) == 0xC00000 || (ip >> 8) == 0xC00002 || (ip >> 8) ==
            0xC05863 { return true } // 192.0.0.0/24, 192.0.2.0/24, 192.88.99.0/24 (RFC 3068/7526)
        if (ip >> 16) == 0xC0A8 { return true } // RFC 1918 192.168.0.0/16
        if ip >= 0xC612_0000 && ip <= 0xC613_FFFF { return true } // Benchmark 198.18.0.0/15
        if (ip >> 8) == 0xC63364 || (ip >> 8) == 0xCB0071 { return true } // TEST-NET-2/3
        if (top8 & 0xF0) == 0xE0 || (top8 & 0xF0) == 0xF0 { return true } // Multicast / Reserved
        return false
    }

    /// Validates whether an IPv6 address belongs to private, loopback, multicast, or reserved ranges.
    static func isPrivateIPv6(_ bytes: UnsafeRawBufferPointer) -> Bool {
        guard bytes.count == 16 else { return false }
        if bytes.allSatisfy({ $0 == 0 }) { return true } // ::/128 Unspecified
        if bytes[0..<15].allSatisfy({ $0 == 0 }) && bytes[15] == 1 { return true } // ::1/128 Loopback
        if bytes[0] == 0xFE && (bytes[1] & 0xC0) == 0x80 { return true } // fe80::/10 Link-Local
        if bytes[0] == 0xFE && (bytes[1] & 0xC0) == 0xC0 { return true } // fec0::/10 Site-Local (RFC 3879)
        if (bytes[0] & 0xFE) == 0xFC { return true } // fc00::/7 ULA
        if bytes[0] == 0xFF { return true } // ff00::/8 Multicast
        if bytes[0] == 0x01 && bytes[1] == 0x00 && bytes[2..<8].allSatisfy({ $0 == 0 }) { return true } // 100::/64
        if bytes[0] == 0x20 && bytes[1] == 0x01 && bytes[2] == 0x0D && bytes[3] == 0xB8 { return true } // 2001:db8::/32
        if bytes[0] == 0x20 && bytes[1] == 0x01 && bytes[2] == 0x00 &&
            ((bytes[3] & 0xF0) == 0x10 || (bytes[3] & 0xF0) == 0x20) {
            return true // 2001:10::/28, 2001:20::/28 ORCHID / ORCHIDv2 (RFC 4843, RFC 7343)
        }
        if bytes[0] == 0x00 && bytes[1] == 0x64 && bytes[2] == 0xFF && bytes[3] == 0x9B {
            if bytes[4] == 0x00 && bytes[5] == 0x01 { return true } // 64:ff9b:1::/48 Local-Use (RFC 8215)
            if bytes[4..<12].allSatisfy({ $0 == 0 }) {
                let v4ip = (UInt32(bytes[12]) << 24) | (UInt32(bytes[13]) << 16) | (UInt32(bytes[14]) << 8) |
                    UInt32(bytes[15])
                return isPrivateIPv4(v4ip) // 64:ff9b::/96 Well-Known Prefix (RFC 6052)
            }
        }
        // RFC 2765 SIIT IPv4-translated (::ffff:0:0/96)
        if bytes[0..<8].allSatisfy({ $0 == 0 }) && bytes[8] == 0xFF && bytes[9] == 0xFF &&
            bytes[10..<12].allSatisfy({ $0 == 0 }) {
            let v4ip = (UInt32(bytes[12]) << 24) | (UInt32(bytes[13]) << 16) | (UInt32(bytes[14]) << 8) |
                UInt32(bytes[15])
            return isPrivateIPv4(v4ip)
        }
        if (bytes[0..<10].allSatisfy { $0 == 0 } && bytes[10] == 0xFF && bytes[11] == 0xFF) ||
            bytes[0..<12].allSatisfy({ $0 == 0 }) {
            let v4ip = (UInt32(bytes[12]) << 24) | (UInt32(bytes[13]) << 16) | (UInt32(bytes[14]) << 8) |
                UInt32(bytes[15])
            return isPrivateIPv4(v4ip)
        }
        if bytes[0] == 0x20 && bytes[1] == 0x02 {
            let v4ip = (UInt32(bytes[2]) << 24) | (UInt32(bytes[3]) << 16) | (UInt32(bytes[4]) << 8) | UInt32(bytes[5])
            return isPrivateIPv4(v4ip)
        }
        if bytes[0] == 0x20 && bytes[1] == 0x01 && bytes[2] == 0x00 && bytes[3] == 0x00 {
            let v4ip = (UInt32(bytes[12] ^ 0xFF) << 24) | (UInt32(bytes[13] ^ 0xFF) << 16) |
                (UInt32(bytes[14] ^ 0xFF) << 8) | UInt32(bytes[15] ^ 0xFF)
            return isPrivateIPv4(v4ip)
        }
        return false
    }

    /// Convenience overload for byte arrays.
    static func isPrivateIPv6(_ bytes: [UInt8]) -> Bool {
        bytes.withUnsafeBytes { isPrivateIPv6($0) }
    }

    /// Trims surrounding whitespace and removes IPv6 square brackets.
    static func cleanHostString(_ host: String) -> String {
        var clean = host.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if clean.hasPrefix("[") && clean.hasSuffix("]") {
            clean = String(clean.dropFirst().dropLast())
        }
        return clean
    }

    /// Evaluates if a host is an IPv4 or IPv6 numeric literal and whether it belongs to a private/loopback range.
    /// Returns true if private/loopback, false if public numeric IP, and nil if domain name.
    static func evaluateNumericIP(_ cleanHost: String) -> Bool? {
        var addr4 = in_addr()
        if inet_pton(AF_INET, cleanHost, &addr4) == 1 {
            return isPrivateIPv4(UInt32(bigEndian: addr4.s_addr))
        }
        var addr6 = in6_addr()
        if inet_pton(AF_INET6, cleanHost, &addr6) == 1 {
            return withUnsafeBytes(of: &addr6) { isPrivateIPv6($0) }
        }
        return nil
    }

    /// Checks if a string represents an IPv4 or IPv6 numeric literal.
    static func isNumericIP(_ host: String) -> Bool {
        evaluateNumericIP(cleanHostString(host)) != nil
    }

    /// Checks if a given host is a private, loopback, or link-local address to prevent SSRF.
    static func isPrivateOrLoopbackHost(_ host: String) -> Bool {
        let clean = cleanHostString(host)
        if clean == "localhost" || clean.hasSuffix(".localhost") || clean.hasSuffix(".local") || clean
            .hasSuffix(".internal") {
            return true
        }
        return evaluateNumericIP(clean) ?? false
    }

    /// Validates transport security: strict HTTPS for clearnet, HTTP or HTTPS for Tor (.onion) hidden services.
    /// Also rejects any loopback, private, or link-local hosts, as well as DNS names resolving to restricted IPs.
    /// Enforces fail-closed semantics: non-.onion hosts failing DNS resolution are strictly rejected.
    static func isSecureEndpoint(url: URL, hostResolver: HostIPResolving = SystemHostIPResolver()) -> Bool {
        guard let scheme = url.scheme?.lowercased(),
              let rawHost = url.host?.lowercased(),
              !rawHost.isEmpty else {
            return false
        }

        // Reject embedded userinfo
        guard url.user == nil, url.password == nil else {
            return false
        }

        let cleanHost = cleanHostString(rawHost)
        guard cleanHost != "localhost",
              !cleanHost.hasSuffix(".localhost"),
              !cleanHost.hasSuffix(".local"),
              !cleanHost.hasSuffix(".internal") else {
            return false
        }

        if cleanHost.hasSuffix(".onion") {
            return scheme == "http" || scheme == "https"
        }

        guard scheme == "https" else { return false }

        if let isPrivate = evaluateNumericIP(cleanHost) {
            return !isPrivate
        }

        let ips = hostResolver.resolveHostIPs(cleanHost)
        guard !ips.isEmpty, !ips.contains(where: { isPrivateOrLoopbackHost($0) }) else {
            return false
        }

        return true
    }

    /// Creates an HTTPS GET request preserving the hostname in the URL for native TLS SNI.
    static func createSecureRequest(from url: URL) -> URLRequest {
        var req = URLRequest(url: url)
        req.httpMethod = "GET"
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        return req
    }
}
