package com.stablechannels.app.domain.lnurl

import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.DecodedBolt11
import com.stablechannels.app.domain.send.DecodedBolt12
import com.stablechannels.app.domain.send.LightningInvoiceDecoder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LnurlInvoiceValidatorTest {

    private class FakeDecoder(private val invoices: Map<String, DecodedBolt11>) :
        LightningInvoiceDecoder {
        override fun decodeBolt11(raw: String): DecodedBolt11? = invoices[raw]

        override fun decodeBolt12(raw: String): DecodedBolt12? = null
    }

    private val metadataHash = "a".repeat(64)
    private val now = 1_700_000_000L
    private val expiry = now + 3600

    private val validator =
        LnurlInvoiceValidator(
            FakeDecoder(
                mapOf(
                    "valid50k" to
                        DecodedBolt11(BitcoinNetwork.REGTEST, 50_000, metadataHash, expiry),
                    "direct" to DecodedBolt11(BitcoinNetwork.REGTEST, 50_000, null, expiry),
                    "mainnet" to
                        DecodedBolt11(BitcoinNetwork.BITCOIN, 50_000, metadataHash, expiry),
                    "amountless" to
                        DecodedBolt11(BitcoinNetwork.REGTEST, null, metadataHash, expiry),
                    "expired" to DecodedBolt11(BitcoinNetwork.REGTEST, 50_000, metadataHash, now),
                )
            )
        )

    private fun validate(invoice: String, amountMsat: Long = 50_000, hash: String = metadataHash) =
        validator.validate(invoice, amountMsat, BitcoinNetwork.REGTEST, hash, now)

    private fun errorOf(block: () -> Unit): LnurlError? =
        try {
            block()
            null
        } catch (e: LnurlError) {
            e
        }

    @Test
    fun `reports description binding`() {
        assertEquals(
            CheckedLnurlInvoice(LnurlDescriptionBinding.MatchesMetadata, expiry),
            validate("valid50k"),
        )
        assertEquals(
            LnurlDescriptionBinding.MatchesMetadata,
            validate("valid50k", hash = metadataHash.uppercase()).binding,
        )
        assertEquals(
            LnurlDescriptionBinding.MismatchedHash(metadataHash),
            validate("valid50k", hash = "0".repeat(64)).binding,
        )
        assertEquals(LnurlDescriptionBinding.Direct, validate("direct").binding)
    }

    @Test
    fun `expired invoice is rejected`() {
        assertEquals(LnurlError.InvoiceExpired, errorOf { validate("expired") })
    }

    @Test
    fun `amountless invoice is rejected`() {
        assertEquals(LnurlError.InvoiceWithoutAmount, errorOf { validate("amountless") })
    }

    @Test
    fun `wrong network invoice is rejected`() {
        assertEquals(LnurlError.InvoiceWrongNetwork, errorOf { validate("mainnet") })
    }

    @Test
    fun `amount mismatch is rejected`() {
        assertEquals(
            LnurlError.InvoiceAmountMismatch(expectedMsat = 49_000, actualMsat = 50_000),
            errorOf { validate("valid50k", amountMsat = 49_000) },
        )
    }

    @Test
    fun `blank or unparseable invoice is invalid response`() {
        assertEquals(LnurlError.InvalidResponse, errorOf { validate("") })
        assertEquals(LnurlError.InvalidResponse, errorOf { validate("   ") })
        assertEquals(LnurlError.InvalidResponse, errorOf { validate("garbage") })
    }

    @Test
    fun `quote expires with a 30 second margin`() {
        val quote =
            LnurlInvoiceQuote("inv", 50_000, null, LnurlDescriptionBinding.MatchesMetadata, 1000)
        assertEquals(30L, LnurlInvoiceQuote.EXPIRY_MARGIN_SECS)
        assertFalse(quote.isExpired(969))
        assertTrue(quote.isExpired(970))
        assertTrue(quote.isExpired(1000))
        assertFalse(quote.isDescriptionUnbound)
        assertTrue(quote.copy(binding = LnurlDescriptionBinding.Direct).isDescriptionUnbound)
    }

    @Test
    fun `normalized comment trims and drops blank`() {
        assertEquals("hi there", LnurlInvoiceQuote.normalizedComment("  hi there \n"))
        assertNull(LnurlInvoiceQuote.normalizedComment("   "))
        assertNull(LnurlInvoiceQuote.normalizedComment(""))
        assertNull(LnurlInvoiceQuote.normalizedComment(null))
    }
}
