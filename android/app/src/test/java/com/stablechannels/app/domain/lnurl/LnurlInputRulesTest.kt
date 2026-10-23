package com.stablechannels.app.domain.lnurl

import java.net.URI
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** Input rules tightened after review, matching how iOS reads the same input. */
class LnurlInputRulesTest {
    @Test
    fun `isAllowed refuses hosts a client could read as an ip address`() {
        listOf(
                "https://2130706433/",
                "https://127.1/",
                "https://10.1/",
                "https://１２７.0.0.1/",
                "https://café.example.com/",
            )
            .forEach { assertFalse(it, LnurlEndpoint.isAllowed(URI(it))) }
        assertTrue(LnurlEndpoint.isAllowed(URI("https://pay.example.com/")))
    }

    @Test
    fun `lightning address handle with a format character is refused`() {
        listOf("ali‮ce@example.com", "ali​ce@example.com").forEach {
            assertThrows(LnurlError.InvalidTarget::class.java) { LnurlEndpoint.resolve(it) }
        }
    }

    @Test
    fun `comment length counts visible characters`() {
        val params =
            LnurlPayParams(
                "payRequest",
                "https://a.example/cb",
                1000,
                2000,
                "[]",
                commentAllowed = 1,
            )
        assertTrue(params.isCommentValid("👨‍👩‍👧")) // family emoji
        assertTrue(params.isCommentValid("é")) // e + combining acute
        assertFalse(params.isCommentValid("ab"))
    }

    @Test
    fun `success action text limits count visible characters`() {
        val message = "👍".repeat(144)
        val action = LnurlSuccessAction(tag = "message", message = message)
        assertEquals(
            LnurlSuccessAction.Action.Message(message),
            action.validated(URI("https://a.example/cb")),
        )
    }

    @Test
    fun `metadata with a non string entry is unusable`() {
        val params =
            LnurlPayParams(
                "payRequest",
                "https://a.example/cb",
                1000,
                2000,
                """[["text/plain","x"],5]""",
            )
        assertNull(params.plainTextDescription)
        val nested =
            LnurlPayParams(
                "payRequest",
                "https://a.example/cb",
                1000,
                2000,
                """[["text/plain","x",1]]""",
            )
        assertNull(nested.plainTextDescription)
    }
}
