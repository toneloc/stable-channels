package com.stablechannels.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.Phase
import com.stablechannels.app.ui.components.UnifiedBalanceLaunchView

@Composable
fun ContentView(appState: AppState) {
  val phase by appState.phase.collectAsState()
  val errorMessage by appState.errorMessage.collectAsState()

  when (phase) {
    Phase.LOADING,
    Phase.ONBOARDING,
    Phase.SYNCING -> SyncingView()
    Phase.WALLET -> MainTabView(appState)
    Phase.ERROR -> ErrorView(errorMessage) { appState.start() }
  }
}

@Composable
fun SyncingView(
    modifier: Modifier = Modifier,
    isSyncComplete: Boolean = false,
    previewStage: com.stablechannels.app.ui.components.BalanceScaleKinematics.Stage? = null,
    onBalanced: (() -> Unit)? = null,
) {
  val elapsedSeconds by
      produceState(initialValue = 0f) {
        val startNanos = withFrameNanos { it }
        while (true) {
          withFrameNanos { frameTimeNanos ->
            value = (frameTimeNanos - startNanos) / 1_000_000_000f
          }
        }
      }

  val shimmerDuration = 1.15f
  val crossfadeDuration = 0.40f

  val effectiveElapsed =
      when (previewStage) {
        is com.stablechannels.app.ui.components.BalanceScaleKinematics.Stage.Shimmer -> 0.70f
        is com.stablechannels.app.ui.components.BalanceScaleKinematics.Stage.Oscillating -> 1.65f
        else -> elapsedSeconds
      }

  val rawProgress = ((effectiveElapsed - shimmerDuration) / crossfadeDuration).coerceIn(0f, 1f)
  val smoothProgress = rawProgress * rawProgress * (3f - 2f * rawProgress)

  Box(
      modifier = modifier.fillMaxSize(),
      contentAlignment = Alignment.Center,
  ) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(22.dp),
    ) {
      UnifiedBalanceLaunchView(
          isSyncComplete = isSyncComplete,
          size = 115.dp,
          elapsedSeconds = effectiveElapsed,
          previewStage = previewStage,
          onBalanced = onBalanced,
      )

      Box(
          modifier = Modifier.fillMaxWidth().height(52.dp),
          contentAlignment = Alignment.Center,
      ) {
        // Stage 1: Brand introduction during initial shimmer
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier =
                Modifier.graphicsLayer {
                  alpha = 1f - smoothProgress
                  translationY = -6.dp.toPx() * smoothProgress
                },
        ) {
          Text(
              text = "Stable Channels",
              style = MaterialTheme.typography.titleMedium,
              fontWeight = FontWeight.Bold,
              color = MaterialTheme.colorScheme.onBackground,
          )
          Spacer(modifier = Modifier.height(4.dp))
          Text(
              text = "Self-custodial bitcoin wallet",
              style = MaterialTheme.typography.bodyMedium,
              color = MaterialTheme.colorScheme.onSurfaceVariant,
          )
        }

        // Stage 2: Active syncing status during oscillation
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier =
                Modifier.graphicsLayer {
                  alpha = smoothProgress
                  translationY = 6.dp.toPx() * (1f - smoothProgress)
                },
        ) {
          Text(
              text = "Wallet Syncing...",
              style = MaterialTheme.typography.titleMedium,
              fontWeight = FontWeight.Bold,
              color = MaterialTheme.colorScheme.onBackground,
          )
          Spacer(modifier = Modifier.height(4.dp))
          Text(
              text = "This may take a moment",
              style = MaterialTheme.typography.bodyMedium,
              color = MaterialTheme.colorScheme.onSurfaceVariant,
          )
        }
      }
    }
  }
}

@Composable
private fun ErrorView(message: String, onRetry: () -> Unit) {
  Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.padding(32.dp),
    ) {
      Text("Error", style = MaterialTheme.typography.headlineMedium)
      Spacer(Modifier.height(8.dp))
      Text(message, style = MaterialTheme.typography.bodyMedium)
      Spacer(Modifier.height(16.dp))
      Button(onClick = onRetry) { Text("Retry") }
    }
  }
}

@Preview(name = "1. App Launch Flow - Dark", showBackground = true, backgroundColor = 0xFF000000)
@Composable
private fun PreviewSyncingFlowDark() {
  MaterialTheme(colorScheme = darkColorScheme()) {
    Surface(modifier = Modifier.fillMaxSize(), color = Color.Black) {
      SyncingView()
    }
  }
}

@Preview(name = "2. Opening Shimmer Beam - Dark", showBackground = true, backgroundColor = 0xFF000000)
@Composable
private fun PreviewShimmerStageDark() {
  MaterialTheme(colorScheme = darkColorScheme()) {
    Surface(modifier = Modifier.fillMaxSize(), color = Color.Black) {
      SyncingView(
          previewStage = com.stablechannels.app.ui.components.BalanceScaleKinematics.Stage.Shimmer(0.5f)
      )
    }
  }
}

@Preview(name = "3. Wallet Syncing Oscillation - Dark", showBackground = true, backgroundColor = 0xFF000000)
@Composable
private fun PreviewOscillatingStageDark() {
  MaterialTheme(colorScheme = darkColorScheme()) {
    Surface(modifier = Modifier.fillMaxSize(), color = Color.Black) {
      SyncingView(
          previewStage = com.stablechannels.app.ui.components.BalanceScaleKinematics.Stage.Oscillating(4.8f)
      )
    }
  }
}

@Preview(name = "4. App Launch Flow - Light", showBackground = true, backgroundColor = 0xFFF2F2F7)
@Composable
private fun PreviewSyncingFlowLight() {
  MaterialTheme(colorScheme = lightColorScheme()) {
    Surface(modifier = Modifier.fillMaxSize(), color = Color(0xFFF2F2F7)) {
      SyncingView()
    }
  }
}
