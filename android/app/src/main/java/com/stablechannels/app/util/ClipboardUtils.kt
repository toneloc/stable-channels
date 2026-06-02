package com.stablechannels.app.util

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle

/**
 * Utility for secure clipboard operations on sensitive data such as seed phrases.
 *
 * Security measures:
 * - Sets EXTRA_IS_SENSITIVE on API 33+ to prevent clipboard content from appearing
 *   in predictive text or clipboard history.
 * - Automatically clears the clipboard after 60 seconds.
 */
object ClipboardUtils {

    private const val CLEAR_DELAY_MS = 60_000L

    private val handler = Handler(Looper.getMainLooper())

    /**
     * Copies sensitive text to clipboard with security measures:
     * - Sets EXTRA_IS_SENSITIVE on API 33+
     * - Schedules clipboard clearing after 60 seconds
     */
    fun copySensitive(context: Context, label: String, text: String) {
        val clipboardManager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val clipData = ClipData.newPlainText(label, text)

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            clipData.description.extras = PersistableBundle().apply {
                putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
            }
        }

        clipboardManager.setPrimaryClip(clipData)

        // Schedule clipboard clearing after 60 seconds
        handler.postDelayed({ clearClipboard(context) }, CLEAR_DELAY_MS)
    }

    /**
     * Clears the clipboard by setting empty content.
     * Uses clearPrimaryClip() on API 28+ for a clean clear,
     * falls back to setting empty ClipData on older APIs.
     */
    fun clearClipboard(context: Context) {
        try {
            val clipboardManager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                clipboardManager.clearPrimaryClip()
            }
            // Also set empty clip as fallback (works even when clearPrimaryClip doesn't)
            clipboardManager.setPrimaryClip(ClipData.newPlainText("", ""))
        } catch (_: Exception) {
            // Some OEMs restrict clipboard access from background — silently fail
        }
    }
}
