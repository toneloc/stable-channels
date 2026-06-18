package com.stablechannels.app.ui.home

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.tween
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp

/**
 * A text composable that animates each digit independently,
 * like a mechanical counter or alarm clock.
 * Non-digit characters (commas, dots, spaces) stay static.
 */
@Composable
fun RollingDigitText(
    text: String,
    style: TextStyle,
    color: Color = Color.Unspecified,
    modifier: Modifier = Modifier
) {
    val textMeasurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val measuredHeight = textMeasurer.measure("0", style).size.height
    val heightDp = with(density) { measuredHeight.toDp() }

    Row(modifier = modifier) {
        text.forEach { char ->
            if (char.isDigit()) {
                Box(modifier = Modifier.height(heightDp)) {
                    AnimatedContent(
                        targetState = char,
                        transitionSpec = {
                            slideInVertically(animationSpec = tween(400)) { height -> height } togetherWith
                                slideOutVertically(animationSpec = tween(400)) { height -> -height } using
                                SizeTransform(clip = true)
                        },
                        label = "digit-${char.hashCode()}"
                    ) { targetChar ->
                        Text(
                            text = targetChar.toString(),
                            style = style,
                            color = color
                        )
                    }
                }
            } else {
                // Non-digit characters (commas, dots, spaces) stay static
                Text(
                    text = char.toString(),
                    style = style,
                    color = color
                )
            }
        }
    }
}
