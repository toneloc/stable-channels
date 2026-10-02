package com.stablechannels.app.ui.home.balancebar

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.max

@Composable
fun BalanceBarEmptyTrack(
    height: Dp,
    modifier: Modifier = Modifier,
) {
    val stableFaded = Color(0xFF10B981).copy(alpha = 0.22f)
    val nativeFaded = Color(0xFFF59E0B).copy(alpha = 0.22f)

    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .height(height)
                .clip(RoundedCornerShape(6.dp))
                .border(
                    width = 1.dp,
                    color = MaterialTheme.colorScheme.outline.copy(alpha = 0.12f),
                    shape = RoundedCornerShape(6.dp),
                )
    ) {
        Box(
            modifier =
                Modifier.weight(0.5f)
                    .fillMaxHeight()
                    .background(
                        Brush.horizontalGradient(
                            listOf(stableFaded.copy(alpha = 0.16f), stableFaded)
                        )
                    )
        )
        Spacer(
            modifier =
                Modifier.width(2.dp)
                    .fillMaxHeight()
                    .background(MaterialTheme.colorScheme.background.copy(alpha = 0.5f))
        )
        Box(
            modifier =
                Modifier.weight(0.5f)
                    .fillMaxHeight()
                    .background(
                        Brush.horizontalGradient(
                            listOf(nativeFaded, nativeFaded.copy(alpha = 0.16f))
                        )
                    )
        )
    }
}

@Composable
fun BalanceBarTrack(
    visFrac: Float,
    barHeight: Dp,
    isEmpty: Boolean,
    isAwakening: Boolean,
    stableColor: Color,
    nativeColor: Color,
    modifier: Modifier = Modifier,
) {
    if (isEmpty && !isAwakening) {
        BalanceBarEmptyTrack(height = barHeight, modifier = modifier)
    } else {
        Row(modifier = modifier.fillMaxWidth().height(barHeight).clip(RoundedCornerShape(6.dp))) {
            if (visFrac > 0.005f) {
                Box(
                    Modifier.weight(max(visFrac, 0.01f))
                        .fillMaxHeight()
                        .background(
                            Brush.horizontalGradient(
                                listOf(stableColor.copy(alpha = 0.8f), stableColor)
                            )
                        )
                )
            }
            if ((1f - visFrac) > 0.005f) {
                Box(
                    Modifier.weight(max(1f - visFrac, 0.01f))
                        .fillMaxHeight()
                        .background(
                            Brush.horizontalGradient(
                                listOf(nativeColor, nativeColor.copy(alpha = 0.8f))
                            )
                        )
                )
            }
        }
    }
}

@Composable
fun BalanceBarAwakeningBloom(
    barWidthPx: Float,
    floodScale: Float,
    floodAlpha: Float,
    nativeColor: Color,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    if (floodAlpha > 0f && barWidthPx > 0f) {
        val glowDiameter = with(density) { (barWidthPx * 1.3f * floodScale).toDp() }
        Box(
            modifier =
                modifier
                    .size(glowDiameter)
                    .graphicsLayer { alpha = floodAlpha }
                    .background(
                        brush =
                            Brush.radialGradient(
                                colors =
                                    listOf(
                                        nativeColor.copy(alpha = 0.85f),
                                        Color(0xFFFBBF24).copy(alpha = 0.45f),
                                        Color.Transparent,
                                    )
                            ),
                        shape = CircleShape,
                    )
        )
    }
}

@Composable
fun BalanceBarThumb(
    thumbOffsetDp: Dp,
    thumbDiameter: Dp,
    scale: Float,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier =
            modifier
                .offset(x = thumbOffsetDp)
                .size(thumbDiameter)
                .scale(scale)
                .shadow(4.dp, CircleShape)
                .clip(CircleShape)
                .background(Color.White)
    )
}
