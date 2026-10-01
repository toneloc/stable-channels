import Foundation

// MARK: - Host IP Resolver Protocol

protocol HostIPResolving: Sendable {
    func resolveHostIPs(_ host: String) -> [String]
}

// MARK: - System Host IP Resolver

final class SystemHostIPResolver: HostIPResolving {
    func resolveHostIPs(_ host: String) -> [String] {
        let cleanHost = SecureEndpointValidator.cleanHostString(host)
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
                if !results.contains(ipStr) { results.append(ipStr) }
            }
            ptr = current.pointee.ai_next
        }
        return results
    }
}

// MARK: - Secure Endpoint Validator

enum SecureEndpointValidator {
    /// Validates whether an IPv4 address belongs to private, loopback, multicast, or reserved ranges.
    static func isPrivateIPv4(_ ip: UInt32) -> Bool {
        let top8 = ip >> 24
        if top8 == 0 || top8 == 10 || top8 == 127 { return true }
        if (ip >= 0x6440_0000 && ip <= 0x647F_FFFF) || (ip >> 16) == 0xA9FE { return true } // CGNAT / Link Local
        if (ip >= 0xAC10_0000 && ip <= 0xAC1F_FFFF) || (ip >> 16) == 0xC0A8 { return true } // RFC 1918 172.16 / 192.168
        let top24 = ip >> 8
        if top24 == 0xC00000 || top24 == 0xC00002 || top24 ==
            0xC05863 { return true } // 192.0.0.0/24, 192.0.2.0/24, 192.88.99.0/24
        if (ip >= 0xC612_0000 && ip <= 0xC613_FFFF) || top24 == 0xC63364 || top24 ==
            0xCB0071 { return true } // Benchmark / TEST-NET
        if (top8 & 0xF0) == 0xE0 || (top8 & 0xF0) == 0xF0 { return true } // Multicast / Reserved
        return false
    }

    /// Extracts an unsigned 32-bit big-endian integer from a byte buffer at the given offset.
    private static func extractIPv4(from bytes: UnsafeRawBufferPointer, offset: Int) -> UInt32 {
        (UInt32(bytes[offset]) << 24) | (UInt32(bytes[offset + 1]) << 16) |
            (UInt32(bytes[offset + 2]) << 8) | UInt32(bytes[offset + 3])
    }

    /// Validates whether an IPv6 address belongs to private, loopback, multicast, or reserved ranges.
    static func isPrivateIPv6(_ bytes: UnsafeRawBufferPointer) -> Bool {
        guard bytes.count == 16 else { return false }
        if bytes.allSatisfy({ $0 == 0 }) { return true } // ::/128 Unspecified
        if bytes[0..<15].allSatisfy({ $0 == 0 }) && bytes[15] == 1 { return true } // ::1/128 Loopback
        if bytes[0] == 0xFE &&
            ((bytes[1] & 0xC0) == 0x80 || (bytes[1] & 0xC0) == 0xC0) { return true } // Link / Site-Local
        if (bytes[0] & 0xFE) == 0xFC || bytes[0] == 0xFF { return true } // ULA / Multicast
        if bytes[0] == 0x01 && bytes[1] == 0x00 && bytes[2..<8].allSatisfy({ $0 == 0 }) { return true } // 100::/64
        if bytes[0] == 0x20 && bytes[1] == 0x01 && bytes[2] == 0x0D && bytes[3] == 0xB8 { return true } // 2001:db8::/32
        if bytes[0] == 0x20 && bytes[1] == 0x01 && bytes[2] == 0x00 &&
            ((bytes[3] & 0xF0) == 0x10 || (bytes[3] & 0xF0) == 0x20) {
            return true // 2001:10::/28, 2001:20::/28 ORCHID
        }
        if bytes[0] == 0x00 && bytes[1] == 0x64 && bytes[2] == 0xFF && bytes[3] == 0x9B {
            if bytes[4] == 0x00 && bytes[5] == 0x01 { return true } // 64:ff9b:1::/48 Local-Use
            if bytes[4..<12].allSatisfy({ $0 == 0 }) {
                return isPrivateIPv4(extractIPv4(from: bytes, offset: 12)) // 64:ff9b::/96
            }
        }
        // RFC 2765 SIIT / IPv4-mapped (::ffff:0:0:0/96 and ::ffff:0:0/96)
        if (bytes[0..<8].allSatisfy { $0 == 0 } && bytes[8] == 0xFF && bytes[9] == 0xFF && bytes[10..<12]
            .allSatisfy { $0 == 0 }) ||
            (bytes[0..<10].allSatisfy { $0 == 0 } && bytes[10] == 0xFF && bytes[11] == 0xFF) ||
            bytes[0..<12].allSatisfy({ $0 == 0 }) {
            return isPrivateIPv4(extractIPv4(from: bytes, offset: 12))
        }
        if bytes[0] == 0x3F && bytes[1] == 0xFF && (bytes[2] & 0xF0) == 0x00 { return true } // 3fff::/20
        if bytes[0] == 0x20 && bytes[1] == 0x02 {
            return isPrivateIPv4(extractIPv4(from: bytes, offset: 2)) // 6to4 2002::/16
        }
        if bytes[0] == 0x20 && bytes[1] == 0x01 && bytes[2] == 0x00 && bytes[3] == 0x00 {
            let v4ip = (UInt32(bytes[12] ^ 0xFF) << 24) | (UInt32(bytes[13] ^ 0xFF) << 16) |
                (UInt32(bytes[14] ^ 0xFF) << 8) | UInt32(bytes[15] ^ 0xFF)
            return isPrivateIPv4(v4ip) // Teredo 2001::/32
        }
        return false
    }

    /// Convenience overload for byte arrays.
    static func isPrivateIPv6(_ bytes: [UInt8]) -> Bool {
        bytes.withUnsafeBytes { isPrivateIPv6($0) }
    }

    /// Trims surrounding whitespace, strips trailing dot, and removes IPv6 square brackets.
    static func cleanHostString(_ host: String) -> String {
        var clean = host.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if clean.hasPrefix("[") && clean.hasSuffix("]") {
            clean = String(clean.dropFirst().dropLast())
        }
        if clean.hasSuffix(".") {
            clean.removeLast()
        }
        return clean
    }

    /// Detects ambiguous leading-zero octets in IPv4 dotted quads that cause decimal/octal differentials.
    static func hasLeadingZeroIPv4Octet(_ host: String) -> Bool {
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return false }
        for part in parts {
            guard !part.isEmpty, part.allSatisfy({ $0.isASCII && $0.isNumber }) else { return false }
            if part.count > 1 && part.hasPrefix("0") { return true }
        }
        return false
    }

    /// Formats a validated numeric IP into its canonical string representation.
    static func canonicalNumericIP(_ host: String) -> String? {
        let clean = cleanHostString(host)
        if clean.contains("%") || hasLeadingZeroIPv4Octet(clean) { return nil }
        var addr4 = in_addr()
        if inet_pton(AF_INET, clean, &addr4) == 1 {
            var buf = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
            if inet_ntop(AF_INET, &addr4, &buf, socklen_t(buf.count)) != nil {
                return String(cString: buf)
            }
        }
        var addr6 = in6_addr()
        if inet_pton(AF_INET6, clean, &addr6) == 1 {
            var buf = [CChar](repeating: 0, count: Int(INET6_ADDRSTRLEN))
            if inet_ntop(AF_INET6, &addr6, &buf, socklen_t(buf.count)) != nil {
                return String(cString: buf)
            }
        }
        return nil
    }

    /// Evaluates if a host is an IPv4 or IPv6 numeric literal and whether it belongs to a private/loopback range.
    static func evaluateNumericIP(_ cleanHost: String) -> Bool? {
        if cleanHost.contains("%") || hasLeadingZeroIPv4Octet(cleanHost) {
            return true
        }
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
    static func isSecureEndpoint(url: URL, hostResolver: HostIPResolving = SystemHostIPResolver()) -> Bool {
        guard let scheme = url.scheme?.lowercased(),
              let rawHost = url.host?.lowercased(),
              !rawHost.isEmpty else {
            return false
        }
        guard url.user == nil, url.password == nil else { return false }

        if let rawPort = url.port {
            guard let validPort = UInt16(exactly: rawPort), validPort > 0 else { return false }
        } else if URLComponents(url: url, resolvingAgainstBaseURL: false)?.rangeOfPort != nil {
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
        if evaluateNumericIP(cleanHost) != nil { return false }

        let ips = hostResolver.resolveHostIPs(cleanHost)
        guard !ips.isEmpty, ips.allSatisfy({ evaluateNumericIP($0) == false }) else {
            return false
        }

        return true
    }

    #if DEBUG
        /// Creates an HTTPS GET request preserving the hostname in the URL for native TLS SNI.
        static func createSecureRequest(from url: URL) -> URLRequest {
            var req = URLRequest(url: url)
            req.httpMethod = "GET"
            req.setValue("application/json", forHTTPHeaderField: "Accept")
            return req
        }
    #endif
}
