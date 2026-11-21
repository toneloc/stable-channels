package com.stablechannels.app.ui.settings.seed

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.stablechannels.app.domain.seed.AppLifecyclePhase

/** ON_PAUSE is inactive (the biometric prompt raises it too); only ON_STOP means the app left. */
internal fun seedLifecyclePhase(event: Lifecycle.Event): AppLifecyclePhase? =
    when (event) {
        Lifecycle.Event.ON_RESUME -> AppLifecyclePhase.ACTIVE
        Lifecycle.Event.ON_PAUSE -> AppLifecyclePhase.INACTIVE
        Lifecycle.Event.ON_STOP -> AppLifecyclePhase.BACKGROUND
        else -> null
    }

/** Calls [onPhase] each time the host lifecycle moves to a new [AppLifecyclePhase]. */
@Composable
internal fun ObserveSeedLifecyclePhase(onPhase: (AppLifecyclePhase) -> Unit) {
    val lifecycleOwner = LocalLifecycleOwner.current
    val currentOnPhase by rememberUpdatedState(onPhase)
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            seedLifecyclePhase(event)?.let(currentOnPhase)
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }
}
