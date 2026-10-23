package com.stablechannels.app.domain.send

import com.stablechannels.app.domain.send.DestinationClassification.Empty
import com.stablechannels.app.domain.send.DestinationClassification.Invalid
import com.stablechannels.app.domain.send.DestinationClassification.Valid
import com.stablechannels.app.domain.send.DestinationRejection as Reason
import java.net.URI

/**
 * Classifies user input as a BOLT11 invoice, BOLT12 offer, Lightning Address, LNURL-pay or on-chain
 * address. With a network, destinations for any other network are rejected.
 */
class PaymentDestinationClassifier(private val decoder: LightningInvoiceDecoder) {
    private companion object {
        val LIGHTNING_ADDRESS = Regex("^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\\.[a-zA-Z0-9-.]+$")
        val BOLT11_PREFIXES = listOf("lnbcrt", "lnbc", "lntbs", "lntb")
    }

    fun classify(input: String, network: BitcoinNetwork? = null): DestinationClassification {
        val trimmed = input.trim()
        if (trimmed.isEmpty()) return Empty
        var normalized = Bech32.stripLightningScheme(trimmed)

        if (normalized.startsWithAscii("bitcoin:")) {
            classifyBip21(normalized, network)?.let {
                return it
            }
            normalized = normalized.substring("bitcoin:".length)
        }

        // Checked before the "ln" prefixes: a handle such as lnbits@example.com starts with "ln".
        if ('@' in normalized && ' ' !in normalized) {
            classifyLightningAddress(normalized)?.let {
                return it
            }
        }

        val lower = normalized.lowercase()
        return when {
            lower.startsWith("ln") -> classifyLightning(normalized, lower, network)
            OnchainAddress.looksLikeAddress(lower) -> classifyOnchain(normalized, null, network)
            else -> Invalid(Reason.UNRECOGNIZED)
        }
    }

    private fun classifyLightning(
        raw: String,
        lower: String,
        network: BitcoinNetwork?,
    ): DestinationClassification =
        when {
            BOLT11_PREFIXES.any { lower.startsWith(it) } -> {
                val invoice = decoder.decodeBolt11(raw)
                when {
                    invoice == null -> Invalid(Reason.INVALID_BOLT11)
                    network != null && invoice.network != network -> Invalid(Reason.WRONG_NETWORK)
                    else -> Valid(SendDestination.Bolt11(raw, invoice.amountMsat))
                }
            }
            lower.startsWith("lno") -> {
                val offer = decoder.decodeBolt12(raw)
                val offerNetworks = offer?.chains?.ifEmpty { setOf(BitcoinNetwork.BITCOIN) }
                when {
                    offerNetworks == null -> Invalid(Reason.INVALID_BOLT12)
                    network != null && network !in offerNetworks -> Invalid(Reason.WRONG_NETWORK)
                    else -> Valid(SendDestination.Bolt12(raw))
                }
            }
            lower.startsWith("lnurl1") ->
                try {
                    Valid(SendDestination.LnurlPay(Bech32.decodeLnurl(raw)))
                } catch (_: Bech32.DecodeException) {
                    Invalid(Reason.INVALID_LNURL)
                }
            else -> Invalid(Reason.UNSUPPORTED_LIGHTNING_PREFIX)
        }

    private fun classifyOnchain(
        address: String,
        amountSats: Long?,
        network: BitcoinNetwork?,
    ): DestinationClassification =
        when {
            OnchainAddress.isValid(address, network) ->
                Valid(SendDestination.Onchain(OnchainAddress.normalize(address), amountSats))
            network != null && OnchainAddress.isValid(address) -> Invalid(Reason.WRONG_NETWORK)
            else -> Invalid(Reason.INVALID_ONCHAIN_ADDRESS)
        }

    private fun classifyLightningAddress(input: String): DestinationClassification? {
        val handle = input.substringBefore('@')
        val domain = input.substringAfter('@')
        if (handle.isEmpty() || domain.isEmpty()) return null
        if (!LIGHTNING_ADDRESS.matches(input)) return Invalid(Reason.INVALID_LIGHTNING_ADDRESS)
        val url =
            runCatching { URI("https://$domain/.well-known/lnurlp/$handle") }.getOrNull()
                ?: return Invalid(Reason.INVALID_LIGHTNING_ADDRESS)
        return Valid(SendDestination.LightningAddress(handle, domain, url))
    }

    /**
     * A valid `lightning` invoice or offer wins over the on-chain address, and an amountless
     * invoice takes the URI amount. Returns null when nothing in the URI is usable, so the caller
     * falls back to classifying the bare address.
     */
    private fun classifyBip21(uri: String, network: BitcoinNetwork?): DestinationClassification? {
        val parsed = Bip21Parser.parse(uri) ?: return null
        parsed.unhandledRequiredParameter?.let {
            return Invalid(Reason.UNHANDLED_REQUIRED_BIP21_PARAMETER)
        }
        if (parsed.hasDuplicateParameter) return Invalid(Reason.DUPLICATE_BIP21_PARAMETER)

        val lightning = parsed.lightning?.let { classify(it, network) as? Valid }?.destination
        when (lightning) {
            is SendDestination.Bolt11 -> {
                val amountMsat = lightning.amountMsat ?: parsed.amountSats?.let { it * 1000 }
                return Valid(lightning.copy(amountMsat = amountMsat))
            }
            is SendDestination.Bolt12 -> return Valid(lightning)
            else -> Unit
        }

        val offer = parsed.offer?.let { classify(it, network) as? Valid }?.destination
        if (offer is SendDestination.Bolt12) return Valid(offer)

        if (parsed.address.isEmpty()) return null
        return classifyOnchain(parsed.address, parsed.amountSats, network).takeUnless {
            it is Invalid && it.reason == Reason.INVALID_ONCHAIN_ADDRESS
        }
    }
}
