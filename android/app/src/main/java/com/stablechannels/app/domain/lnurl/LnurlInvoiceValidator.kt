package com.stablechannels.app.domain.lnurl

import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.LightningInvoiceDecoder

/**
 * Checks the invoice an LNURL payee returned before anything is shown or paid: it must parse, be
 * for the wallet's network, carry exactly the requested amount and be unexpired.
 *
 * The description is reported, not enforced. LUD-06 asks for description_hash == sha256(metadata),
 * but the metadata and the callback come from the same server, so a hostile payee can always match
 * it, while custom-domain Lightning Address forwarders that rewrite the metadata would become
 * unpayable.
 */
/** A checked invoice: how its description relates to the metadata, and when it expires. */
data class CheckedLnurlInvoice(val binding: LnurlDescriptionBinding, val expiresAtEpochSecs: Long)

class LnurlInvoiceValidator(private val decoder: LightningInvoiceDecoder) {
    fun validate(
        invoice: String,
        amountMsat: Long,
        network: BitcoinNetwork,
        metadataHashHex: String,
        nowEpochSecs: Long,
    ): CheckedLnurlInvoice {
        if (invoice.isBlank()) throw LnurlError.InvalidResponse
        val decoded = decoder.decodeBolt11(invoice) ?: throw LnurlError.InvalidResponse
        if (decoded.network != network) throw LnurlError.InvoiceWrongNetwork
        val invoiceMsat = decoded.amountMsat ?: throw LnurlError.InvoiceWithoutAmount
        if (invoiceMsat != amountMsat)
            throw LnurlError.InvoiceAmountMismatch(amountMsat, invoiceMsat)
        if (nowEpochSecs >= decoded.expiresAtEpochSecs) throw LnurlError.InvoiceExpired
        val hash = decoded.descriptionHash
        val binding =
            when {
                hash == null -> LnurlDescriptionBinding.Direct
                hash.equals(metadataHashHex, ignoreCase = true) ->
                    LnurlDescriptionBinding.MatchesMetadata
                else -> LnurlDescriptionBinding.MismatchedHash(hash)
            }
        return CheckedLnurlInvoice(binding, decoded.expiresAtEpochSecs)
    }
}
