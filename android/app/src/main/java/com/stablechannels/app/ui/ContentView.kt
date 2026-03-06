package com.stablechannels.app.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.Phase

@Composable
fun ContentView() {
    val context = LocalContext.current
    val appState = remember { AppState(context) }

    LaunchedEffect(Unit) { appState.start() }
    DisposableEffect(Unit) { onDispose { appState.stop() } }

    val phase by appState.phase.collectAsState()
    val errorMessage by appState.errorMessage.collectAsState()

    when (phase) {
        Phase.LOADING -> LoadingView()
        Phase.ONBOARDING -> SyncingView() // Auto-create handles this
        Phase.SYNCING -> SyncingView()
        Phase.WALLET -> MainTabView(appState)
        Phase.ERROR -> ErrorView(errorMessage) { appState.start() }
    }
}

@Composable
private fun LoadingView() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            CircularProgressIndicator()
            Spacer(Modifier.height(16.dp))
            Text("Starting...")
        }
    }
}

@Composable
private fun SyncingView() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            CircularProgressIndicator()
            Spacer(Modifier.height(16.dp))
            Text("Syncing wallet...")
        }
    }
}

@Composable
private fun ErrorView(message: String, onRetry: () -> Unit) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier.padding(32.dp)
        ) {
            Text("Error", style = MaterialTheme.typography.headlineMedium)
            Spacer(Modifier.height(8.dp))
            Text(message, style = MaterialTheme.typography.bodyMedium)
            Spacer(Modifier.height(16.dp))
            Button(onClick = onRetry) { Text("Retry") }
        }
    }
}

