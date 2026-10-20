package com.stablechannels.app.domain.lnurl

import java.net.URI
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LnurlSuccessActionTest {

    private val callback = URI("https://pay.shop.com/callback")

    private fun urlAction(description: String, url: String) =
        LnurlSuccessAction(tag = "url", description = description, url = url)

    @Test
    fun `url action on same host is accepted`() {
        val action = urlAction("Receipt", "https://pay.shop.com/receipt/123")
        assertTrue(action.isSameHostOrSubdomain(callback))
        assertEquals(
            LnurlSuccessAction.Action.Url("Receipt", URI("https://pay.shop.com/receipt/123")),
            action.validated(callback),
        )
    }

    @Test
    fun `url action on subdomain is accepted`() {
        val action = urlAction("Receipt", "https://sub.pay.shop.com/receipt/123")
        assertTrue(action.isSameHostOrSubdomain(callback))
        assertTrue(action.validated(callback) is LnurlSuccessAction.Action.Url)
    }

    @Test
    fun `url action on other host is unknown`() {
        val action = urlAction("Phishing", "https://malicious-redirect.com/login")
        assertFalse(action.isSameHostOrSubdomain(callback))
        assertEquals(LnurlSuccessAction.Action.Unknown("url"), action.validated(callback))
    }

    @Test
    fun `url action scheme rules`() {
        val torCallback = URI("http://shop.onion/callback")
        assertEquals(
            LnurlSuccessAction.Action.Url("Tor Receipt", URI("http://shop.onion/receipt/123")),
            urlAction("Tor Receipt", "http://shop.onion/receipt/123").validated(torCallback),
        )
        assertEquals(
            LnurlSuccessAction.Action.Unknown("url"),
            urlAction("Insecure", "http://insecure-shop.com/receipt").validated(callback),
        )
        assertEquals(
            LnurlSuccessAction.Action.Unknown("url"),
            urlAction("Exploit", "javascript:alert(1)").validated(callback),
        )
    }

    @Test
    fun `message length limit`() {
        assertEquals(
            LnurlSuccessAction.Action.Message("Payment received!"),
            LnurlSuccessAction(tag = "message", message = "Payment received!").validated(callback),
        )
        assertEquals(
            LnurlSuccessAction.Action.Unknown("message"),
            LnurlSuccessAction(tag = "message", message = "a".repeat(145)).validated(callback),
        )
    }

    @Test
    fun `aes action requires 16 byte iv`() {
        val valid =
            LnurlSuccessAction(
                tag = "aes",
                description = "Secret",
                ciphertext = "AQIDBA==",
                iv = "MDEyMzQ1Njc4OWFiY2RlZg==",
            )
        assertEquals(
            LnurlSuccessAction.Action.Aes("Secret", "AQIDBA==", "MDEyMzQ1Njc4OWFiY2RlZg=="),
            valid.validated(callback),
        )

        val badIv = valid.copy(iv = "AQID")
        assertEquals(LnurlSuccessAction.Action.Unknown("aes"), badIv.validated(callback))
    }
}
