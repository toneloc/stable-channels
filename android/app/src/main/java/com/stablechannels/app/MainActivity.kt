package com.stablechannels.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import com.stablechannels.app.services.AppAccessPreferencesManager
import com.stablechannels.app.services.BiometricService
import com.stablechannels.app.ui.ContentView
import com.stablechannels.app.ui.theme.StableChannelsTheme
import kotlinx.coroutines.launch

class MainActivity : FragmentActivity() {

    private lateinit var appState: AppState

    private val notificationPermissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { _ -> }

    private var isLocked by mutableStateOf(false)
    private var isAuthenticating by mutableStateOf(false)
    private var lastBackgroundedTime: Long = 0L
    private var isFirstResume = true

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        requestNotificationPermission()

        appState = AppState(applicationContext)

        // Lock on launch if app unlock is enabled
        if (AppAccessPreferencesManager.isAppUnlockEnabled(this)) {
            isLocked = true
        }

        setContent {
            StableChannelsTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background
                ) {
                    if (isLocked) {
                        AuthLockOverlay()
                    } else {
                        ContentView(appState)
                    }
                }
            }
        }
    }

    override fun onPause() {
        super.onPause()
        lastBackgroundedTime = System.currentTimeMillis()
        appState.stopNodeForBackground()
    }

    override fun onResume() {
        super.onResume()
        if (isFirstResume) {
            isFirstResume = false
            appState.restartNodeFromForeground()
            return
        }
        appState.restartNodeFromForeground()
        if (AppAccessPreferencesManager.isAppUnlockEnabled(this)) {
            val elapsed = System.currentTimeMillis() - lastBackgroundedTime
            if (elapsed > 5000L) {
                isLocked = true
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        appState.stop()
    }

    @Composable
    private fun AuthLockOverlay() {
        val scope = rememberCoroutineScope()
        var authError by remember { mutableStateOf<String?>(null) }

        fun performAuth() {
            if (isAuthenticating) return
            isAuthenticating = true
            authError = null
            scope.launch {
                val result = BiometricService.authenticate(this@MainActivity, "Unlock Stable Channels")
                if (result == BiometricService.AuthResult.SUCCESS) {
                    isLocked = false
                } else {
                    authError = "Authentication failed. Tap Unlock to try again."
                }
                isAuthenticating = false
            }
        }

        // Auto-trigger auth on first composition
        LaunchedEffect(Unit) {
            performAuth()
        }

        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center
        ) {
            Text(
                "Stable Channels",
                style = MaterialTheme.typography.headlineMedium
            )
            Spacer(Modifier.height(16.dp))
            Text(
                "Authentication required",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            authError?.let {
                Spacer(Modifier.height(8.dp))
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error
                )
            }
            Spacer(Modifier.height(24.dp))
            Button(
                onClick = { performAuth() },
                enabled = !isAuthenticating
            ) {
                if (isAuthenticating) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                } else {
                    Text("Unlock")
                }
            }
        }
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED
            ) {
                notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
    }
}
