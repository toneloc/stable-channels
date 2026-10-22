package com.stablechannels.app.services.lnurl

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import com.stablechannels.app.domain.lnurl.LnurlAvatar

/** Decodes a payee avatar to a small bitmap, refusing oversized or unexpected images. */
object LnurlAvatarDecoder {
    fun thumbnail(bytes: ByteArray): Bitmap? {
        if (!LnurlAvatar.isAcceptableEncoding(bytes)) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (!LnurlAvatar.isAcceptableSize(bounds.outWidth, bounds.outHeight)) return null

        // Power-of-two subsampling keeps the decoded bitmap near the thumbnail size.
        var sample = 1
        while (
            maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= LnurlAvatar.MAX_PIXEL_SIZE
        ) sample *= 2
        val decoded =
            BitmapFactory.decodeByteArray(
                bytes,
                0,
                bytes.size,
                BitmapFactory.Options().apply { inSampleSize = sample },
            ) ?: return null
        val scale = LnurlAvatar.MAX_PIXEL_SIZE.toFloat() / maxOf(decoded.width, decoded.height)
        if (scale >= 1f) return decoded
        return Bitmap.createScaledBitmap(
            decoded,
            (decoded.width * scale).toInt().coerceAtLeast(1),
            (decoded.height * scale).toInt().coerceAtLeast(1),
            true,
        )
    }
}
