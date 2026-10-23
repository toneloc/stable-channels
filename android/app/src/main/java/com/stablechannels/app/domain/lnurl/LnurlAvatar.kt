package com.stablechannels.app.domain.lnurl

/**
 * Limits on a payee avatar. The bytes come from whoever runs the payee's server, so they are
 * checked before any decoder sees them.
 */
object LnurlAvatar {
    /** Largest encoded avatar accepted, in bytes. */
    const val MAX_ENCODED_BYTES = 256 * 1024

    /** Longest source side accepted, checked from the image header before decoding. */
    const val MAX_SOURCE_SIDE = 4096

    /** Longest side of the decoded thumbnail; covers a 36dp avatar at 3x. */
    const val MAX_PIXEL_SIZE = 128

    private val PNG_SIGNATURE = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
    private val JPEG_SIGNATURE = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte())

    /** True for a PNG or JPEG no larger than [MAX_ENCODED_BYTES]. */
    fun isAcceptableEncoding(bytes: ByteArray): Boolean =
        bytes.size <= MAX_ENCODED_BYTES &&
            (bytes.startsWith(PNG_SIGNATURE) || bytes.startsWith(JPEG_SIGNATURE))

    fun isAcceptableSize(width: Int, height: Int): Boolean =
        width in 1..MAX_SOURCE_SIDE && height in 1..MAX_SOURCE_SIDE

    private fun ByteArray.startsWith(prefix: ByteArray): Boolean =
        size >= prefix.size && prefix.indices.all { this[it] == prefix[it] }
}
