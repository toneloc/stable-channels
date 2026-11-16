package com.stablechannels.app.ui.settings.seed

import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Security
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.DefaultShadowColor
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private val VaultShape =
    RoundedCornerShape(topStart = 48.dp, topEnd = 48.dp, bottomEnd = 6.dp, bottomStart = 6.dp)
private val FloorShape = RoundedCornerShape(12.dp)
private val KeyholeWhite = Color.White.copy(alpha = 0.85f)

/** Decorative vault artwork on the quiz intro, drawn to match the iOS illustration. */
@Composable
internal fun RevealQuizPortalIllustration(modifier: Modifier = Modifier) {
    val orange = RevealColors.orange
    val primary = RevealColors.stablePrimary
    // Compose follows the system animator scale, so the pulse stops when animations are off.
    val pulse by
        rememberInfiniteTransition(label = "portalPulse")
            .animateFloat(
                initialValue = 0.95f,
                targetValue = 1.05f,
                animationSpec =
                    infiniteRepeatable(tween(3200, easing = EaseInOut), RepeatMode.Reverse),
                label = "portalPulseScale",
            )

    Box(
        modifier.fillMaxWidth().height(200.dp).clearAndSetSemantics {},
        contentAlignment = Alignment.Center,
    ) {
        // Halo: solid inside 25 dp, then fading out to 120 dp, as the iOS radial gradient.
        Canvas(
            Modifier.requiredSize(240.dp).graphicsLayer {
                scaleX = pulse
                scaleY = pulse
            }
        ) {
            drawCircle(
                Brush.radialGradient(
                    0.21f to orange.copy(alpha = 0.32f),
                    0.6f to primary.copy(alpha = 0.18f),
                    1f to Color.Transparent,
                    radius = 120.dp.toPx(),
                )
            )
        }
        Canvas(Modifier.size(160.dp)) {
            val beam =
                Brush.horizontalGradient(
                    listOf(
                        orange.copy(alpha = 0.35f),
                        primary.copy(alpha = 0.15f),
                        Color.Transparent,
                    ),
                    startX = center.x,
                    endX = size.width,
                )
            val thickness = 1.5.dp.toPx()
            repeat(12) { i ->
                rotate(i * 30f) {
                    drawRect(
                        beam,
                        Offset(0f, center.y - thickness / 2),
                        Size(size.width, thickness),
                    )
                }
            }
        }
        Vault(orange, primary)
        Badge(Icons.Filled.Security, 15.dp, primary.copy(alpha = 0.85f), -64.dp, -44.dp)
        Badge(Icons.Filled.Key, 14.dp, orange.copy(alpha = 0.85f), 64.dp, -36.dp)
        Badge(
            Icons.Filled.AutoAwesome,
            13.dp,
            MaterialTheme.colorScheme.onSurface.copy(alpha = 0.75f),
            -54.dp,
            32.dp,
        )
        Badge(Icons.Filled.GridView, 12.dp, primary.copy(alpha = 0.75f), 58.dp, 28.dp)
        Box(
            Modifier.offset(y = 64.dp)
                .size(160.dp, 14.dp)
                .shadowBelow(4.dp, 8.dp, FloorShape, ambient = Color.Black.copy(alpha = 0.4f))
                .background(
                    Brush.verticalGradient(listOf(Color(0xFF2E2E2E), Color(0xFF141414))),
                    FloorShape,
                )
                .border(1.dp, Color.White.copy(alpha = 0.12f), FloorShape)
        )
    }
}

@Composable
private fun Vault(orange: Color, primary: Color) {
    val glow = orange.copy(alpha = 0.25f)
    Box(
        Modifier.offset(y = (-10).dp)
            .size(104.dp, 140.dp)
            .shadowBelow(6.dp, 14.dp, VaultShape, ambient = glow, spot = glow)
            .background(
                Brush.verticalGradient(listOf(Color(0xFF1F2433), Color(0xFF0F141F))),
                VaultShape,
            )
            .border(
                2.5.dp,
                Brush.linearGradient(listOf(orange, orange.copy(alpha = 0.7f), primary)),
                VaultShape,
            ),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            Modifier.offset(y = (-4).dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                Modifier.size(42.dp).background(orange.copy(alpha = 0.18f), CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Text("₿", color = orange, fontSize = 26.sp, fontWeight = FontWeight.Bold)
            }
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Box(Modifier.size(10.dp).background(KeyholeWhite, CircleShape))
                Box(
                    Modifier.size(6.dp, 12.dp)
                        .background(
                            KeyholeWhite,
                            RoundedCornerShape(bottomStart = 2.dp, bottomEnd = 2.dp),
                        )
                )
            }
        }
    }
}

@Composable
private fun Badge(icon: ImageVector, size: Dp, tint: Color, x: Dp, y: Dp) {
    Icon(icon, contentDescription = null, modifier = Modifier.offset(x, y).size(size), tint = tint)
}

/**
 * A shadow cast [y] below the content, like the iOS shadow offset: the outer layer moves down and
 * casts the shadow, the inner one moves the content back.
 */
private fun Modifier.shadowBelow(
    y: Dp,
    elevation: Dp,
    shape: Shape,
    ambient: Color,
    spot: Color = DefaultShadowColor,
): Modifier = graphicsLayer {
    translationY = y.toPx()
    shadowElevation = elevation.toPx()
    this.shape = shape
    ambientShadowColor = ambient
    spotShadowColor = spot
}
    .graphicsLayer { translationY = -y.toPx() }
