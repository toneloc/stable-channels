package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.domain.seed.RevealQuizQuestion

/** A quiz question with its two answer pills. */
@Composable
internal fun RevealQuizQuestion(
    question: RevealQuizQuestion,
    progress: String,
    onSelectOption: (Int) -> Unit,
    onLearnMore: () -> Unit,
) {
    QuizStepLayout(
        progress = progress,
        body = {
            Text(
                question.prompt,
                modifier = Modifier.padding(top = 12.dp),
                fontSize = 28.sp,
                lineHeight = 36.sp,
                fontWeight = FontWeight.Bold,
            )
        },
        actions = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                question.options.forEachIndexed { index, option ->
                    OptionPill(option, onClick = { onSelectOption(index) })
                }
            }
            // 20 dp above, as the iOS 12 pt stack spacing plus 8 pt top padding.
            LearnMoreButton(
                onClick = onLearnMore,
                modifier = Modifier.padding(top = 20.dp, bottom = 20.dp),
            )
        },
    )
}

@Composable
private fun OptionPill(text: String, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth().heightIn(min = 54.dp),
        shape = RoundedCornerShape(14.dp),
        color = RevealColors.secondaryGroupedBackground,
        border = BorderStroke(1.dp, RevealColors.hairline),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(
                text,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                fontSize = 17.sp,
                fontWeight = FontWeight.Medium,
                textAlign = TextAlign.Center,
            )
        }
    }
}
