package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** First quiz step: the vault artwork, why the quiz exists, and Get started. */
@Composable
internal fun RevealQuizIntro(onGetStarted: () -> Unit, onLearnMore: () -> Unit) {
    Column(Modifier.fillMaxSize()) {
        // Centred when it fits; scrolls at large text sizes instead of truncating.
        BoxWithConstraints(Modifier.weight(1f)) {
            Column(
                modifier =
                    Modifier.verticalScroll(rememberScrollState())
                        .heightIn(min = maxHeight)
                        .fillMaxWidth()
                        // Only shows once it scrolls: centred content keeps the same position.
                        .padding(vertical = 24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                RevealQuizPortalIllustration(Modifier.padding(bottom = 36.dp))
                Text(
                    "To reveal your Secret Recovery Phrase, you need to correctly answer two questions",
                    modifier = Modifier.padding(horizontal = 32.dp),
                    fontSize = 17.sp,
                    lineHeight = 26.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
        }
        Column(
            modifier = Modifier.padding(horizontal = 24.dp).padding(bottom = 20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            RevealCapsuleButton(text = "Get started", onClick = onGetStarted)
            LearnMoreButton(onClick = onLearnMore, modifier = Modifier.padding(bottom = 20.dp))
        }
    }
}
