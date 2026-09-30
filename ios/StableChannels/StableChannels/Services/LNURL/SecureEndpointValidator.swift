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
              SecureEndpointValidator.isSecureEndpoint(url: targetURL, hostResolver: hostResolver) else {
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
        // 0.0.0.0/8 (Current network), 10.0.0.0/8 (RFC 1918), 127.0.0.0/8 (Loopback)
        if top8 == 0 || top8 == 10 || top8 == 127 { return true }
        // 100.64.0.0/10 (Shared Address Space / CGNAT, RFC 6598)
        if ip >= 0x6440_0000 && ip <= 0x647F_FFFF { return true }
        // 169.254.0.0/16 (Link Local, RFC 3927)
        if (ip >> 16) == 0xA9FE { return true }
        // 172.16.0.0/12 (RFC 1918 Private)
        if ip >= 0xAC10_0000 && ip <= 0xAC1F_FFFF { return true }
        // 192.0.0.0/24 (IETF Protocol Assignments, RFC 6890), 192.0.2.0/24 (TEST-NET-1, RFC 5737)
        if (ip >> 8) == 0xC00000 || (ip >> 8) == 0xC00002 { return true }
        // 192.168.0.0/16 (RFC 1918 Private)
        if (ip >> 16) == 0xC0A8 { return true }
        // 198.18.0.0/15 (Benchmarking, RFC 2544)
        if ip >= 0xC612_0000 && ip <= 0xC613_FFFF { return true }
        // 198.51.100.0/24 (TEST-NET-2, RFC 5737)
        if (ip >> 8) == 0xC63364 { return true }
        // 203.0.113.0/24 (TEST-NET-3, RFC 5737)
        if (ip >> 8) == 0xCB0071 { return true }
        // 224.0.0.0/4 (Multicast, RFC 5771)
        if (top8 & 0xF0) == 0xE0 { return true }
        // 240.0.0.0/4 (Reserved / Class E / Broadcast, RFC 1112)
        if (top8 & 0xF0) == 0xF0 { return true }
        return false
    }

    /// Validates whether an IPv6 address belongs to private, loopback, multicast, or reserved ranges.
    static func isPrivateIPv6(_ bytes: UnsafeRawBufferPointer) -> Bool {
        guard bytes.count == 16 else { return false }
        // ::/128 (Unspecified)
        if bytes.allSatisfy({ $0 == 0 }) { return true }
        // ::1/128 (Loopback)
        if bytes[0..<15].allSatisfy({ $0 == 0 }) && bytes[15] == 1 { return true }
        // fe80::/10 (Link-Local unicast)
        if bytes[0] == 0xFE && (bytes[1] & 0xC0) == 0x80 { return true }
        // fc00::/7 (Unique Local unicast ULA)
        if (bytes[0] & 0xFE) == 0xFC { return true }
        // ff00::/8 (Multicast)
        if bytes[0] == 0xFF { return true }
        // 100::/64 (Discard-only prefix, RFC 6666)
        if bytes[0] == 0x01 && bytes[1] == 0x00 && bytes[2..<8].allSatisfy({ $0 == 0 }) { return true }
        // 2001:db8::/32 (Documentation prefix, RFC 3849)
        if bytes[0] == 0x20 && bytes[1] == 0x01 && bytes[2] == 0x0D && bytes[3] == 0xB8 { return true }
        // ::ffff:0:0/96 (IPv4-mapped IPv6)
        if bytes[0..<10].allSatisfy({ $0 == 0 }) && bytes[10] == 0xFF && bytes[11] == 0xFF {
            let v4ip = (UInt32(bytes[12]) << 24) | (UInt32(bytes[13]) << 16) | (UInt32(bytes[14]) << 8) |
                UInt32(bytes[15])
            return isPrivateIPv4(v4ip)
        }
        // ::0:0/96 (IPv4-compatible IPv6, deprecated RFC 4291)
        if bytes[0..<12].allSatisfy({ $0 == 0 }) {
            let v4ip = (UInt32(bytes[12]) << 24) | (UInt32(bytes[13]) << 16) | (UInt32(bytes[14]) << 8) |
                UInt32(bytes[15])
            return isPrivateIPv4(v4ip)
        }
        // 2002::/16 (6to4 prefix, RFC 3056): embedded IPv4 in bytes 2..5
        if bytes[0] == 0x20 && bytes[1] == 0x02 {
            let v4ip = (UInt32(bytes[2]) << 24) | (UInt32(bytes[3]) << 16) | (UInt32(bytes[4]) << 8) | UInt32(bytes[5])
            return isPrivateIPv4(v4ip)
        }
        // 2001::/32 (Teredo prefix, RFC 4380): client IPv4 in bytes 12..15 XOR 0xFF
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

    /// Checks if a given host is a private, loopback, or link-local address to prevent SSRF.
    static func isPrivateOrLoopbackHost(_ host: String) -> Bool {
        var cleanHost = host.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if cleanHost.hasPrefix("[") && cleanHost.hasSuffix("]") {
            cleanHost = String(cleanHost.dropFirst().dropLast())
        }
        if cleanHost == "localhost" || cleanHost.hasSuffix(".localhost") || cleanHost.hasSuffix(".local") || cleanHost
            .hasSuffix(".internal") {
            return true
        }
        var addr4 = in_addr()
        if inet_aton(cleanHost, &addr4) != 0 {
            return isPrivateIPv4(UInt32(bigEndian: addr4.s_addr))
        }
        var addr6 = in6_addr()
        if inet_pton(AF_INET6, cleanHost, &addr6) == 1 {
            return withUnsafeBytes(of: &addr6) { isPrivateIPv6($0) }
        }
        return false
    }

    /// Validates transport security: strict HTTPS for clearnet, HTTP or HTTPS for Tor (.onion) hidden services.
    /// Also rejects any loopback, private, or link-local hosts, as well as DNS names resolving to restricted IPs.
    /// Enforces fail-closed semantics: non-.onion hosts failing DNS resolution are strictly rejected.
    static func isSecureEndpoint(url: URL, hostResolver: HostIPResolving = SystemHostIPResolver()) -> Bool {
        guard let scheme = url.scheme?.lowercased(), let host = url.host?.lowercased(), !host.isEmpty else {
            return false
        }
        guard !isPrivateOrLoopbackHost(host) else {
            return false
        }
        if host.hasSuffix(".onion") {
            return scheme == "http" || scheme == "https"
        }
        guard scheme == "https" else {
            return false
        }
        let ips = hostResolver.resolveHostIPs(host)
        guard !ips.isEmpty else {
            return false
        }
        if ips.contains(where: { isPrivateOrLoopbackHost($0) }) {
            return false
        }
        return true
    }
}
