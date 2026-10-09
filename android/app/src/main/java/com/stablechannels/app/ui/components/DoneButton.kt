package com.stablechannels.app.ui.components

import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.stablechannels.app.ui.theme.LocalDarkTheme

private val ToolbarLight = Color(0xFFE5E5EA)
private val ToolbarDark = Color(0xFF1C1C1E)

/** Closes a finished flow. Compact and centered, in the same style as [InkButton]. */
@Composable
fun DoneButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    Row(modifier = modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) {
        InkButton(
            onClick = onClick,
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
            minHeight = 40.dp,
            restingRadius = 20.dp,
            pressedRadius = 10.dp,
        ) {
            Text("Done", style = MaterialTheme.typography.bodyMedium)
        }
    }
}

/** Smaller tonal Done for a top toolbar, where the ink button would outrank the screen content. */
@Composable
fun DoneToolbarButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    val interactionSource = remember { MutableInteractionSource() }
    val dark = LocalDarkTheme.current
    Button(
        onClick = onClick,
        modifier = modifier.height(40.dp),
        shape = morphingShape(interactionSource, restingRadius = 20.dp, pressedRadius = 10.dp),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = if (dark) ToolbarDark else ToolbarLight,
                contentColor = MaterialTheme.colorScheme.onSurface,
            ),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
        interactionSource = interactionSource,
    ) {
        Text("Done", style = MaterialTheme.typography.bodyMedium)
    }
}
