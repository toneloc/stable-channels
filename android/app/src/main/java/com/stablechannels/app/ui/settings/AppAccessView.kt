package com.stablechannels.app.ui.settings

import android.content.Context
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.biometric.BiometricManager

@Composable
fun AppAccessView() {
    val context = LocalContext.current
    val prefs = context.getSharedPreferences("app_access_prefs", Context.MODE_PRIVATE)

    var appUnlockEnabled by remember {
        mutableStateOf(prefs.getBoolean("app_unlock_enabled", false))
    }
    var paymentConfirmEnabled by remember {
        mutableStateOf(prefs.getBoolean("payment_confirmation_enabled", false))
    }

    val biometricManager = BiometricManager.from(context)
    val biometricsAvailable = biometricManager.canAuthenticate(
        BiometricManager.Authenticators.BIOMETRIC_STRONG or
                BiometricManager.Authenticators.DEVICE_CREDENTIAL
    ) == BiometricManager.BIOMETRIC_SUCCESS

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        if (!biometricsAvailable) {
            Surface(
                shape = MaterialTheme.shapes.medium,
                tonalElevation = 1.dp,
                color = MaterialTheme.colorScheme.errorContainer,
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    text = "Biometric enrollment is required to use these features. Please set up fingerprint, face, or device credentials in your device settings.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onErrorContainer,
                    modifier = Modifier.padding(16.dp)
                )
            }
            Spacer(Modifier.height(20.dp))
        }

        // App Unlock
        Surface(
            shape = MaterialTheme.shapes.medium,
            tonalElevation = 1.dp,
            modifier = Modifier.fillMaxWidth()
        ) {
            Row(
                modifier = Modifier.padding(16.dp),
                verticalAlignment = androidx.compose.ui.Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = "App Unlock",
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.Medium
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = "Require authentication on app launch and resume",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Switch(
                    checked = appUnlockEnabled,
                    onCheckedChange = { newValue ->
                        if (biometricsAvailable) {
                            appUnlockEnabled = newValue
                            prefs.edit().putBoolean("app_unlock_enabled", newValue).apply()
                        }
                    },
                    enabled = biometricsAvailable,
                    colors = SwitchDefaults.colors(
                        checkedThumbColor = Color.White,
                        checkedTrackColor = Color(0xFF10B981),
                        uncheckedThumbColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        uncheckedTrackColor = MaterialTheme.colorScheme.surfaceVariant
                    ),
                    modifier = Modifier.height(24.dp)
                )
            }
        }

        Spacer(Modifier.height(12.dp))

        // Payment Confirmation
        Surface(
            shape = MaterialTheme.shapes.medium,
            tonalElevation = 1.dp,
            modifier = Modifier.fillMaxWidth()
        ) {
            Row(
                modifier = Modifier.padding(16.dp),
                verticalAlignment = androidx.compose.ui.Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = "Payment Confirmation",
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.Medium
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = "Require authentication before Lightning sends",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Switch(
                    checked = paymentConfirmEnabled,
                    onCheckedChange = { newValue ->
                        if (biometricsAvailable) {
                            paymentConfirmEnabled = newValue
                            prefs.edit().putBoolean("payment_confirmation_enabled", newValue).apply()
                        }
                    },
                    enabled = biometricsAvailable,
                    colors = SwitchDefaults.colors(
                        checkedThumbColor = Color.White,
                        checkedTrackColor = Color(0xFF10B981),
                        uncheckedThumbColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        uncheckedTrackColor = MaterialTheme.colorScheme.surfaceVariant
                    ),
                    modifier = Modifier.height(24.dp)
                )
            }
        }
    }
}
