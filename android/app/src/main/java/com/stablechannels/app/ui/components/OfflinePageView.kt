package com.stablechannels.app.ui.components

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.R
import com.stablechannels.app.util.OfflineMessages
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Wi-Fi warning icon with centered exclamation mark. */
@Composable
fun WifiExclamationmarkIcon(
    modifier: Modifier = Modifier,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.85f),
) {
    Icon(
        painter = painterResource(R.drawable.ic_wifi_exclamation),
        contentDescription = "No Internet Connection",
        tint = color,
        modifier = modifier.size(width = 54.dp, height = 46.dp),
    )
}

/** Full-screen offline presentation. */
@Composable
fun OfflinePageView(
    isRetrying: Boolean = false,
    onRetry: () -> Unit,
    onGoToHome: (() -> Unit)? = null,
) {
    val scope = rememberCoroutineScope()
    var isSpinning by remember { mutableStateOf(false) }

    val iconScale = remember { Animatable(0.75f) }
    val iconOpacity = remember { Animatable(0f) }

    LaunchedEffect(Unit) {
        launch {
            iconScale.animateTo(
                targetValue = 1.0f,
                animationSpec =
                    spring(
                        dampingRatio = 0.62f,
                        stiffness = Spring.StiffnessMediumLow,
                    ),
            )
        }
        launch {
            iconOpacity.animateTo(
                targetValue = 1.0f,
                animationSpec = tween(durationMillis = 350),
            )
        }
    }

    LaunchedEffect(isRetrying) {
        isSpinning = isRetrying
    }

    val infiniteTransition = rememberInfiniteTransition(label = "spinTransition")
    val spinAngle by
        infiniteTransition.animateFloat(
            initialValue = 0f,
            targetValue = 360f,
            animationSpec =
                infiniteRepeatable(
                    animation = tween(durationMillis = 850, easing = LinearEasing),
                    repeatMode = RepeatMode.Restart,
                ),
            label = "spinAngle",
        )

    fun triggerRetry() {
        if (isRetrying || isSpinning) return
        isSpinning = true
        scope.launch {
            val minSpin = launch { delay(850) }
            onRetry()
            minSpin.join()
            isSpinning = false
        }
    }

    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Column(
            modifier = Modifier.fillMaxSize().padding(horizontal = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(modifier = Modifier.weight(1f))

            Box(
                contentAlignment = Alignment.Center,
                modifier = Modifier.scale(iconScale.value).alpha(iconOpacity.value),
            ) {
                WifiExclamationmarkIcon()
            }

            Spacer(modifier = Modifier.height(20.dp))

            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    text = OfflineMessages.TITLE,
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.onBackground,
                    textAlign = TextAlign.Center,
                )

                Text(
                    text = OfflineMessages.BODY,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                    lineHeight = 20.sp,
                    modifier = Modifier.padding(horizontal = 16.dp),
                )
            }

            Spacer(modifier = Modifier.height(24.dp))

            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Surface(
                    shape = CircleShape,
                    border =
                        BorderStroke(1.dp, MaterialTheme.colorScheme.onSurface.copy(alpha = 0.22f)),
                    color = Color.Transparent,
                    modifier =
                        Modifier.clip(CircleShape)
                            .clickable(enabled = !isSpinning && !isRetrying) {
                                triggerRetry()
                            }
                            .alpha(if (isSpinning || isRetrying) 0.65f else 1.0f),
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(
                            Icons.Default.Refresh,
                            contentDescription = null,
                            modifier =
                                Modifier.size(14.dp)
                                    .rotate(if (isSpinning || isRetrying) spinAngle else 0f),
                            tint = MaterialTheme.colorScheme.onSurface,
                        )
                        Text(
                            text = "Try again",
                            style = MaterialTheme.typography.bodyMedium,
                            fontWeight = FontWeight.Medium,
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                    }
                }

                if (onGoToHome != null) {
                    TextButton(
                        onClick = onGoToHome,
                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                    ) {
                        Text(
                            text = "Go to Home",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.weight(1f))
        }
    }
}

/** Compact pill badge indicating offline state. */
@Composable
fun OfflineBadgeView(
    modifier: Modifier = Modifier,
    subtitle: String? = OfflineMessages.CHECK_NETWORK,
) {
    val red = Color(0xFFEF4444)

    val infiniteTransition = rememberInfiniteTransition(label = "offlinePulse")
    val pulseAlpha by
        infiniteTransition.animateFloat(
            initialValue = 1f,
            targetValue = 0.25f,
            animationSpec =
                infiniteRepeatable(
                    animation = tween(durationMillis = 750),
                    repeatMode = RepeatMode.Reverse,
                ),
            label = "pulseAlpha",
        )

    Surface(
        shape = RoundedCornerShape(6.dp),
        color = red.copy(alpha = 0.08f),
        border = BorderStroke(1.dp, red.copy(alpha = 0.35f)),
        modifier = modifier,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 3.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(1.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                Icon(
                    painter = painterResource(R.drawable.ic_wifi_slash),
                    contentDescription = null,
                    tint = red,
                    modifier = Modifier.size(11.dp).alpha(pulseAlpha),
                )
                Text(
                    text = OfflineMessages.TITLE,
                    style = MaterialTheme.typography.labelSmall.copy(fontSize = 11.sp),
                    fontWeight = FontWeight.Bold,
                    color = red,
                )
                Box(modifier = Modifier.size(4.dp).alpha(pulseAlpha).background(red, CircleShape))
            }
            if (!subtitle.isNullOrBlank()) {
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.labelSmall.copy(fontSize = 9.sp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                    maxLines = 1,
                )
            }
        }
    }
}

@Composable
fun OfflineBadge(
    modifier: Modifier = Modifier,
    info: String? = OfflineMessages.CHECK_NETWORK,
    tooltipMargin: Dp = 18.dp,
) {
    OfflineBadgeView(modifier = modifier, subtitle = info)
}
