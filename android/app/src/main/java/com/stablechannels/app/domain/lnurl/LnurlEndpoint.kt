package com.stablechannels.app.domain.lnurl

import com.stablechannels.app.domain.send.Bech32
import com.stablechannels.app.domain.send.startsWithAscii
import java.net.URI

/**
 * Turns a Lightning Address (LUD-16), an LNURL (LUD-01) or a URL into the endpoint to request, and
 * decides which endpoints may be contacted. These checks need no network; the infrastructure also
 * checks every address a hostname resolves to when it connects.
 */
object LnurlEndpoint {
    const val MAX_REDIRECTS = 3

    private const val FORBIDDEN_HANDLE_CHARS = "/?#@: \\\"%<>{}|^`[]"
    private val DOMAIN_CHARS = ('a'..'z').toSet() + ('0'..'9') + '.' + '-'

    /** The endpoint for [input], or [LnurlError.InvalidTarget] when it cannot be used. */
    fun resolve(input: String): URI {
        var clean = input.trim()
        for (prefix in listOf("lightning://", "lightning:")) {
            if (clean.startsWithAscii(prefix)) {
                clean = clean.substring(prefix.length).trim()
                break
            }
        }
        val url =
            when {
                '@' in clean -> lightningAddressUrl(clean)
                clean.startsWithAscii("lnurl1") ->
                    try {
                        Bech32.decodeLnurl(clean)
                    } catch (_: Bech32.DecodeException) {
                        throw LnurlError.InvalidTarget
                    }
                else -> runCatching { URI(clean) }.getOrNull() ?: throw LnurlError.InvalidTarget
            }
        if (!isAllowed(url)) throw LnurlError.InvalidTarget
        return url
    }

    private fun lightningAddressUrl(address: String): URI {
        if (address.count { it == '@' } != 1) throw LnurlError.InvalidTarget
        val handle = address.substringBefore('@').trim()
        val domain = address.substringAfter('@').trim().lowercase()
        val validHandle =
            handle.isNotEmpty() &&
                handle.none {
                    it in FORBIDDEN_HANDLE_CHARS ||
                        it.isISOControl() ||
                        Character.getType(it) == Character.FORMAT.toInt()
                }
        val validDomain =
            '.' in domain &&
                !domain.startsWith(".") &&
                !domain.endsWith(".") &&
                domain.all { it in DOMAIN_CHARS } &&
                !HostAddressPolicy.isPrivateOrLoopbackHost(domain)
        if (!validHandle || !validDomain) throw LnurlError.InvalidTarget
        val scheme = if (domain.endsWith(".onion")) "http" else "https"
        return runCatching { URI("$scheme://$domain/.well-known/lnurlp/$handle") }.getOrNull()
            ?: throw LnurlError.InvalidTarget
    }

    /**
     * True when [url] may be requested: HTTPS for clearnet (HTTP or HTTPS for `.onion`), a plain
     * ASCII host name rather than an IP literal, no credentials, a valid port, and no local names.
     *
     * Anything a client could read as an IP address is refused, including short and single-number
     * IPv4 forms (`127.1`, `2130706433`) and any host with non-ASCII characters, which IDN mapping
     * can turn into digits and dots (`１２７.０.０.１`). Such hosts would be connected to directly,
     * without the resolved-address check.
     */
    fun isAllowed(url: URI): Boolean {
        val scheme = url.scheme?.lowercase() ?: return false
        val host = hostOf(url) ?: return false
        // rawUserInfo is null for hosts java.net.URI cannot parse, so check the authority too.
        if (url.rawUserInfo != null || url.rawAuthority?.contains('@') == true) return false
        if (url.port != -1 && url.port !in 1..65535) return false
        if (url.port == -1 && url.rawAuthority?.substringAfterLast(']')?.contains(':') == true)
            return false

        val clean = HostAddressPolicy.cleanHost(host)
        if (
            clean.any { it.code >= 0x80 } ||
                clean.all { it in '0'..'9' || it == '.' } ||
                ':' in clean
        ) {
            return false
        }
        if (HostAddressPolicy.isLocalName(clean)) return false
        if (clean.endsWith(".onion")) return scheme == "http" || scheme == "https"
        return scheme == "https" && HostAddressPolicy.isBlockedIpLiteral(clean) == null
    }

    fun isOnion(url: URI): Boolean =
        HostAddressPolicy.cleanHost(hostOf(url).orEmpty()).endsWith(".onion")

    /** The host, also for names `java.net.URI` leaves unparsed (for example with `_`). */
    fun hostOf(url: URI): String? =
        (url.host ?: url.rawAuthority?.substringAfterLast('@')?.substringBefore(':'))?.takeIf {
            it.isNotEmpty()
        }

    /**
     * The URL a redirect from [current] leads to. Refuses after [MAX_REDIRECTS] hops, a missing or
     * unparseable `Location`, a jump from clearnet to `.onion`, and any target [isAllowed] refuses.
     */
    fun redirectTarget(
        current: URI,
        location: String?,
        hop: Int,
        allow: (URI) -> Boolean = ::isAllowed,
    ): URI {
        if (hop >= MAX_REDIRECTS) throw LnurlError.TooManyRedirects
        val target =
            location?.let { runCatching { current.resolve(it.trim()) }.getOrNull() }
                ?: throw LnurlError.InvalidResponse
        if (!isOnion(current) && isOnion(target)) throw LnurlError.InsecureEndpoint
        if (!allow(target)) throw LnurlError.InsecureEndpoint
        return target
    }
}
