package com.stablechannels.app.domain.lnurl

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LnurlAvatarTest {

    private val png = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
    private val jpeg = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 0xE0.toByte())

    @Test
    fun `rejects non-image and oversized bytes`() {
        assertFalse(LnurlAvatar.isAcceptableEncoding("not an image".toByteArray()))
        assertFalse(
            LnurlAvatar.isAcceptableEncoding(png + ByteArray(LnurlAvatar.MAX_ENCODED_BYTES))
        )
        assertFalse(LnurlAvatar.isAcceptableEncoding(ByteArray(0)))
    }

    @Test
    fun `accepts png and jpeg within limit`() {
        assertTrue(LnurlAvatar.isAcceptableEncoding(png))
        assertTrue(LnurlAvatar.isAcceptableEncoding(jpeg))
        assertTrue(
            LnurlAvatar.isAcceptableEncoding(
                png + ByteArray(LnurlAvatar.MAX_ENCODED_BYTES - png.size)
            )
        )
    }

    @Test
    fun `source size limits`() {
        assertTrue(LnurlAvatar.isAcceptableSize(1, 1))
        assertTrue(
            LnurlAvatar.isAcceptableSize(LnurlAvatar.MAX_SOURCE_SIDE, LnurlAvatar.MAX_SOURCE_SIDE)
        )
        assertFalse(LnurlAvatar.isAcceptableSize(0, 10))
        assertFalse(LnurlAvatar.isAcceptableSize(10, -1))
        assertFalse(LnurlAvatar.isAcceptableSize(LnurlAvatar.MAX_SOURCE_SIDE + 1, 1))
    }
}
