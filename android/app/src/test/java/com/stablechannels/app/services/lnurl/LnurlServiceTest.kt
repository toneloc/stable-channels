package com.stablechannels.app.services.lnurl

import com.stablechannels.app.domain.lnurl.LnurlDescriptionBinding
import com.stablechannels.app.domain.lnurl.LnurlError
import com.stablechannels.app.domain.lnurl.LnurlInvoiceValidator
import com.stablechannels.app.domain.lnurl.LnurlPayParams
import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.DecodedBolt11
import com.stablechannels.app.domain.send.DecodedBolt12
import com.stablechannels.app.domain.send.LightningInvoiceDecoder
import java.net.URI
import kotlinx.coroutines.runBlocking
import okhttp3.HttpUrl.Companion.toHttpUrl
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** Ported from the service-level cases of the iOS LNURLServiceTests. */
class LnurlServiceTest {

    private val now = 1_700_000_000L

    /** sha256 of the empty string: the metadata hash of `makeParams()` with default metadata. */
    private val emptyMetadataHash =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

    private val invoices = mutableMapOf<String, DecodedBolt11>()
    private val decoder =
        object : LightningInvoiceDecoder {
            override fun decodeBolt11(raw: String) = invoices[raw]

            override fun decodeBolt12(raw: String): DecodedBolt12? = null
        }

    private val responses = mutableMapOf<URI, LnurlHttpResponse>()
    private val requested = mutableListOf<URI>()
    private val transport = LnurlTransport { url ->
        requested += url
        responses[url] ?: LnurlHttpResponse(200, invoiceJson(VALID_50K))
    }

    private val audits = mutableListOf<Pair<String, Map<String, Any?>>>()

    private fun service(
        network: BitcoinNetwork = BitcoinNetwork.REGTEST,
        transport: LnurlTransport = this.transport,
    ) =
        LnurlService(network, LnurlInvoiceValidator(decoder), transport, { now }) { event, data ->
            audits += event to data
        }

    private fun invoice(
        raw: String,
        amountMsat: Long? = 50_000,
        descriptionHash: String? = emptyMetadataHash,
        expiresAt: Long = now + 3600,
        network: BitcoinNetwork = BitcoinNetwork.REGTEST,
    ) {
        invoices[raw] = DecodedBolt11(network, amountMsat, descriptionHash, expiresAt)
    }

    private fun invoiceJson(pr: String) = """{"pr":"$pr","status":"OK"}"""

    private fun makeParams(
        callback: String = CALLBACK,
        metadata: String = "",
        minSendable: Long = 1_000,
        maxSendable: Long = 100_000_000,
        commentAllowed: Int? = 100,
    ) = LnurlPayParams("payRequest", callback, minSendable, maxSendable, metadata, commentAllowed)

    private fun payParamsJson(
        tag: String = "payRequest",
        minSendable: String = "1000",
        maxSendable: String = "1000000000",
        metadata: String = """[[\"text/plain\",\"prabal\"]]""",
    ) =
        """
        {
            "tag": "$tag",
            "callback": "https://service.example.com/callback",
            "minSendable": $minSendable,
            "maxSendable": $maxSendable,
            "metadata": "$metadata",
            "commentAllowed": 140
        }
        """

    private inline fun <reified T : Throwable> assertFails(noinline block: suspend () -> Unit): T =
        assertThrows(T::class.java) { runBlocking { block() } }

    init {
        invoice(VALID_50K)
    }

    // fetchPayParams

    @Test
    fun `404 with an error body maps to RecipientNotFound before parsing the body`() {
        responses[PAY_URL] =
            LnurlHttpResponse(404, """{"status":"ERROR","reason":"User not found"}""")
        assertFails<LnurlError.RecipientNotFound> { service().fetchPayParams(PAY_URL) }
    }

    @Test
    fun `non 2xx other than 404 is InvalidResponse`() {
        responses[PAY_URL] = LnurlHttpResponse(500, payParamsJson())
        assertFails<LnurlError.InvalidResponse> { service().fetchPayParams(PAY_URL) }
    }

    @Test
    fun `status ERROR in a 200 body is ErrorResponse with the reason`() {
        responses[PAY_URL] =
            LnurlHttpResponse(200, """{"status":"ERROR","reason":"User not found"}""")
        val error = assertFails<LnurlError.ErrorResponse> { service().fetchPayParams(PAY_URL) }
        assertEquals("User not found", error.reason)
    }

    @Test
    fun `bare reason without tag is ErrorResponse`() {
        responses[PAY_URL] = LnurlHttpResponse(200, """{"reason":"Disabled"}""")
        val error = assertFails<LnurlError.ErrorResponse> { service().fetchPayParams(PAY_URL) }
        assertEquals("Disabled", error.reason)
    }

    @Test
    fun `mocked pay resolution parses the parameters`() = runBlocking {
        responses[PAY_URL] = LnurlHttpResponse(200, payParamsJson())
        val params = service().fetchPayParams(PAY_URL)
        assertEquals("payrequest", params.tag.lowercase())
        assertEquals(CALLBACK, params.callback)
        assertEquals(1_000L, params.minSendable)
        assertEquals(1_000_000_000L, params.maxSendable)
        assertEquals(140, params.commentAllowed)
        assertEquals("prabal", params.plainTextDescription)
        assertEquals(listOf(PAY_URL), requested)
    }

    @Test
    fun `missing text plain metadata is InvalidMetadata`() {
        responses[PAY_URL] =
            LnurlHttpResponse(
                200,
                payParamsJson(metadata = """[[\"image/png;base64\",\"abc123def\"]]"""),
            )
        assertFails<LnurlError.InvalidMetadata> { service().fetchPayParams(PAY_URL) }
    }

    @Test
    fun `non payRequest tag is UnsupportedTag`() {
        responses[PAY_URL] = LnurlHttpResponse(200, payParamsJson(tag = "withdrawRequest"))
        val error = assertFails<LnurlError.UnsupportedTag> { service().fetchPayParams(PAY_URL) }
        assertEquals("withdrawRequest", error.tag)
    }

    // iOS uses UInt64; Kotlin Long cannot hold UInt64.max, so the parser rejects it outright.
    @Test
    fun `uint64 max minSendable is InvalidResponse`() {
        val max = "18446744073709551615"
        responses[PAY_URL] =
            LnurlHttpResponse(200, payParamsJson(minSendable = max, maxSendable = max))
        assertFails<LnurlError.InvalidResponse> { service().fetchPayParams(PAY_URL) }
    }

    @Test
    fun `long max minSendable is AmountOutOfBounds without overflow`() {
        val max = Long.MAX_VALUE.toString()
        responses[PAY_URL] =
            LnurlHttpResponse(200, payParamsJson(minSendable = max, maxSendable = max))
        val error = assertFails<LnurlError.AmountOutOfBounds> { service().fetchPayParams(PAY_URL) }
        assertEquals(Long.MAX_VALUE / 1000 + 1, error.minSats)
        assertEquals(Long.MAX_VALUE / 1000, error.maxSats)
    }

    // fetchInvoice

    @Test
    fun `valid invoice succeeds and is bound to the metadata`() = runBlocking {
        val quote = service().fetchInvoice(makeParams(), 50_000, "Tip")
        assertEquals(VALID_50K, quote.invoice)
        assertEquals(50_000L, quote.amountMsat)
        assertEquals("Tip", quote.comment)
        assertEquals(LnurlDescriptionBinding.MatchesMetadata, quote.binding)
        assertEquals(now + 3600, quote.expiresAtEpochSecs)
        assertFalse(quote.isDescriptionUnbound)
        assertTrue(audits.isEmpty())
    }

    @Test
    fun `blank comment is dropped even when the payee allows no comments`() = runBlocking {
        val quote = service().fetchInvoice(makeParams(commentAllowed = null), 50_000, "   ")
        assertNull(quote.comment)
        assertNull(requested.single().toString().toHttpUrl().queryParameter("comment"))
    }

    @Test
    fun `amount below or above the bounds is AmountOutOfBounds without a request`() {
        val params = makeParams(minSendable = 10_000, maxSendable = 100_000, commentAllowed = 20)
        assertFails<LnurlError.AmountOutOfBounds> { service().fetchInvoice(params, 5_000) }
        assertFails<LnurlError.AmountOutOfBounds> { service().fetchInvoice(params, 150_000) }
        assertTrue(requested.isEmpty())
    }

    @Test
    fun `mismatched amount is InvoiceAmountMismatch`() {
        invoice(MISMATCHED_100K, amountMsat = 100_000)
        responses[callbackFor(50_000)] = LnurlHttpResponse(200, invoiceJson(MISMATCHED_100K))
        val error =
            assertFails<LnurlError.InvoiceAmountMismatch> {
                service().fetchInvoice(makeParams(), 50_000)
            }
        assertEquals(50_000L, error.expectedMsat)
        assertEquals(100_000L, error.actualMsat)
    }

    // A forwarder that rewrites the metadata returns an invoice whose hash no longer matches.
    @Test
    fun `mismatched description hash still succeeds and is audit logged`() = runBlocking {
        val params = makeParams(metadata = """[["text/plain","Coffee"]]""")
        val quote = service().fetchInvoice(params, 50_000)
        assertEquals(VALID_50K, quote.invoice)
        assertEquals(LnurlDescriptionBinding.MismatchedHash(emptyMetadataHash), quote.binding)
        assertTrue(quote.isDescriptionUnbound)
        assertEquals(1, audits.size)
        val (event, data) = audits.single()
        assertEquals("LNURL_INVOICE_DESCRIPTION_UNBOUND", event)
        assertEquals(params.metadataHashHex, data["expected_hash"])
        assertEquals("service.example.com", data["callback_host"])
    }

    @Test
    fun `direct description succeeds and is audit logged`() = runBlocking {
        invoice(DIRECT, descriptionHash = null)
        responses[callbackFor(50_000)] = LnurlHttpResponse(200, invoiceJson(DIRECT))
        val quote = service().fetchInvoice(makeParams(), 50_000)
        assertEquals(DIRECT, quote.invoice)
        assertEquals(LnurlDescriptionBinding.Direct, quote.binding)
        assertTrue(quote.isDescriptionUnbound)
        assertEquals("LNURL_INVOICE_DESCRIPTION_UNBOUND", audits.single().first)
    }

    // Relaxing the description check must not relax the amount check.
    @Test
    fun `mismatched amount and description hash is still InvoiceAmountMismatch`() {
        invoice(MISMATCHED_100K, amountMsat = 100_000, descriptionHash = "00".repeat(32))
        responses[callbackFor(50_000)] = LnurlHttpResponse(200, invoiceJson(MISMATCHED_100K))
        val params = makeParams(metadata = """[["text/plain","Coffee"]]""")
        assertFails<LnurlError.InvoiceAmountMismatch> { service().fetchInvoice(params, 50_000) }
        assertTrue(audits.isEmpty())
    }

    @Test
    fun `expired invoice is InvoiceExpired`() {
        invoice(EXPIRED, expiresAt = now)
        responses[callbackFor(50_000)] = LnurlHttpResponse(200, invoiceJson(EXPIRED))
        assertFails<LnurlError.InvoiceExpired> { service().fetchInvoice(makeParams(), 50_000) }
    }

    @Test
    fun `amountless invoice is InvoiceWithoutAmount`() {
        invoice(AMOUNTLESS, amountMsat = null)
        responses[callbackFor(50_000)] = LnurlHttpResponse(200, invoiceJson(AMOUNTLESS))
        assertFails<LnurlError.InvoiceWithoutAmount> {
            service().fetchInvoice(makeParams(), 50_000)
        }
    }

    @Test
    fun `malformed bolt11 is InvalidResponse`() {
        responses[callbackFor(50_000)] =
            LnurlHttpResponse(200, invoiceJson("lnbcnotaninvoice12345"))
        assertFails<LnurlError.InvalidResponse> { service().fetchInvoice(makeParams(), 50_000) }
    }

    @Test
    fun `missing pr is InvalidResponse`() {
        responses[callbackFor(50_000)] = LnurlHttpResponse(200, """{"status":"OK"}""")
        assertFails<LnurlError.InvalidResponse> { service().fetchInvoice(makeParams(), 50_000) }
    }

    @Test
    fun `status ERROR on the callback is ErrorResponse`() {
        responses[callbackFor(50_000)] =
            LnurlHttpResponse(200, """{"status":"ERROR","reason":"Amount too small"}""")
        val error =
            assertFails<LnurlError.ErrorResponse> { service().fetchInvoice(makeParams(), 50_000) }
        assertEquals("Amount too small", error.reason)
    }

    @Test
    fun `invoice for another network is InvoiceWrongNetwork`() {
        assertFails<LnurlError.InvoiceWrongNetwork> {
            service(network = BitcoinNetwork.TESTNET).fetchInvoice(makeParams(), 50_000)
        }
    }

    @Test
    fun `overlong comment is InvalidComment without a request`() {
        val params = makeParams(commentAllowed = 10)
        assertFails<LnurlError.InvalidComment> {
            service().fetchInvoice(params, 50_000, "this comment is far too long for limit 10")
        }
        assertTrue(requested.isEmpty())
    }

    @Test
    fun `unparsable callback is InvalidTarget without a request`() {
        val params = makeParams(callback = "not a valid url at all")
        assertFails<LnurlError.InvalidTarget> { service().fetchInvoice(params, 50_000) }
        assertTrue(requested.isEmpty())
    }

    @Test
    fun `comment with reserved characters is encoded in the requested url`() = runBlocking {
        val comment = "coffee+croissant & hello? test=1"
        service().fetchInvoice(makeParams(), 50_000, comment)
        val url = requested.single()
        val parsed = url.toString().toHttpUrl()
        assertEquals("50000", parsed.queryParameter("amount"))
        assertEquals(comment, parsed.queryParameter("comment"))
        assertTrue(url.rawQuery.contains("%26"))
        assertTrue(url.rawQuery.contains("%2B"))
    }

    // buildInvoiceCallbackUrl

    @Test
    fun `callback url preserves an existing query and encodes the appended comment`() {
        val url =
            LnurlService.buildInvoiceCallbackUrl(
                "https://example.com/api?tag=pay&search=hello+world",
                50_000,
                "tip+bonus=yes;extra",
            )
        assertEquals(
            "tag=pay&search=hello+world&amount=50000&comment=tip%2Bbonus%3Dyes%3Bextra",
            url.rawQuery,
        )
    }

    @Test
    fun `callback url encodes plus ampersand equals hash and spaces`() {
        val url = LnurlService.buildInvoiceCallbackUrl(CALLBACK, 1_000, "a+b&c=d#e f")
        assertEquals("amount=1000&comment=a%2Bb%26c%3Dd%23e%20f", url.rawQuery)
        assertNull(url.rawFragment)
    }

    @Test
    fun `callback url trims the comment and drops a blank one`() {
        assertEquals(
            "$CALLBACK?amount=1000&comment=Tip",
            LnurlService.buildInvoiceCallbackUrl(CALLBACK, 1_000, "  Tip \n").toString(),
        )
        assertEquals(
            "$CALLBACK?amount=1000",
            LnurlService.buildInvoiceCallbackUrl(CALLBACK, 1_000, "   ").toString(),
        )
        assertEquals(
            "$CALLBACK?amount=1000",
            LnurlService.buildInvoiceCallbackUrl(CALLBACK, 1_000, null).toString(),
        )
    }

    @Test
    fun `callback url that cannot be parsed is InvalidTarget`() {
        assertThrows(LnurlError.InvalidTarget::class.java) {
            LnurlService.buildInvoiceCallbackUrl("not a valid url at all", 1_000, null)
        }
    }

    // Transport policy through the real client

    @Test
    fun `onion endpoint without a tor transport is OnionUnsupported`() {
        val service = service(transport = LnurlHttpClient())
        assertFails<LnurlError.OnionUnsupported> {
            service.fetchPayParams(URI("http://service.onion/lnurlp"))
        }
        assertFails<LnurlError.OnionUnsupported> {
            service.fetchInvoice(makeParams(callback = "http://service.onion/callback"), 50_000)
        }
    }

    @Test
    fun `clearnet http is InsecureEndpoint for both stages`() {
        val service = service(transport = LnurlHttpClient())
        assertFails<LnurlError.InsecureEndpoint> {
            service.fetchPayParams(URI("http://service.com/lnurlp"))
        }
        assertFails<LnurlError.InsecureEndpoint> {
            service.fetchInvoice(makeParams(callback = "http://service.com/callback"), 50_000)
        }
    }

    private companion object {
        const val CALLBACK = "https://service.example.com/callback"
        val PAY_URL = URI("https://service.example.com/.well-known/lnurlp/prabal")

        // Opaque invoice strings; the fake decoder decides what each one decodes to.
        const val VALID_50K = "lnbcrt500n1valid"
        const val MISMATCHED_100K = "lnbcrt1u1mismatched"
        const val DIRECT = "lnbcrt500n1direct"
        const val EXPIRED = "lnbcrt500n1expired"
        const val AMOUNTLESS = "lnbcrt1amountless"

        fun callbackFor(amountMsat: Long) = URI("$CALLBACK?amount=$amountMsat")
    }
}
