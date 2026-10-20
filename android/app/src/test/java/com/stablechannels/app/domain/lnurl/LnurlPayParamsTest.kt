package com.stablechannels.app.domain.lnurl

import java.security.MessageDigest
import java.util.Base64
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LnurlPayParamsTest {

    private fun params(
        minSendable: Long,
        maxSendable: Long,
        metadata: String = "[[\"text/plain\",\"Test\"]]",
        commentAllowed: Int? = null,
    ) =
        LnurlPayParams(
            tag = "payRequest",
            callback = "https://service.com/callback",
            minSendable = minSendable,
            maxSendable = maxSendable,
            metadata = metadata,
            commentAllowed = commentAllowed,
        )

    @Test
    fun `metadata parsing and hash`() {
        val metadata = "[[\"text/plain\",\"Coffee Tip\"],[\"image/png;base64\",\"abc123def\"]]"
        val p = params(1000, 2_100_000_000_000_000_000, metadata, 140)

        assertEquals(LnurlPayParams.MAX_SUPPLY_MSAT, p.maxSendable)
        assertEquals(1L, p.minSats)
        assertEquals(2_100_000_000_000_000L, p.maxSats)
        assertEquals("Coffee Tip", p.plainTextDescription)
        assertFalse(p.hasCustomSendBounds)
        assertTrue(p.hasValidBounds)
        assertTrue(p.isAmountValid(50_000))
        assertFalse(p.isAmountValid(500))

        val expected =
            MessageDigest.getInstance("SHA-256").digest(metadata.toByteArray()).joinToString("") {
                "%02x".format(it)
            }
        assertEquals(expected, p.metadataHashHex)
        assertEquals(64, p.metadataHashHex.length)
    }

    @Test
    fun `avatar image data decoding`() {
        val sample = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
        val base64 = Base64.getEncoder().encodeToString(sample)
        val p =
            params(
                1000,
                LnurlPayParams.MAX_SUPPLY_MSAT,
                "[[\"text/plain\",\"Coffee Tip\"],[\"image/png;base64\",\"$base64\"]]",
                140,
            )
        assertArrayEquals(sample, p.avatarBytes)
        assertNull(params(1000, 2000).avatarBytes)
    }

    @Test
    fun `custom bounds detection and validation`() {
        val restricted = params(50_000, 1_000_000)
        assertTrue(restricted.hasCustomSendBounds)
        assertTrue(restricted.hasValidBounds)
        assertEquals(50L, restricted.minSats)
        assertEquals(1000L, restricted.maxSats)

        assertFalse(params(500_000, 100_000).hasValidBounds)
    }

    @Test
    fun `comment validation`() {
        val p = params(1000, 10_000, commentAllowed = 50)
        assertTrue(p.isCommentValid(null))
        assertTrue(p.isCommentValid(""))
        assertTrue(p.isCommentValid("Thank you!"))
        assertTrue(p.isCommentValid("x".repeat(50)))
        assertFalse(p.isCommentValid("x".repeat(51)))
    }

    @Test
    fun `minSats ceiling at extreme values does not overflow`() {
        val max = params(Long.MAX_VALUE, Long.MAX_VALUE)
        assertEquals(Long.MAX_VALUE / 1000 + 1, max.minSats)
        assertEquals(Long.MAX_VALUE / 1000, max.maxSats)
        assertFalse(max.hasValidBounds)

        val nearMax = params(Long.MAX_VALUE - 998, Long.MAX_VALUE)
        assertEquals((Long.MAX_VALUE - 998) / 1000 + 1, nearMax.minSats)

        assertEquals(10L, params(10_000, 50_000).minSats)

        val supply = params(LnurlPayParams.MAX_SUPPLY_MSAT, LnurlPayParams.MAX_SUPPLY_MSAT)
        assertTrue(supply.hasValidBounds)
        assertFalse(params(1000, LnurlPayParams.MAX_SUPPLY_MSAT + 1).hasValidBounds)
    }
}
