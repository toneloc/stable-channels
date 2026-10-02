package com.stablechannels.app.ui.home.balancebar

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.services.StabilizationPolicy

@Composable
fun BalanceBarHeader(
    usdPct: Int,
    btcPct: Int,
    atSellLimit: Boolean,
    maxSellUSD: Double,
    showConversion: Boolean,
    showDepositPrompt: Boolean,
    stableColor: Color,
    nativeColor: Color,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val textMeasurer = rememberTextMeasurer()

    val conversionAlpha by
        animateFloatAsState(
            targetValue = if (showConversion || showDepositPrompt) 1f else 0f,
            animationSpec = tween(150),
            label = "conversionAlpha",
        )

    val headerHeight by
        animateDpAsState(
            targetValue = if (atSellLimit) 34.dp else 24.dp,
            animationSpec = tween(150),
            label = "headerHeight",
        )

    val pillTextStyle =
        MaterialTheme.typography.labelSmall.copy(
            fontSize = 11.sp,
            fontWeight = FontWeight.Bold,
            fontFeatureSettings = "tnum",
        )

    val metrics =
        remember(usdPct, btcPct, density) {
            SliderConversionMetrics.calculate(
                usdPct = usdPct,
                btcPct = btcPct,
                density = density,
                measureWidthPx = { textMeasurer.measure(it, pillTextStyle).size.width },
            )
        }

    Box(
        modifier = modifier.fillMaxWidth().height(headerHeight),
        contentAlignment = Alignment.Center,
    ) {
        if (conversionAlpha > 0f) {
            Row(
                modifier =
                    Modifier.graphicsLayer { alpha = conversionAlpha }
                        .background(
                            color = MaterialTheme.colorScheme.surfaceVariant,
                            shape = RoundedCornerShape(12.dp),
                        )
                        .padding(
                            horizontal = 10.dp,
                            vertical = if (atSellLimit) 2.dp else 4.dp,
                        ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.Center,
            ) {
                if (showDepositPrompt) {
                    Icon(
                        Icons.Default.ArrowDownward,
                        contentDescription = null,
                        tint = stableColor,
                        modifier = Modifier.size(12.dp),
                    )
                    Spacer(Modifier.width(4.dp))
                    Text(
                        text = "Deposit to balance channel",
                        style = pillTextStyle.copy(color = MaterialTheme.colorScheme.onSurface),
                        maxLines = 1,
                    )
                } else if (atSellLimit) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.Center,
                    ) {
                        Text(
                            text =
                                StabilizationPolicy.maximumMessage(
                                    (maxSellUSD * 100 + 1e-7).toLong()
                                ),
                            style =
                                pillTextStyle.copy(
                                    color = MaterialTheme.colorScheme.error,
                                    fontSize = 10.sp,
                                ),
                            maxLines = 1,
                        )
                        Text(
                            text = "Keeps a small BTC reserve in the channel.",
                            style =
                                pillTextStyle.copy(
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    fontSize = 9.sp,
                                    fontWeight = FontWeight.Normal,
                                ),
                            maxLines = 1,
                        )
                    }
                } else {
                    Text(
                        text = "$usdPct% USD",
                        style = pillTextStyle.copy(color = stableColor),
                        textAlign = TextAlign.End,
                        modifier = Modifier.width(metrics.sideWidthDp),
                        maxLines = 1,
                    )
                    Text(
                        text = " · ",
                        style =
                            pillTextStyle.copy(color = MaterialTheme.colorScheme.onSurfaceVariant),
                        maxLines = 1,
                    )
                    Text(
                        text = "$btcPct% BTC",
                        style = pillTextStyle.copy(color = nativeColor),
                        textAlign = TextAlign.Start,
                        modifier = Modifier.width(metrics.sideWidthDp),
                        maxLines = 1,
                    )
                }
            }
        }
    }
}
