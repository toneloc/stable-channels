package com.stablechannels.app.ui.components

import android.os.Build
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.view.WindowCompat
import com.stablechannels.app.ui.theme.LocalDarkTheme

// Edge-to-edge for a Dialog or ModalBottomSheet window. Android 15+ enforces
// transparent system bars, so the (deprecated) color setters only run on older
// versions, where they are still the only way to clear the bars.
@Composable
internal fun DialogEdgeToEdgeEffect() {
    val view = LocalView.current
    val isDark = LocalDarkTheme.current
    DisposableEffect(view, isDark) {
        var context = view.context
        var dialog: android.app.Dialog? = null
        while (context is android.content.ContextWrapper) {
            if (context is android.app.Dialog) {
                dialog = context
                break
            }
            context = context.baseContext
        }
        // A Compose Dialog's view sits in a DialogWindowProvider, not a Dialog context.
        val window =
            generateSequence(view.parent) { it.parent }
                .filterIsInstance<DialogWindowProvider>()
                .firstOrNull()
                ?.window ?: dialog?.window
        if (window != null) {
            WindowCompat.setDecorFitsSystemWindows(window, false)
            // A dialog window still fits inside the system bars unless told not to, which pushes
            // its bottom under the navigation bar.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                window.attributes =
                    window.attributes.apply {
                        fitInsetsTypes = 0
                        layoutInDisplayCutoutMode =
                            android.view.WindowManager.LayoutParams
                                .LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                    }
            } else {
                window.addFlags(android.view.WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS)
            }
            val insetsController = WindowCompat.getInsetsController(window, view)
            insetsController.isAppearanceLightStatusBars = !isDark
            insetsController.isAppearanceLightNavigationBars = !isDark
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) {
                @Suppress("DEPRECATION")
                window.navigationBarColor = android.graphics.Color.TRANSPARENT
                @Suppress("DEPRECATION")
                window.statusBarColor = android.graphics.Color.TRANSPARENT
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                window.isNavigationBarContrastEnforced = false
            }
            window.setLayout(
                android.view.ViewGroup.LayoutParams.MATCH_PARENT,
                android.view.ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        onDispose {}
    }
}
