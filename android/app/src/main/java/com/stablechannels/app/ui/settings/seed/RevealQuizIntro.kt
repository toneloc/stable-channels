package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
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
    RevealStepFrame(
        centerBody = true,
        body = {
            Column(
                modifier = Modifier.fillMaxWidth().padding(vertical = 24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
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
        },
        actions = {
            Column(
                modifier =
                    Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                RevealCapsuleButton(text = "Get started", onClick = onGetStarted)
                LearnMoreButton(onClick = onLearnMore, modifier = Modifier.padding(bottom = 20.dp))
            }
        },
    )
}
