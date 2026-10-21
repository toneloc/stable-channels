package com.stablechannels.app.services.lnurl

import com.stablechannels.app.domain.lnurl.LnurlError
import com.stablechannels.app.domain.lnurl.LnurlInvoiceValidator
import com.stablechannels.app.domain.lnurl.LnurlPayParams
import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.DecodedBolt11
import com.stablechannels.app.domain.send.DecodedBolt12
import com.stablechannels.app.domain.send.LightningInvoiceDecoder
import java.net.URI
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

/** Response shapes iOS refuses to decode are refused here too. */
class LnurlResponseRulesTest {
    private var body = ""
    private val decoder =
        object : LightningInvoiceDecoder {
            override fun decodeBolt11(raw: String) =
                DecodedBolt11(BitcoinNetwork.REGTEST, 50_000, null, Long.MAX_VALUE)

            override fun decodeBolt12(raw: String): DecodedBolt12? = null
        }
    private val service =
        LnurlService(
            BitcoinNetwork.REGTEST,
            LnurlInvoiceValidator(decoder),
            { LnurlHttpResponse(200, body) },
            { 0L },
        ) { _, _ ->
        }

    private fun payParams(commentAllowed: String) =
        """{"tag":"payRequest","callback":"https://a.example/cb","minSendable":1000,""" +
            """"maxSendable":100000,"metadata":"[[\"text/plain\",\"x\"]]","commentAllowed":$commentAllowed}"""

    private val params =
        LnurlPayParams(
            "payRequest",
            "https://a.example/cb",
            1000,
            100_000,
            """[["text/plain","x"]]""",
        )

    @Test
    fun `commentAllowed must be an integer when present`() = runBlocking {
        body = payParams("null")
        assertNull(service.fetchPayParams(URI("https://a.example/p")).commentAllowed)
        body = payParams("100")
        assertEquals(100, service.fetchPayParams(URI("https://a.example/p")).commentAllowed)
        for (bad in listOf("\"100\"", "1.5", "99999999999")) {
            body = payParams(bad)
            assertThrows(LnurlError.InvalidResponse::class.java) {
                runBlocking { service.fetchPayParams(URI("https://a.example/p")) }
            }
        }
    }

    @Test
    fun `malformed success action is InvalidResponse`() {
        for (action in listOf("5", "{}", """{"tag":1}""", """{"tag":"message","message":7}""")) {
            body = """{"pr":"lnbcrt1x","successAction":$action}"""
            assertThrows(LnurlError.InvalidResponse::class.java) {
                runBlocking { service.fetchInvoice(params, 50_000) }
            }
        }
    }

    @Test
    fun `absent or null success action is fine`() = runBlocking {
        body = """{"pr":"lnbcrt1x","successAction":null}"""
        assertEquals("lnbcrt1x", service.fetchInvoice(params, 50_000).invoice)
        body = """{"pr":"lnbcrt1x","successAction":{"tag":"message","message":"thanks"}}"""
        assertEquals("lnbcrt1x", service.fetchInvoice(params, 50_000).invoice)
    }
}
