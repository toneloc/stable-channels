package com.stablechannels.app.domain.send

import java.net.URI
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AddressVisualChunkerTest {
    private val rawInvoice =
        "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"

    private fun invoicePreview(raw: String): InvoicePreview =
        (AddressVisualChunker.format(SendDestination.Bolt11(raw, null))
                as DestinationVisual.Invoice)
            .preview

    @Test
    fun `p2pkh address highlights boundary chunks`() {
        val address = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        val chunked = AddressVisualChunker.chunk(address)

        assertEquals(address, chunked.raw)
        assertEquals(9, chunked.chunks.size)
        assertTrue(chunked.chunks[0].isHighlighted)
        assertTrue(chunked.chunks[1].isHighlighted)
        assertTrue(chunked.chunks.last().isHighlighted)
    }

    @Test
    fun `lightning address is not chunked`() {
        val dest =
            SendDestination.LightningAddress(
                "alice",
                "example.com",
                URI("https://example.com/.well-known/lnurlp/alice"),
            )
        val visual = AddressVisualChunker.format(dest)

        assertEquals(DestinationVisual.LightningAddress("alice", "example.com"), visual)
        assertEquals("alice@example.com", dest.raw)
    }

    @Test
    fun `lnurl pay is not chunked`() {
        val visual =
            AddressVisualChunker.format(SendDestination.LnurlPay(URI("https://ln.tips/service")))

        assertEquals(DestinationVisual.Lnurl("ln.tips", "https://ln.tips/service"), visual)
    }

    @Test
    fun `invoice preview keeps prefix and suffix`() {
        val visual =
            AddressVisualChunker.format(SendDestination.Bolt11(rawInvoice, 10_000_000L))
                as DestinationVisual.Invoice
        val preview = visual.preview

        assertEquals(rawInvoice, preview.raw)
        assertEquals(rawInvoice.take(14), preview.prefix)
        assertEquals("········", preview.middle)
        assertEquals(rawInvoice.takeLast(10), preview.suffix)
    }

    @Test
    fun `bolt12 offer uses invoice preview`() {
        val visual = AddressVisualChunker.format(SendDestination.Bolt12(rawInvoice))
        assertTrue(visual is DestinationVisual.Invoice)
    }

    @Test
    fun `segwit address highlights two chunks at each end`() {
        val address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        val result = AddressVisualChunker.chunk(address, chunkSize = 4)

        assertEquals(11, result.chunks.size)
        assertEquals("bc1q", result.chunks[0].text)
        assertEquals("ar0s", result.chunks[1].text)
        assertTrue(result.chunks[0].isHighlighted)
        assertTrue(result.chunks[1].isHighlighted)
        for (i in 2..8) assertFalse("chunk $i", result.chunks[i].isHighlighted)
        assertEquals("wf5m", result.chunks[9].text)
        assertEquals("dq", result.chunks[10].text)
        assertTrue(result.chunks[9].isHighlighted)
        assertTrue(result.chunks[10].isHighlighted)
        assertEquals(address, result.chunks.joinToString("") { it.text })
    }

    @Test
    fun `tiny addresses form a single highlighted chunk`() {
        listOf("b", "1Az", "bc1q").forEach { raw ->
            val chunked = AddressVisualChunker.chunk(raw)
            assertEquals(1, chunked.chunks.size)
            assertEquals(raw, chunked.chunks[0].text)
            assertTrue(chunked.chunks[0].isHighlighted)
        }
    }

    @Test
    fun `exact chunk multiples highlight the right chunks`() {
        fun highlights(raw: String) =
            AddressVisualChunker.chunk(raw).chunks.map { it.isHighlighted }

        assertEquals(listOf(true, true), highlights("12345678"))
        assertEquals(listOf(true, false, true), highlights("123456789012"))
        assertEquals(listOf(true, false, false, true), highlights("1234567890123456"))
        assertEquals(listOf(true, true, false, true, true), highlights("12345678901234567890"))
    }

    @Test
    fun `surrounding whitespace is trimmed`() {
        val clean = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        val result = AddressVisualChunker.chunk("\n  \t $clean \r\n  ")

        assertEquals(11, result.chunks.size)
        assertEquals(clean, result.raw)
        assertEquals(clean, result.chunks.joinToString("") { it.text })
    }

    @Test
    fun `invoice preview threshold is 28 characters`() {
        val invoice28 = "lnbc123456789012345678901234"
        assertEquals(InvoicePreview(invoice28, "", "", invoice28), invoicePreview(invoice28))

        val invoice29 = "lnbc1234567890123456789012345"
        assertEquals(
            InvoicePreview(invoice29.take(14), "········", invoice29.takeLast(10), invoice29),
            invoicePreview(invoice29),
        )
    }

    @Test
    fun `empty and whitespace only input yields no chunks`() {
        listOf("", "   \n\t  ").forEach {
            val chunked = AddressVisualChunker.chunk(it)
            assertEquals(0, chunked.chunks.size)
            assertEquals("", chunked.raw)
        }
    }

    @Test
    fun `zero or negative chunk size yields no chunks`() {
        listOf(0, -4).forEach { size ->
            val chunked = AddressVisualChunker.chunk("bc1qar0s", chunkSize = size)
            assertEquals(0, chunked.chunks.size)
            assertEquals("bc1qar0s", chunked.raw)
        }
    }
}
