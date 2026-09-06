package com.stablechannels.app.ui.components

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.sin

enum class CurvePattern {
    SIX_PETAL_SPIRAL,
    SPIRAL_SEARCH
}

@Composable
fun CurveProgressIndicator(
    modifier: Modifier = Modifier,
    size: Dp = 48.dp,
    pattern: CurvePattern = CurvePattern.SIX_PETAL_SPIRAL,
    primaryColor: Color = Color(0xFF38BDF8),
    glowColor: Color = Color(0xFF818CF8),
    trackColor: Color = Color(0xFF38BDF8).copy(alpha = 0.12f),
    durationMillis: Int = 4600
) {
    val infiniteTransition = rememberInfiniteTransition(label = "CurveProgressIndicatorTransition")
    val progress by infiniteTransition.animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = durationMillis, easing = LinearEasing),
            repeatMode = RepeatMode.Restart
        ),
        label = "CurveProgressIndicatorProgress"
    )

    val pulseProgress by infiniteTransition.animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 4200, easing = LinearEasing),
            repeatMode = RepeatMode.Restart
        ),
        label = "CurveProgressIndicatorPulse"
    )

    Canvas(modifier = modifier.size(size)) {
        val width = this.size.width
        val height = this.size.height
        val center = Offset(width / 2f, height / 2f)
        val scale = min(width, height) / 100f

        val pulseAngle = pulseProgress * 2f * PI.toFloat()
        val detailScale = 0.52f + ((sin(pulseAngle + 0.55f) + 1f) / 2f) * 0.48f

        val trackSteps = 120
        val trackPath = Path().apply {
            for (step in 0..trackSteps) {
                val u = step.toFloat() / trackSteps.toFloat()
                val pt = calculateCurvePoint(pattern, u, detailScale, center, scale)
                if (step == 0) moveTo(pt.x, pt.y) else lineTo(pt.x, pt.y)
            }
            close()
        }

        drawPath(
            path = trackPath,
            color = trackColor,
            style = Stroke(
                width = 1.2f * scale,
                pathEffect = PathEffect.dashPathEffect(floatArrayOf(3f * scale, 3f * scale), 0f)
            )
        )

        val trailCount = 36
        val trailSpan = if (pattern == CurvePattern.SPIRAL_SEARCH) 0.28f else 0.34f

        for (i in (trailCount - 1) downTo 0) {
            val offsetFrac = i.toFloat() / (trailCount - 1)
            var u = progress - offsetFrac * trailSpan
            if (u < 0f) u += 1f

            val pt = calculateCurvePoint(pattern, u, detailScale, center, scale)
            val intensity = (1f - offsetFrac).pow(0.56f)
            val particleRadius = maxOf(1.2f, (1.0f + (1f - offsetFrac) * 2.8f) * scale)
            val particleColor = lerp(glowColor, primaryColor, 1f - offsetFrac)

            drawCircle(
                color = particleColor.copy(alpha = intensity * 0.85f),
                radius = particleRadius,
                center = pt
            )
        }

        val headPt = calculateCurvePoint(pattern, progress, detailScale, center, scale)

        drawCircle(
            color = primaryColor.copy(alpha = 0.22f),
            radius = 6.5f * scale,
            center = headPt
        )
        drawCircle(
            color = glowColor.copy(alpha = 0.55f),
            radius = 4.0f * scale,
            center = headPt
        )
        drawCircle(
            color = Color.White,
            radius = 2.2f * scale,
            center = headPt
        )
    }
}

@Composable
fun MathCurveLoader(
    modifier: Modifier = Modifier,
    size: Dp = 48.dp,
    pattern: CurvePattern = CurvePattern.SIX_PETAL_SPIRAL,
    primaryColor: Color = Color(0xFF38BDF8),
    glowColor: Color = Color(0xFF818CF8),
    trackColor: Color = Color(0xFF38BDF8).copy(alpha = 0.12f),
    durationMillis: Int = 4600
) {
    CurveProgressIndicator(
        modifier = modifier,
        size = size,
        pattern = pattern,
        primaryColor = primaryColor,
        glowColor = glowColor,
        trackColor = trackColor,
        durationMillis = durationMillis
    )
}

private fun calculateCurvePoint(
    pattern: CurvePattern,
    progress: Float,
    detailScale: Float,
    center: Offset,
    viewportScale: Float
): Offset {
    val t = progress * 2f * PI.toFloat()
    return when (pattern) {
        CurvePattern.SIX_PETAL_SPIRAL -> {
            val d = 3f + detailScale * 0.25f
            val baseX = 5f * cos(t) + d * cos(5f * t)
            val baseY = 5f * sin(t) - d * sin(5f * t)
            val s = (2.2f + detailScale * 0.45f) * 1.85f * viewportScale
            Offset(center.x + baseX * s, center.y + baseY * s)
        }
        CurvePattern.SPIRAL_SEARCH -> {
            val angle = t * 4f
            val radius = (8f + (1f - cos(t)) * (8.5f + detailScale * 2.4f)) * 1.4f * viewportScale
            Offset(center.x + cos(angle) * radius, center.y + sin(angle) * radius)
        }
    }
}
