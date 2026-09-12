package com.stablechannels.app.ui.components

import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.exp
import kotlin.math.sin

class BalanceScaleKinematics(
    val shimmerDelay: Float = 0.25f,
    val shimmerDuration: Float = 0.90f,
    val oscillationPeriod: Float = 2.0f,
    val maxAngleDegrees: Float = 4.8f,
    val settleDuration: Float = 1.2f,
) {
  sealed interface Stage {
    data object Resting : Stage

    data class Shimmer(val progress: Float) : Stage

    data class Oscillating(val angle: Float) : Stage

    data class Settling(val angle: Float) : Stage

    data object Balanced : Stage
  }

  fun evaluate(
      elapsedSinceStart: Float,
      isSyncComplete: Boolean,
      settleElapsed: Float?,
  ): Stage {
    if (isSyncComplete && settleElapsed != null) {
      if (settleElapsed >= settleDuration) {
        return Stage.Balanced
      }
      val progress = (settleElapsed / settleDuration).coerceIn(0f, 1f)
      val envelope = exp(-3.2f * progress)
      val oscillation = cos(3.5f * 2.0f * PI.toFloat() * progress)
      val linearFade = 1.0f - progress
      val angle = maxAngleDegrees * envelope * oscillation * linearFade
      return Stage.Settling(angle)
    }

    if (elapsedSinceStart < shimmerDelay) {
      return Stage.Resting
    }

    val shimmerElapsed = elapsedSinceStart - shimmerDelay
    if (shimmerElapsed < shimmerDuration) {
      val progress = shimmerElapsed / shimmerDuration
      return Stage.Shimmer(progress)
    }

    val oscillationElapsed = shimmerElapsed - shimmerDuration
    val cycle =
        if (oscillationPeriod > 0f) {
          (oscillationElapsed / oscillationPeriod) % 1.0f
        } else {
          0f
        }
    val angle = maxAngleDegrees * sin(cycle * 2.0f * PI.toFloat())
    return Stage.Oscillating(angle)
  }

  companion object {
    fun shimmerSweepRange(progress: Float): Pair<Float, Float> {
      val sweep = progress * 1.8f - 0.4f
      val bandWidth = 0.28f
      return Pair(sweep - bandWidth, sweep + bandWidth)
    }
  }
}
