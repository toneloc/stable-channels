package com.stablechannels.app.ui.home.balancebar

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.runtime.*
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * Pure mathematical transforms deriving visual presentation properties from normalized animation
 * progress (0.0 to 1.0).
 */
object BalanceBarAnimationMath {
    fun thumbScale(progress: Float): Float {
        if (progress <= 0f || progress >= 0.6f) return 1f
        return if (progress < 0.22f) {
            val phase = progress / 0.22f
            1f + (phase * 0.35f)
        } else {
            val phase = (progress - 0.22f) / (0.6f - 0.22f)
            1.35f - (phase * 0.35f)
        }
    }

    fun floodScale(progress: Float): Float {
        if (progress <= 0.05f) return 0.01f
        if (progress >= 0.55f) return 1f
        val phase = (progress - 0.05f) / 0.50f
        return 0.01f + (phase * 0.99f)
    }

    fun floodAlpha(progress: Float): Float {
        if (progress <= 0.05f || progress >= 0.65f) return 0f
        return if (progress < 0.28f) {
            val phase = (progress - 0.05f) / (0.28f - 0.05f)
            phase * 0.55f
        } else {
            val phase = (progress - 0.28f) / (0.65f - 0.28f)
            0.55f * (1f - phase)
        }
    }

    fun settleFraction(
        initialFraction: Float = 0.5f,
        targetFraction: Float,
        progress: Float,
    ): Float? {
        if (progress <= 0f) return null
        if (progress < 0.45f) return initialFraction
        val phase = ((progress - 0.45f) / 0.55f).coerceIn(0f, 1f)
        return initialFraction + (targetFraction - initialFraction) * phase
    }
}

@Composable
fun rememberBalanceBarAnimationCoordinator(): BalanceBarAnimationCoordinator {
    val scope = rememberCoroutineScope()
    return remember(scope) { BalanceBarAnimationCoordinator(scope) }
}

/**
 * Declarative coordinator for Awakening animation. Derives visual presentation properties from a
 * single progress value.
 */
class BalanceBarAnimationCoordinator(private val scope: CoroutineScope) {
    var isAwakening by mutableStateOf(false)
        private set

    val progressAnim = Animatable(0f)
    private var targetFraction: Float = 0.5f
    private var animationJob: Job? = null

    val thumbAwakenScale: Float
        get() = BalanceBarAnimationMath.thumbScale(progressAnim.value)

    val radialFloodScale: Float
        get() = BalanceBarAnimationMath.floodScale(progressAnim.value)

    val radialFloodAlpha: Float
        get() = BalanceBarAnimationMath.floodAlpha(progressAnim.value)

    val settleFraction: Float?
        get() =
            if (isAwakening) {
                BalanceBarAnimationMath.settleFraction(0.5f, targetFraction, progressAnim.value)
            } else null

    fun triggerAwakening(target: Float) {
        animationJob?.cancel()
        targetFraction = target
        isAwakening = true

        animationJob = scope.launch {
            progressAnim.snapTo(0f)
            progressAnim.animateTo(
                targetValue = 1f,
                animationSpec = tween(durationMillis = 1200, easing = LinearEasing),
            )
            isAwakening = false
            progressAnim.snapTo(0f)
        }
    }
}
