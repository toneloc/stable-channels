package com.stablechannels.app.domain.lnurl

/**
 * Decides whether a host or IP address may be contacted for LNURL. Loopback, private, link-local,
 * CGNAT, documentation, benchmarking, multicast and reserved ranges are refused, including when
 * they are wrapped in IPv6 (mapped, NAT64, 6to4, Teredo), so a payee cannot point the wallet at the
 * user's own network.
 */
object HostAddressPolicy {
    private val LOCAL_SUFFIXES = listOf(".localhost", ".local", ".internal")

    /** Lowercased, trimmed, without a trailing dot or IPv6 brackets. */
    fun cleanHost(host: String): String {
        var clean = host.trim().lowercase()
        if (clean.startsWith("[") && clean.endsWith("]"))
            clean = clean.substring(1, clean.length - 1)
        return clean.removeSuffix(".")
    }

    fun isLocalName(cleanHost: String): Boolean =
        cleanHost == "localhost" || LOCAL_SUFFIXES.any { cleanHost.endsWith(it) }

    /**
     * True for a host that must not be contacted: a local name, a blocked IP literal, or an IP
     * literal that cannot be read unambiguously (zone ids, octets with leading zeros).
     */
    fun isPrivateOrLoopbackHost(host: String): Boolean {
        val clean = cleanHost(host)
        return isLocalName(clean) || isBlockedIpLiteral(clean) == true
    }

    /**
     * For an IP literal: true when blocked, false when public. Null when [cleanHost] is not an IP
     * literal. Ambiguous forms (a `%` zone id, an octet such as `010`) count as blocked.
     */
    fun isBlockedIpLiteral(cleanHost: String): Boolean? {
        if ('%' in cleanHost || hasLeadingZeroIpv4Octet(cleanHost)) return true
        parseIpv4(cleanHost)?.let {
            return isPrivateIpv4(it)
        }
        parseIpv6(cleanHost)?.let {
            return isPrivateIpv6(it)
        }
        return null
    }

    /** True when [address] (4 or 16 bytes, as resolved by DNS) is in a blocked range. */
    fun isBlockedAddress(address: ByteArray): Boolean =
        when (address.size) {
            4 -> isPrivateIpv4(ipv4At(address.map { it.toInt() and 0xFF }, 0))
            16 -> isPrivateIpv6(address.map { it.toInt() and 0xFF })
            else -> true
        }

    /** A dotted quad whose octets have leading zeros, which some parsers read as octal. */
    fun hasLeadingZeroIpv4Octet(host: String): Boolean {
        val parts = host.split('.')
        if (parts.size != 4 || parts.any { p -> p.isEmpty() || !p.all { it in '0'..'9' } })
            return false
        return parts.any { it.length > 1 && it.startsWith("0") }
    }

    /** Strict dotted-quad parser: exactly four decimal octets, no short or hex forms. */
    fun parseIpv4(text: String): Long? {
        val parts = text.split('.')
        if (parts.size != 4) return null
        var value = 0L
        for (part in parts) {
            if (part.isEmpty() || part.length > 3 || !part.all { it in '0'..'9' }) return null
            val octet = part.toInt()
            if (octet > 255) return null
            value = (value shl 8) or octet.toLong()
        }
        return value
    }

    /** RFC 4291 text form, including `::` compression and a trailing dotted quad. */
    fun parseIpv6(text: String): List<Int>? {
        if (':' !in text || text.count { it == ':' } > 8) return null
        val doubleColon = text.indexOf("::")
        if (doubleColon >= 0 && text.indexOf("::", doubleColon + 1) >= 0) return null

        fun groups(part: String): List<Int>? {
            if (part.isEmpty()) return emptyList()
            val out = mutableListOf<Int>()
            val pieces = part.split(':')
            pieces.forEachIndexed { i, piece ->
                if (i == pieces.lastIndex && '.' in piece) {
                    val v4 = parseIpv4(piece) ?: return null
                    out += ((v4 shr 16) and 0xFFFF).toInt()
                    out += (v4 and 0xFFFF).toInt()
                } else {
                    if (piece.isEmpty() || piece.length > 4) return null
                    out += piece.toIntOrNull(16) ?: return null
                }
            }
            return out
        }

        val words =
            if (doubleColon >= 0) {
                val head = groups(text.substring(0, doubleColon)) ?: return null
                val tail = groups(text.substring(doubleColon + 2)) ?: return null
                if (head.size + tail.size > 7) return null
                head + List(8 - head.size - tail.size) { 0 } + tail
            } else {
                groups(text)?.takeIf { it.size == 8 } ?: return null
            }
        return words.flatMap { listOf(it shr 8, it and 0xFF) }
    }

    fun isPrivateIpv4(ip: Long): Boolean {
        val top8 = ip shr 24
        val top16 = ip shr 16
        val top24 = ip shr 8
        return top8 == 0L ||
            top8 == 10L ||
            top8 == 127L ||
            ip in 0x64400000L..0x647FFFFFL || // 100.64.0.0/10 CGNAT
            top16 == 0xA9FEL || // 169.254.0.0/16 link-local
            ip in 0xAC100000L..0xAC1FFFFFL || // 172.16.0.0/12
            top16 == 0xC0A8L || // 192.168.0.0/16
            top24 == 0xC00000L || // 192.0.0.0/24
            top24 == 0xC00002L || // 192.0.2.0/24 TEST-NET-1
            top24 == 0xC05863L || // 192.88.99.0/24 6to4 relay
            ip in 0xC6120000L..0xC613FFFFL || // 198.18.0.0/15 benchmarking
            top24 == 0xC63364L || // 198.51.100.0/24 TEST-NET-2
            top24 == 0xCB0071L || // 203.0.113.0/24 TEST-NET-3
            top8 >= 0xE0L // 224.0.0.0/4 multicast and 240.0.0.0/4 reserved
    }

    /** [b] holds 16 unsigned bytes. */
    fun isPrivateIpv6(b: List<Int>): Boolean {
        if (b.size != 16) return false
        fun zero(range: IntRange) = range.all { b[it] == 0 }
        return when {
            zero(0..15) -> true // ::
            zero(0..14) && b[15] == 1 -> true // ::1
            b[0] == 0xFE && (b[1] and 0xC0) in setOf(0x80, 0xC0) -> true // link-local, site-local
            (b[0] and 0xFE) == 0xFC || b[0] == 0xFF -> true // ULA, multicast
            b[0] == 0x01 && b[1] == 0x00 && zero(2..7) -> true // 100::/64 discard
            b[0] == 0x20 && b[1] == 0x01 && b[2] == 0x0D && b[3] == 0xB8 -> true // 2001:db8::/32
            b[0] == 0x20 && b[1] == 0x01 && b[2] == 0x00 && (b[3] and 0xF0) in setOf(0x10, 0x20) ->
                true // ORCHID
            b[0] == 0x00 &&
                b[1] == 0x64 &&
                b[2] == 0xFF &&
                b[3] == 0x9B &&
                b[4] == 0 &&
                b[5] == 1 -> true // 64:ff9b:1::/48 local-use NAT64
            b[0] == 0x00 && b[1] == 0x64 && b[2] == 0xFF && b[3] == 0x9B && zero(4..11) ->
                isPrivateIpv4(ipv4At(b, 12)) // NAT64
            (zero(0..7) && b[8] == 0xFF && b[9] == 0xFF && zero(10..11)) ||
                (zero(0..9) && b[10] == 0xFF && b[11] == 0xFF) ||
                zero(0..11) -> isPrivateIpv4(ipv4At(b, 12)) // mapped, SIIT, compatible
            b[0] == 0x3F && b[1] == 0xFF && (b[2] and 0xF0) == 0 -> true // 3fff::/20 documentation
            b[0] == 0x20 && b[1] == 0x02 -> isPrivateIpv4(ipv4At(b, 2)) // 6to4
            b[0] == 0x20 && b[1] == 0x01 && b[2] == 0 && b[3] == 0 ->
                isPrivateIpv4(ipv4At(b.map { it xor 0xFF }, 12)) // Teredo client
            else -> false
        }
    }

    private fun ipv4At(b: List<Int>, offset: Int): Long =
        (b[offset].toLong() shl 24) or
            (b[offset + 1].toLong() shl 16) or
            (b[offset + 2].toLong() shl 8) or
            b[offset + 3].toLong()
}
