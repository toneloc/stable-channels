package com.stablechannels.app.ui.settings

import androidx.compose.runtime.Composable
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.transfer.OnChainSendScreen

/**
 * Wrapper that embeds the existing OnChainSendScreen within the settings navigation.
 * The dismiss callback is a no-op since back navigation is handled by the scaffold.
 */
@Composable
fun OnChainSendSettingsView(appState: AppState) {
    OnChainSendScreen(appState = appState, onDismiss = {})
}
