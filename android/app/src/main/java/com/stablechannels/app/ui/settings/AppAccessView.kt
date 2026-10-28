package com.stablechannels.app.ui.settings

import android.content.Context
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.fragment.app.FragmentActivity
import com.stablechannels.app.services.AppAccessPreferencesManager
import com.stablechannels.app.services.BiometricService
import com.stablechannels.app.ui.components.inkColor
import com.stablechannels.app.ui.components.onInkColor
import kotlinx.coroutines.launch

@Composable
fun AppAccessView() {
    val context = LocalContext.current
    val activity = context as? FragmentActivity
    val scope = rememberCoroutineScope()
    val prefs = context.getSharedPreferences("app_access_prefs", Context.MODE_PRIVATE)

    var appUnlockEnabled by remember {
        mutableStateOf(prefs.getBoolean("app_unlock_enabled", false))
    }
    var paymentConfirmEnabled by remember {
        mutableStateOf(prefs.getBoolean("payment_confirmation_enabled", false))
    }
    var isAuthenticating by remember { mutableStateOf(false) }

    val biometricsAvailable = BiometricService.isAuthenticationAvailable(context)

    // Turning a toggle off needs auth first, so someone holding the unlocked phone can't
    // remove the protection. Turning one on does not.
    fun changeToggle(
        current: Boolean,
        requested: Boolean,
        reason: String,
        save: (Boolean) -> Unit,
    ) {
        if (!AppAccessPreferencesManager.requiresAuthForToggleChange(current, requested)) {
            save(requested)
            return
        }
        if (activity == null || isAuthenticating) return
        isAuthenticating = true
        scope.launch {
            val result = BiometricService.authenticate(activity, reason)
            if (result == BiometricService.AuthResult.SUCCESS) save(requested)
            isAuthenticating = false
        }
    }

    Column(modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        if (!biometricsAvailable) {
            Surface(
                shape = MaterialTheme.shapes.medium,
                tonalElevation = 1.dp,
                color = MaterialTheme.colorScheme.errorContainer,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(
                    text =
                        "Biometric enrollment is required to use these features. Please set up fingerprint, face, or device credentials in your device settings.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onErrorContainer,
                    modifier = Modifier.padding(16.dp),
                )
            }
            Spacer(Modifier.height(20.dp))
        }

        // App Unlock
        Surface(
            shape = MaterialTheme.shapes.medium,
            tonalElevation = 1.dp,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Row(
                modifier = Modifier.padding(16.dp),
                verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = "App Unlock",
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.Medium,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = "Require authentication on app launch and resume",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(
                    checked = appUnlockEnabled,
                    onCheckedChange = { newValue ->
                        if (biometricsAvailable) {
                            changeToggle(appUnlockEnabled, newValue, "Disable App Unlock") {
                                appUnlockEnabled = it
                                prefs.edit().putBoolean("app_unlock_enabled", it).apply()
                            }
                        }
                    },
                    enabled = biometricsAvailable && !isAuthenticating,
                    colors =
                        SwitchDefaults.colors(
                            checkedThumbColor = onInkColor(),
                            checkedTrackColor = inkColor(),
                            uncheckedThumbColor = MaterialTheme.colorScheme.onSurfaceVariant,
                            uncheckedTrackColor = MaterialTheme.colorScheme.surfaceVariant,
                        ),
                    modifier = Modifier.height(24.dp),
                )
            }
        }

        Spacer(Modifier.height(12.dp))

        // Payment Confirmation
        Surface(
            shape = MaterialTheme.shapes.medium,
            tonalElevation = 1.dp,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Row(
                modifier = Modifier.padding(16.dp),
                verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = "Payment Confirmation",
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.Medium,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = "Require authentication before sends (Lightning and on-chain)",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(
                    checked = paymentConfirmEnabled,
                    onCheckedChange = { newValue ->
                        if (biometricsAvailable) {
                            changeToggle(
                                paymentConfirmEnabled,
                                newValue,
                                "Disable Payment Confirmation",
                            ) {
                                paymentConfirmEnabled = it
                                prefs.edit().putBoolean("payment_confirmation_enabled", it).apply()
                            }
                        }
                    },
                    enabled = biometricsAvailable && !isAuthenticating,
                    colors =
                        SwitchDefaults.colors(
                            checkedThumbColor = onInkColor(),
                            checkedTrackColor = inkColor(),
                            uncheckedThumbColor = MaterialTheme.colorScheme.onSurfaceVariant,
                            uncheckedTrackColor = MaterialTheme.colorScheme.surfaceVariant,
                        ),
                    modifier = Modifier.height(24.dp),
                )
            }
        }
    }
}
