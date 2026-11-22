package com.stablechannels.app.ui.settings.seed

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import com.stablechannels.app.ui.theme.LocalDarkTheme

/** Recovery phrase reveal tones; they invert with the system appearance, as on iOS. */
internal object RevealColors {
    /** Filled call to action: white in dark mode, black in light mode. */
    val buttonFill: Color
        @Composable get() = MaterialTheme.colorScheme.onBackground

    val buttonLabel: Color
        @Composable get() = MaterialTheme.colorScheme.background

    val hairline: Color
        @Composable get() = MaterialTheme.colorScheme.onBackground.copy(alpha = 0.1f)

    /** Covers the blurred words until tapped. */
    val veil: Color
        @Composable get() = groupedBackground.copy(alpha = 0.55f)

    // iOS systemGroupedBackground and secondarySystemGroupedBackground; the theme has no match.
    val groupedBackground: Color
        @Composable get() = if (LocalDarkTheme.current) Color(0xFF000000) else Color(0xFFF2F2F7)

    val secondaryGroupedBackground: Color
        @Composable get() = if (LocalDarkTheme.current) Color(0xFF1C1C1E) else Color(0xFFFFFFFF)

    // iOS systemGreen, systemRed and systemOrange.
    val green: Color
        @Composable get() = if (LocalDarkTheme.current) Color(0xFF30D158) else Color(0xFF34C759)

    val red: Color
        @Composable get() = if (LocalDarkTheme.current) Color(0xFFFF453A) else Color(0xFFFF3B30)

    val orange: Color
        @Composable get() = if (LocalDarkTheme.current) Color(0xFFFF9F0A) else Color(0xFFFF9500)

    /** iOS stablePrimary: the Learn more links and illustration accents. */
    val stablePrimary = Color(0xFF00A3E0)
}
