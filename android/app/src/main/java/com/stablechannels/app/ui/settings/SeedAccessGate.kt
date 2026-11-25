package com.stablechannels.app.ui.settings

import androidx.compose.runtime.Composable
import com.stablechannels.app.domain.seed.AppLifecyclePhase
import com.stablechannels.app.domain.seed.SeedAccessPolicy
import com.stablechannels.app.services.BiometricService.AuthResult
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

internal const val REVEAL_AUTH_REASON = "Authenticate to reveal your Secret Recovery Phrase"
internal const val RESTORE_AUTH_REASON = "Authenticate to restore from a recovery phrase"
internal const val AUTH_UNAVAILABLE_MESSAGE =
    "Authentication isn't available on this device right now"

internal const val AUTH_FAILED_MESSAGE = "Authentication failed. Try again."

/** What to tell the user after an auth attempt: nothing on success or a cancel. */
internal fun authFeedback(result: AuthResult): String? =
    when (result) {
        AuthResult.SUCCESS,
        AuthResult.CANCELLED -> null
        AuthResult.NOT_AVAILABLE,
        AuthResult.NOT_ENROLLED -> AUTH_UNAVAILABLE_MESSAGE
        AuthResult.BIOMETRY_FAILED,
        AuthResult.DEVICE_CREDENTIAL_FAILED -> AUTH_FAILED_MESSAGE
    }

/**
 * Runs one [SeedAccessPolicy.authorize] at a time: taps while one is pending are ignored, so a
 * double tap cannot raise two prompts or open two seed screens.
 */
internal class SeedAccessGate(
    private val scope: CoroutineScope,
    private val requiresAuth: () -> Boolean,
    private val authenticate: suspend (reason: String) -> AuthResult,
    private val phase: () -> AppLifecyclePhase,
    private val onAuthFeedback: (String) -> Unit,
) {
    private var inFlight = false

    fun request(reason: String, then: () -> Unit) {
        if (inFlight) return
        inFlight = true
        scope.launch {
            try {
                val authorized =
                    SeedAccessPolicy.authorize(
                        requiresAuth = requiresAuth(),
                        authenticate = {
                            val result = authenticate(reason)
                            authFeedback(result)?.let(onAuthFeedback)
                            result == AuthResult.SUCCESS
                        },
                        phaseAfter = phase,
                    )
                if (authorized) then()
            } finally {
                inFlight = false
            }
        }
    }
}

/** [BackupSettingsList] with both rows behind [gate]. */
@Composable
internal fun GatedBackupSettingsList(
    canReveal: Boolean,
    gate: SeedAccessGate,
    onReveal: () -> Unit,
    onRestore: () -> Unit,
) {
    BackupSettingsList(
        canReveal = canReveal,
        onReveal = { gate.request(REVEAL_AUTH_REASON, onReveal) },
        onRestore = { gate.request(RESTORE_AUTH_REASON, onRestore) },
    )
}
