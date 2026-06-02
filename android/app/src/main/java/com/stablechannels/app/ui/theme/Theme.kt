package com.stablechannels.app.ui.theme

import android.content.Context
import android.content.SharedPreferences
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

// ─── Branded Color Palette ───────────────────────────────────────────────────

// Primary: Green (stability/USD)
private val PrimaryLight = Color(0xFF10B981)
private val OnPrimaryLight = Color(0xFFFFFFFF)
private val PrimaryContainerLight = Color(0xFFD1FAE5)
private val OnPrimaryContainerLight = Color(0xFF064E3B)

private val PrimaryDark = Color(0xFF34D399)
private val OnPrimaryDark = Color(0xFF064E3B)
private val PrimaryContainerDark = Color(0xFF065F46)
private val OnPrimaryContainerDark = Color(0xFFD1FAE5)

// Secondary: Orange (BTC/native)
private val SecondaryLight = Color(0xFFF59E0B)
private val OnSecondaryLight = Color(0xFFFFFFFF)
private val SecondaryContainerLight = Color(0xFFFEF3C7)
private val OnSecondaryContainerLight = Color(0xFF78350F)

private val SecondaryDark = Color(0xFFFBBF24)
private val OnSecondaryDark = Color(0xFF78350F)
private val SecondaryContainerDark = Color(0xFF92400E)
private val OnSecondaryContainerDark = Color(0xFFFEF3C7)

// Tertiary: Blue (info)
private val TertiaryLight = Color(0xFF3B82F6)
private val OnTertiaryLight = Color(0xFFFFFFFF)
private val TertiaryContainerLight = Color(0xFFDBEAFE)
private val OnTertiaryContainerLight = Color(0xFF1E3A5F)

private val TertiaryDark = Color(0xFF60A5FA)
private val OnTertiaryDark = Color(0xFF1E3A5F)
private val TertiaryContainerDark = Color(0xFF1E40AF)
private val OnTertiaryContainerDark = Color(0xFFDBEAFE)

// Error
private val ErrorLight = Color(0xFFDC2626)
private val OnErrorLight = Color(0xFFFFFFFF)
private val ErrorContainerLight = Color(0xFFFEE2E2)
private val OnErrorContainerLight = Color(0xFF7F1D1D)

private val ErrorDark = Color(0xFFF87171)
private val OnErrorDark = Color(0xFF7F1D1D)
private val ErrorContainerDark = Color(0xFF991B1B)
private val OnErrorContainerDark = Color(0xFFFEE2E2)

// Surfaces - Light
private val BackgroundLight = Color(0xFFFAFAFA)
private val OnBackgroundLight = Color(0xFF1A1A1A)
private val SurfaceLight = Color(0xFFFFFFFF)
private val OnSurfaceLight = Color(0xFF1A1A1A)
private val SurfaceVariantLight = Color(0xFFF3F4F6)
private val OnSurfaceVariantLight = Color(0xFF4B5563)
private val OutlineLight = Color(0xFFD1D5DB)
private val OutlineVariantLight = Color(0xFFE5E7EB)

// Surfaces - Dark
private val BackgroundDark = Color(0xFF111111)
private val OnBackgroundDark = Color(0xFFF3F4F6)
private val SurfaceDark = Color(0xFF1A1A1A)
private val OnSurfaceDark = Color(0xFFF3F4F6)
private val SurfaceVariantDark = Color(0xFF2D2D2D)
private val OnSurfaceVariantDark = Color(0xFFD1D5DB)
private val OutlineDark = Color(0xFF4B5563)
private val OutlineVariantDark = Color(0xFF374151)

// ─── Color Schemes ───────────────────────────────────────────────────────────

private val LightColorScheme: ColorScheme = lightColorScheme(
    primary = PrimaryLight,
    onPrimary = OnPrimaryLight,
    primaryContainer = PrimaryContainerLight,
    onPrimaryContainer = OnPrimaryContainerLight,
    secondary = SecondaryLight,
    onSecondary = OnSecondaryLight,
    secondaryContainer = SecondaryContainerLight,
    onSecondaryContainer = OnSecondaryContainerLight,
    tertiary = TertiaryLight,
    onTertiary = OnTertiaryLight,
    tertiaryContainer = TertiaryContainerLight,
    onTertiaryContainer = OnTertiaryContainerLight,
    error = ErrorLight,
    onError = OnErrorLight,
    errorContainer = ErrorContainerLight,
    onErrorContainer = OnErrorContainerLight,
    background = BackgroundLight,
    onBackground = OnBackgroundLight,
    surface = SurfaceLight,
    onSurface = OnSurfaceLight,
    surfaceVariant = SurfaceVariantLight,
    onSurfaceVariant = OnSurfaceVariantLight,
    outline = OutlineLight,
    outlineVariant = OutlineVariantLight,
)

private val DarkColorScheme: ColorScheme = darkColorScheme(
    primary = PrimaryDark,
    onPrimary = OnPrimaryDark,
    primaryContainer = PrimaryContainerDark,
    onPrimaryContainer = OnPrimaryContainerDark,
    secondary = SecondaryDark,
    onSecondary = OnSecondaryDark,
    secondaryContainer = SecondaryContainerDark,
    onSecondaryContainer = OnSecondaryContainerDark,
    tertiary = TertiaryDark,
    onTertiary = OnTertiaryDark,
    tertiaryContainer = TertiaryContainerDark,
    onTertiaryContainer = OnTertiaryContainerDark,
    error = ErrorDark,
    onError = OnErrorDark,
    errorContainer = ErrorContainerDark,
    onErrorContainer = OnErrorContainerDark,
    background = BackgroundDark,
    onBackground = OnBackgroundDark,
    surface = SurfaceDark,
    onSurface = OnSurfaceDark,
    surfaceVariant = SurfaceVariantDark,
    onSurfaceVariant = OnSurfaceVariantDark,
    outline = OutlineDark,
    outlineVariant = OutlineVariantDark,
)

// ─── Semantic Colors ─────────────────────────────────────────────────────────

@Immutable
data class SemanticColors(
    val success: Color,
    val warning: Color,
    val error: Color,
    val btcNative: Color,
    val usdStable: Color,
    val info: Color,
)

private val LightSemanticColors = SemanticColors(
    success = Color(0xFF059669),    // Emerald-600
    warning = Color(0xFFD97706),    // Amber-600
    error = Color(0xFFDC2626),      // Red-600
    btcNative = Color(0xFFF59E0B),  // Amber-500 (orange)
    usdStable = Color(0xFF10B981),  // Emerald-500 (distinct from success)
    info = Color(0xFF2563EB),       // Blue-600
)

private val DarkSemanticColors = SemanticColors(
    success = Color(0xFF34D399),    // Emerald-400
    warning = Color(0xFFFBBF24),    // Amber-400
    error = Color(0xFFF87171),      // Red-400
    btcNative = Color(0xFFFBBF24),  // Amber-400 (orange)
    usdStable = Color(0xFF6EE7B7),  // Emerald-300 (distinct from success)
    info = Color(0xFF60A5FA),       // Blue-400
)

val LocalSemanticColors = staticCompositionLocalOf {
    LightSemanticColors
}

// ─── Theme Preference ────────────────────────────────────────────────────────

enum class ThemePreference(val label: String) {
    LIGHT("Light"),
    DARK("Dark"),
    SYSTEM("System");

    companion object {
        private const val PREFS_NAME = "theme_prefs"
        private const val KEY_THEME = "theme_preference"

        fun load(context: Context): ThemePreference {
            val prefs: SharedPreferences =
                context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            val stored = prefs.getString(KEY_THEME, null)
            return entries.find { it.name == stored } ?: SYSTEM
        }

        fun save(context: Context, preference: ThemePreference) {
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                .edit()
                .putString(KEY_THEME, preference.name)
                .apply()
        }
    }
}

// ─── Theme Composable ────────────────────────────────────────────────────────

@Composable
private fun rememberThemePreference(): ThemePreference {
    val context = LocalContext.current
    var preference by remember { mutableStateOf(ThemePreference.load(context)) }

    DisposableEffect(context) {
        val prefs = context.getSharedPreferences("theme_prefs", Context.MODE_PRIVATE)
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key == "theme_preference") {
                preference = ThemePreference.load(context)
            }
        }
        prefs.registerOnSharedPreferenceChangeListener(listener)
        onDispose {
            prefs.unregisterOnSharedPreferenceChangeListener(listener)
        }
    }

    return preference
}

@Composable
fun StableChannelsTheme(
    content: @Composable () -> Unit
) {
    val themePreference = rememberThemePreference()

    val darkTheme = when (themePreference) {
        ThemePreference.LIGHT -> false
        ThemePreference.DARK -> true
        ThemePreference.SYSTEM -> isSystemInDarkTheme()
    }

    val colorScheme = if (darkTheme) DarkColorScheme else LightColorScheme
    val semanticColors = if (darkTheme) DarkSemanticColors else LightSemanticColors

    CompositionLocalProvider(LocalSemanticColors provides semanticColors) {
        MaterialTheme(
            colorScheme = colorScheme,
            content = content
        )
    }
}
