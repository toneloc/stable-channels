package com.stablechannels.app.ui

import androidx.compose.animation.core.*
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.Phase
import com.stablechannels.app.R

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
private fun PulsatingLogo() {
    val infiniteTransition = rememberInfiniteTransition(label = "pulse")
    val scale by infiniteTransition.animateFloat(
        initialValue = 0.94f,
        targetValue = 1.06f,
        animationSpec = infiniteRepeatable(
            animation = tween(1200, easing = EaseInOut),
            repeatMode = RepeatMode.Reverse
        ),
        label = "pulse_scale"
    )
    Image(
        painter = painterResource(R.mipmap.ic_launcher),
        contentDescription = "Stable Channels",
        modifier = Modifier
            .size(90.dp)
            .clip(CircleShape)
            .scale(scale)
    )
}

@Composable
private fun LoadingView() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(24.dp)
        ) {
            PulsatingLogo()
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text("Stable Channels", fontWeight = FontWeight.SemiBold)
                Spacer(Modifier.height(4.dp))
                Text(
                    "Self-custodial bitcoin trading",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

@Composable
private fun SyncingView() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(24.dp)
        ) {
            PulsatingLogo()
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                CircularProgressIndicator()
                Spacer(Modifier.height(12.dp))
                Text("Syncing wallet...")
                Text(
                    "This may take a moment",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
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

