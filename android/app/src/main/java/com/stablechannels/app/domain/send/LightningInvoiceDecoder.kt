package com.stablechannels.app.domain.send

/**
 * What the send flow needs from a parsed BOLT11 invoice. [descriptionHash] is the hex `h` tag, null
 * when the invoice carries a plain description.
 */
data class DecodedBolt11(
    val network: BitcoinNetwork,
    val amountMsat: Long?,
    val descriptionHash: String? = null,
    val expiresAtEpochSecs: Long = Long.MAX_VALUE,
)

/** What the classifier needs from a parsed BOLT12 offer. An empty [chains] means mainnet. */
data class DecodedBolt12(val chains: Set<BitcoinNetwork>)

/**
 * Parses BOLT11 invoices and BOLT12 offers. The node library does the parsing, so the domain
 * depends on this port and tests supply a fake.
 */
interface LightningInvoiceDecoder {
    /** The parsed invoice, or null when [raw] is not a valid BOLT11 invoice. */
    fun decodeBolt11(raw: String): DecodedBolt11?

    /** The parsed offer, or null when [raw] is not a valid BOLT12 offer. */
    fun decodeBolt12(raw: String): DecodedBolt12?
}
