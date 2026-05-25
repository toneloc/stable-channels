package com.stablechannels.app.ui.settings

import android.content.Context
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.google.firebase.messaging.FirebaseMessaging
import com.stablechannels.app.push.FCMService
import kotlinx.coroutines.launch
import kotlinx.coroutines.tasks.await
import kotlinx.coroutines.withTimeoutOrNull
import java.util.Date
import com.stablechannels.app.util.relativeString

@Composable
fun PushConnectivityView() {
    val context = LocalContext.current
    val clipboardManager = LocalClipboardManager.current
    val scope = rememberCoroutineScope()

    val prefs = FCMService.getPrefs(context)
    var fcmToken by remember { mutableStateOf(prefs.getString("fcm_token", null)) }
    var isRetrying by remember { mutableStateOf(false) }
    var retryError by remember { mutableStateOf<String?>(null) }
    var copiedToken by remember { mutableStateOf(false) }

    val nodeId = FCMService.getNodeId(context)
    val isRegistered = fcmToken != null && nodeId != null

    val lastHeartbeat = prefs.getLong("main_app_last_active", 0L)

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        // FCM Token card
        Surface(
            onClick = {
                if (fcmToken != null) {
                    clipboardManager.setText(AnnotatedString(fcmToken!!))
                    copiedToken = true
                }
            },
            shape = MaterialTheme.shapes.medium,
            tonalElevation = 1.dp,
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(modifier = Modifier.padding(16.dp)) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("FCM Token", style = MaterialTheme.typography.bodyLarge)
                    if (fcmToken != null) {
                        Text(
                            text = if (copiedToken) "Copied ✓" else "Tap to copy",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (copiedToken) Color(0xFF10B981) else MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
                Spacer(Modifier.height(8.dp))
                if (fcmToken != null) {
                    val truncated = if (fcmToken!!.length > 24) {
                        "${fcmToken!!.take(16)}...${fcmToken!!.takeLast(8)}"
                    } else {
                        fcmToken!!
                    }
                    Text(
                        text = truncated,
                        fontFamily = FontFamily.Monospace,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                } else {
                    Text(
                        text = "No token available",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }

        Spacer(Modifier.height(16.dp))

        // Registration status
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text("Registration", style = MaterialTheme.typography.bodyLarge)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Surface(
                    shape = MaterialTheme.shapes.small,
                    color = if (isRegistered) Color(0xFF10B981) else Color(0xFFEF4444),
                    modifier = Modifier.size(8.dp)
                ) {}
                Text(
                    text = if (isRegistered) "Registered" else "Unregistered",
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.Medium,
                    color = if (isRegistered) Color(0xFF10B981) else Color(0xFFEF4444)
                )
            }
        }

        Spacer(Modifier.height(16.dp))

        // Last heartbeat
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text("Last Heartbeat", style = MaterialTheme.typography.bodyLarge)
            Text(
                text = if (lastHeartbeat > 0) {
                    Date(lastHeartbeat * 1000).relativeString()
                } else {
                    "No heartbeat recorded"
                },
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }

        Spacer(Modifier.height(24.dp))

        // Retry button — green to match app branding
        Button(
            onClick = {
                isRetrying = true
                retryError = null
                scope.launch {
                    try {
                        val token = withTimeoutOrNull(10_000L) {
                            FirebaseMessaging.getInstance().token.await()
                        }
                        if (token != null) {
                            FCMService.saveToken(context, token)
                            fcmToken = token
                            copiedToken = false
                            retryError = null
                        } else {
                            retryError = "Token could not be retrieved (timeout)"
                        }
                    } catch (e: Exception) {
                        retryError = e.message ?: "Token retrieval failed"
                    } finally {
                        isRetrying = false
                    }
                }
            },
            modifier = Modifier.fillMaxWidth(),
            enabled = !isRetrying,
            colors = ButtonDefaults.buttonColors(
                containerColor = Color(0xFF10B981),
                contentColor = Color.White
            )
        ) {
            if (isRetrying) {
                CircularProgressIndicator(
                    modifier = Modifier.size(16.dp),
                    strokeWidth = 2.dp,
                    color = Color.White
                )
                Spacer(Modifier.width(8.dp))
            }
            Text("Retry Token Retrieval")
        }

        if (retryError != null) {
            Spacer(Modifier.height(8.dp))
            Text(
                text = retryError!!,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error
            )
        }
    }
}
