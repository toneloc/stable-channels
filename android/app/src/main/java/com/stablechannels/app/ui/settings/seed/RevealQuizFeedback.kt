package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.domain.seed.RevealQuizQuestion

/** Right or wrong result for the answered question, with Continue or Try again. */
@Composable
internal fun RevealQuizFeedback(
    question: RevealQuizQuestion,
    progress: String,
    isCorrect: Boolean,
    onContinue: () -> Unit,
    onLearnMore: () -> Unit,
) {
    QuizStepLayout(
        progress = progress,
        body = {
            val statusColor = if (isCorrect) RevealColors.green else RevealColors.red
            Row(
                modifier = Modifier.padding(top = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    imageVector = if (isCorrect) Icons.Filled.CheckCircle else Icons.Filled.Cancel,
                    contentDescription = null,
                    modifier = Modifier.size(22.dp),
                    tint = statusColor,
                )
                Text(
                    if (isCorrect) "Correct!" else "Incorrect!",
                    color = statusColor,
                    fontSize = 20.sp,
                    fontWeight = FontWeight.Bold,
                )
            }
            Text(
                if (isCorrect) question.correctHeading else question.incorrectHeading,
                modifier = Modifier.padding(top = 12.dp),
                fontSize = 22.sp,
                lineHeight = 30.sp,
                fontWeight = FontWeight.Bold,
            )
            Text(
                if (isCorrect) question.correctExplanation else question.incorrectExplanation,
                modifier = Modifier.padding(top = 16.dp),
                fontSize = 17.sp,
                lineHeight = 26.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        actions = {
            RevealCapsuleButton(
                text = if (isCorrect) "Continue" else "Try again",
                onClick = onContinue,
                modifier = Modifier.padding(bottom = 16.dp),
            )
            LearnMoreButton(onClick = onLearnMore, modifier = Modifier.padding(bottom = 20.dp))
        },
    )
}
