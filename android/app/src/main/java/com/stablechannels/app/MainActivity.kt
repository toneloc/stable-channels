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
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

class MainActivity : FragmentActivity() {

    companion object {
        // How long we tolerate the app being backgrounded by the system share/chooser dialog
        // before treating it as a real background transition. Bounds the suppression requested
        // in AppState.suppressNextBackgroundCycle so a user who continues into another app still
        // gets the node stopped (and later correctly resynced) instead of it staying active and
        // the eventual foreground sync being skipped indefinitely.
        private const val SHARE_SUPPRESS_WINDOW_MS = 3000L
    }

    private lateinit var appState: AppState

    private val notificationPermissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { _ -> }

    private var isLocked by mutableStateOf(false)
    private var isAuthenticating by mutableStateOf(false)
    private var lastBackgroundedTime: Long = 0L
    private var isFirstResume = true
    private var pendingBackgroundStopJob: Job? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        appState = AppState(applicationContext)
        requestNotificationPermission()

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
        if (!::appState.isInitialized) return
        if (AppState.suppressNextBackgroundCycle) {
            // Give the transient share/chooser dialog a short grace window instead of stopping
            // the node immediately. If the user is still away once the window elapses (e.g. they
            // continued into the target app), fall back to the normal background stop so the
            // node doesn't stay active indefinitely and the next resume still resyncs correctly.
            pendingBackgroundStopJob = lifecycleScope.launch {
                delay(SHARE_SUPPRESS_WINDOW_MS)
                AppState.suppressNextBackgroundCycle = false
                appState.stopNodeForBackground()
            }
        } else {
            appState.stopNodeForBackground()
        }
    }

    override fun onResume() {
        super.onResume()
        if (!::appState.isInitialized) return
        // Cancel the pending background-stop job synchronously, on the main thread, before
        // anything else. A process that was frozen while backgrounded can have its delayed
        // stop and this resume fire almost simultaneously on unfreeze; racing that cancellation
        // through restartNodeFromForeground's IO dispatch instead lost that race in practice,
        // letting the node fully stop moments before resume and forcing a visible full resync.
        appState.cancelBackgroundStop()
        // Defensive reset: whatever caused the last pause (picker or otherwise) has resolved by
        // the time we're resumed, so isPickingMedia can never get stuck true across a full cycle.
        appState.isPickingMedia = false
        val quickReturnFromShare = AppState.suppressNextBackgroundCycle
        pendingBackgroundStopJob?.cancel()
        pendingBackgroundStopJob = null
        AppState.suppressNextBackgroundCycle = false
        if (quickReturnFromShare) {
            // Returned within the grace window: the node was never stopped, so skip the
            // foreground resync that would otherwise refresh the UI.
        } else if (isFirstResume) {
            isFirstResume = false
            appState.restartNodeFromForeground()
            return
        } else {
            appState.restartNodeFromForeground()
        }
        if (AppAccessPreferencesManager.isAppUnlockEnabled(this)) {
            val elapsed = System.currentTimeMillis() - lastBackgroundedTime
            if (elapsed > 5000L) {
                isLocked = true
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        if (!::appState.isInitialized) return
        // Route through the graced stop (like onPause()) instead of stopping immediately,
        // since the OS can destroy/recreate this Activity on plain backgrounding, not just a real close.
        appState.stopNodeForBackground()
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
