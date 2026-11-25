package com.stablechannels.app.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class BackupSettingsListTest {

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    private var reveals = 0
    private var restores = 0

    private fun showList(canReveal: Boolean) {
        compose.setContent {
            BackupSettingsList(
                canReveal = canReveal,
                onReveal = { reveals++ },
                onRestore = { restores++ },
            )
        }
    }

    @Test
    fun `reveal row is disabled with no saved phrase`() {
        showList(canReveal = false)
        compose.onNodeWithText("Reveal Secret Recovery Phrase").assertIsNotEnabled()
        compose.onNodeWithText("Restore from Seed").assertIsEnabled()
    }

    @Test
    fun `rows open reveal and restore`() {
        showList(canReveal = true)
        compose.onNodeWithText("Reveal Secret Recovery Phrase").performClick()
        compose.onNodeWithText("Restore from Seed").performClick()
        assertEquals(1, reveals)
        assertEquals(1, restores)
    }

    @Test
    fun `shows the support banner and no old backup button`() {
        showList(canReveal = true)
        compose.onNodeWithText("Need Help?").assertExists()
        compose.onNodeWithText("Backup Seed Words").assertDoesNotExist()
    }
}
