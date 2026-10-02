package com.stablechannels.app.ui.home.balancebar

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.EaseOut
import androidx.compose.animation.core.tween
import androidx.compose.runtime.*
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun rememberBalanceBarAnimationCoordinator(): BalanceBarAnimationCoordinator {
    val scope = rememberCoroutineScope()
    return remember(scope) { BalanceBarAnimationCoordinator(scope) }
}

class BalanceBarAnimationCoordinator(private val scope: CoroutineScope) {
    var isAwakening by mutableStateOf(false)
        private set

    var settleFraction by mutableStateOf<Float?>(null)
        private set

    val thumbAwakenScale = Animatable(1f)
    val radialFloodScale = Animatable(0.01f)
    val radialFloodAlpha = Animatable(0f)

    private var awakeningGeneration = 0

    fun triggerAwakening(targetFraction: Float) {
        awakeningGeneration++
        val currentGen = awakeningGeneration

        isAwakening = true
        settleFraction = 0.5f

        scope.launch {
            thumbAwakenScale.snapTo(1f)
            radialFloodScale.snapTo(0.01f)
            radialFloodAlpha.snapTo(0f)

            launch {
                thumbAwakenScale.animateTo(1.35f, tween(220, easing = EaseOut))
                if (awakeningGeneration == currentGen) {
                    thumbAwakenScale.animateTo(1f, tween(400, easing = EaseInOut))
                }
            }
            launch {
                radialFloodScale.animateTo(1f, tween(450, easing = EaseOut))
            }
            launch {
                radialFloodAlpha.animateTo(0.55f, tween(200, easing = EaseOut))
                if (awakeningGeneration == currentGen) {
                    radialFloodAlpha.animateTo(0f, tween(400, easing = EaseInOut))
                }
            }
            launch {
                delay(450)
                if (awakeningGeneration == currentGen) {
                    val settleAnim = Animatable(0.5f)
                    settleAnim.animateTo(targetFraction, tween(650, easing = EaseOut))
                    settleFraction = settleAnim.value
                }
            }
            delay(1200)
            if (awakeningGeneration == currentGen) {
                isAwakening = false
                settleFraction = null
            }
        }
    }
}
