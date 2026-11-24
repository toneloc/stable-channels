package com.stablechannels.app.util

import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowLooper

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ClipboardUtilsTest {

    private val context: Context = RuntimeEnvironment.getApplication()
    private val clipboard = context.getSystemService(ClipboardManager::class.java)

    @Test
    fun `copySensitive marks the clip sensitive`() {
        ClipboardUtils.copySensitive(context, "Seed Phrase", "abandon ability able")

        val clip = clipboard.primaryClip!!
        assertEquals("abandon ability able", clip.getItemAt(0).text.toString())
        assertTrue(clip.description.extras.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE))
    }

    @Test
    fun `copySensitive clears the clipboard after 60 seconds`() {
        ClipboardUtils.copySensitive(context, "Seed Phrase", "abandon ability able")

        ShadowLooper.idleMainLooper(ClipboardUtils.CLEAR_DELAY_SECONDS - 1, TimeUnit.SECONDS)
        assertEquals("abandon ability able", clipboard.primaryClip?.getItemAt(0)?.text.toString())

        ShadowLooper.idleMainLooper(1, TimeUnit.SECONDS)

        assertEquals("", clipboard.primaryClip?.getItemAt(0)?.text?.toString().orEmpty())
    }
}
