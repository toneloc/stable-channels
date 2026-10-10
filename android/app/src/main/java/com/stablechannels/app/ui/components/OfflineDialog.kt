package com.stablechannels.app.ui.components

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.WifiOff
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TooltipBox
import androidx.compose.material3.TooltipDefaults
import androidx.compose.material3.rememberTooltipState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import com.stablechannels.app.ui.theme.LocalDarkTheme
import com.stablechannels.app.ui.theme.LocalSemanticColors
import kotlinx.coroutines.launch

@Composable
fun OfflineDialog(onTryAgain: () -> Unit, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss) { OfflineCard(onTryAgain, onDismiss) }
}

@Composable
private fun OfflineCard(onTryAgain: () -> Unit, onContinueOffline: () -> Unit) {
    val colors = LocalSemanticColors.current
    val scope = rememberCoroutineScope()
    var checking by remember { mutableStateOf(false) }

    Surface(
        shape = RoundedCornerShape(28.dp),
        color = if (LocalDarkTheme.current) Color(0xFF1C1C1E) else Color.White,
        tonalElevation = 6.dp,
        shadowElevation = 12.dp,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                PulsingWifiOff(MaterialTheme.colorScheme.secondary)
                Text(
                    "You're offline",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                )
            }
            Spacer(Modifier.height(8.dp))
            Text(
                "Stable Channels can't reach the network.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(20.dp))

            // Reassurance box
            Column(
                modifier =
                    Modifier.fillMaxWidth()
                        .background(colors.success.copy(alpha = 0.10f), RoundedCornerShape(14.dp))
                        .padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    "Your wallet and funds are safe, but payments are unavailable until you reconnect.",
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.Medium,
                    textAlign = TextAlign.Center,
                )
            }
            Spacer(Modifier.height(24.dp))

            val buttonColors =
                ButtonDefaults.outlinedButtonColors(
                    contentColor = MaterialTheme.colorScheme.secondary
                )
            val buttonPadding = PaddingValues(horizontal = 8.dp, vertical = 8.dp)

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedButton(
                    onClick = onContinueOffline,
                    shape = RoundedCornerShape(12.dp),
                    border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                    colors = buttonColors,
                    contentPadding = buttonPadding,
                    modifier = Modifier.weight(1f),
                ) {
                    Text("Use offline", maxLines = 1)
                }
                OutlinedButton(
                    onClick = {
                        scope.launch {
                            checking = true
                            onTryAgain()
                            kotlinx.coroutines.delay(850)
                            checking = false
                        }
                    },
                    enabled = !checking,
                    shape = RoundedCornerShape(12.dp),
                    border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                    colors = buttonColors,
                    contentPadding = buttonPadding,
                    modifier = Modifier.weight(1f),
                ) {
                    if (checking) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp,
                            color = MaterialTheme.colorScheme.secondary,
                        )
                    } else {
                        Text("Try again", fontWeight = FontWeight.SemiBold, maxLines = 1)
                    }
                }
            }
        }
    }
}

@Composable
private fun PulsingWifiOff(tint: Color) {
    val pulse by
        rememberInfiniteTransition(label = "offlinePulse")
            .animateFloat(
                initialValue = 1f,
                targetValue = 1.2f,
                animationSpec = infiniteRepeatable(tween(1100), RepeatMode.Reverse),
                label = "scale",
            )
    Box(contentAlignment = Alignment.Center) {
        Box(Modifier.size(40.dp).scale(pulse).background(tint.copy(alpha = 0.12f), CircleShape))
        Box(
            Modifier.size(32.dp).background(tint.copy(alpha = 0.20f), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                Icons.Default.WifiOff,
                contentDescription = null,
                tint = tint,
                modifier = Modifier.size(18.dp),
            )
        }
    }
}

/**
 * "No Internet Connection" capsule. When [info] is set, tapping it shows a tooltip explaining
 * what's unavailable on that screen.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OfflineBadge(modifier: Modifier = Modifier, info: String? = null, tooltipMargin: Dp = 18.dp) {
    val red = MaterialTheme.colorScheme.error
    val tooltipState = rememberTooltipState(isPersistent = true)
    val scope = rememberCoroutineScope()
    val maxTooltipWidth = LocalConfiguration.current.screenWidthDp.dp - tooltipMargin * 2

    val capsule: @Composable () -> Unit = {
        Row(
            modifier =
                modifier
                    .clip(RoundedCornerShape(6.dp))
                    .then(
                        if (info != null)
                            Modifier.clickable { scope.launch { tooltipState.show() } }
                        else Modifier
                    )
                    .border(BorderStroke(1.dp, red.copy(alpha = 0.35f)), RoundedCornerShape(6.dp))
                    .background(red.copy(alpha = 0.08f), RoundedCornerShape(6.dp))
                    .padding(horizontal = 10.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                "No Internet Connection",
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.Bold,
                color = red,
            )
            if (info != null) {
                Icon(
                    Icons.Default.Info,
                    contentDescription = "More info",
                    tint = red.copy(alpha = 0.7f),
                    modifier = Modifier.size(13.dp),
                )
            }
        }
    }

    if (info == null) {
        capsule()
    } else {
        TooltipBox(
            positionProvider = TooltipDefaults.rememberPlainTooltipPositionProvider(),
            tooltip = {
                Surface(
                    shape = RoundedCornerShape(8.dp),
                    color = MaterialTheme.colorScheme.inverseSurface,
                    contentColor = MaterialTheme.colorScheme.inverseOnSurface,
                    shadowElevation = 4.dp,
                    modifier = Modifier.widthIn(max = maxTooltipWidth),
                ) {
                    Text(
                        info,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                    )
                }
            },
            state = tooltipState,
        ) {
            capsule()
        }
    }
}

@Preview
@Composable
private fun OfflineDialogPreview() {
    OfflineCard(onTryAgain = {}, onContinueOffline = {})
}
