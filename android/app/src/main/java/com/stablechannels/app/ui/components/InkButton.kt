package com.stablechannels.app.ui.components

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.stablechannels.app.ui.theme.LocalDarkTheme

// Neutral "ink" palette. Status colours stay on icons and amounts; actions stay neutral.
private val InkLight = Color(0xFF1C1C1E)
private val OnInkLight = Color(0xFFFFFFFF)
private val InkDark = Color(0xFFE5E5EA)
private val OnInkDark = Color(0xFF000000)

// Slightly bouncy spring, like the shape morph in Material 3 Expressive.
private val MorphSpring = spring<Dp>(dampingRatio = 0.6f, stiffness = 800f)

@Composable
internal fun morphingShape(
    interactionSource: MutableInteractionSource,
    restingRadius: Dp,
    pressedRadius: Dp,
): RoundedCornerShape {
    val pressed by interactionSource.collectIsPressedAsState()
    val radius by
        animateDpAsState(
            targetValue = if (pressed) pressedRadius else restingRadius,
            animationSpec = MorphSpring,
            label = "buttonCornerRadius",
        )
    return RoundedCornerShape(radius)
}

/** Neutral accent for selected controls such as radio buttons and switches. */
@Composable internal fun inkColor(): Color = if (LocalDarkTheme.current) InkDark else InkLight

@Composable internal fun onInkColor(): Color = if (LocalDarkTheme.current) OnInkDark else OnInkLight

@Composable
internal fun inkButtonColors(): ButtonColors {
    val dark = LocalDarkTheme.current
    return ButtonDefaults.buttonColors(
        containerColor = if (dark) InkDark else InkLight,
        contentColor = if (dark) OnInkDark else OnInkLight,
    )
}

/**
 * The app's filled action button: neutral ink colour, pill corners that squeeze while pressed. Pass
 * [colors] to override, e.g. for a status colour.
 */
@Composable
fun InkButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: ButtonColors = inkButtonColors(),
    contentPadding: PaddingValues = PaddingValues(horizontal = 20.dp),
    minHeight: Dp = 44.dp,
    restingRadius: Dp = 22.dp,
    pressedRadius: Dp = 10.dp,
    content: @Composable RowScope.() -> Unit,
) {
    val interactionSource = remember { MutableInteractionSource() }
    Button(
        onClick = onClick,
        modifier = modifier.heightIn(min = minHeight),
        enabled = enabled,
        shape = morphingShape(interactionSource, restingRadius, pressedRadius),
        colors = colors,
        contentPadding = contentPadding,
        interactionSource = interactionSource,
        content = content,
    )
}
