package com.stablechannels.app.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.stablechannels.app.domain.seed.AppLifecyclePhase
import com.stablechannels.app.services.BiometricService.AuthResult
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SeedAccessGateTest {

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    private val prompts = mutableListOf<String>()
    private var unavailable = 0

    private fun gate(
        scope: CoroutineScope,
        requiresAuth: Boolean,
        reply: suspend () -> AuthResult,
    ) =
        SeedAccessGate(
            scope = scope,
            requiresAuth = { requiresAuth },
            authenticate = { reason ->
                prompts += reason
                reply()
            },
            phase = { AppLifecyclePhase.ACTIVE },
            onAuthUnavailable = { unavailable++ },
        )

    @Test
    fun `with App Lock on both rows ask for auth`() {
        var reveals = 0
        var restores = 0
        compose.setContent {
            val scope = rememberCoroutineScope()
            GatedBackupSettingsList(
                canReveal = true,
                gate = gate(scope, requiresAuth = true) { AuthResult.SUCCESS },
                onReveal = { reveals++ },
                onRestore = { restores++ },
            )
        }
        compose.onNodeWithText("Reveal Secret Recovery Phrase").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("Restore from Seed").performClick()
        compose.waitForIdle()
        assertEquals(listOf(REVEAL_AUTH_REASON, RESTORE_AUTH_REASON), prompts)
        assertEquals(1, reveals)
        assertEquals(1, restores)
    }

    @Test
    fun `taps while an auth prompt is pending are ignored`() = runTest {
        val reply = CompletableDeferred<AuthResult>()
        val gate = gate(this, requiresAuth = true) { reply.await() }
        var opened = 0
        gate.request("first") { opened++ }
        gate.request("second") { opened++ }
        runCurrent()
        assertEquals(listOf("first"), prompts)

        reply.complete(AuthResult.SUCCESS)
        runCurrent()
        assertEquals(1, opened)
        gate.request("third") { opened++ }
        runCurrent()
        assertEquals(listOf("first", "third"), prompts)
        assertEquals(2, opened)
    }

    @Test
    fun `with App Lock off a second quick tap opens nothing`() = runTest {
        val gate = gate(this, requiresAuth = false) { AuthResult.SUCCESS }
        var opened = 0
        gate.request("reveal") { opened++ }
        gate.request("restore") { opened++ }
        runCurrent()
        assertEquals(1, opened)
        assertEquals(emptyList<String>(), prompts)
    }

    @Test
    fun `a missing device lock says so and a cancel stays silent`() = runTest {
        var result = AuthResult.NOT_ENROLLED
        val gate = gate(this, requiresAuth = true) { result }
        var opened = 0
        gate.request("a") { opened++ }
        runCurrent()
        result = AuthResult.NOT_AVAILABLE
        gate.request("b") { opened++ }
        runCurrent()
        result = AuthResult.CANCELLED
        gate.request("c") { opened++ }
        runCurrent()
        assertEquals(2, unavailable)
        assertEquals(0, opened)
    }
}
