package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.ui.components.InkButton

/** Full-width filled capsule used for the main action of each reveal step. */
@Composable
internal fun RevealCapsuleButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    minHeight: Dp = 52.dp,
) {
    InkButton(
        onClick = onClick,
        modifier = modifier.fillMaxWidth(),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = RevealColors.buttonFill,
                contentColor = RevealColors.buttonLabel,
            ),
        minHeight = minHeight,
        restingRadius = minHeight / 2,
    ) {
        Text(text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
internal fun LearnMoreButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    TextButton(onClick = onClick, modifier = modifier.heightIn(min = 48.dp)) {
        Text(
            "Learn more",
            color = RevealColors.stablePrimary,
            fontSize = 15.sp,
            fontWeight = FontWeight.Medium,
        )
    }
}

/**
 * Scrolling frame of every quiz step: [body] on top and [actions] at the bottom when they fit; at
 * large text sizes both scroll together, so no button is cut off. [centerBody] centres the body in
 * the space above the actions.
 */
@Composable
internal fun RevealStepFrame(
    body: @Composable () -> Unit,
    actions: @Composable () -> Unit,
    centerBody: Boolean = false,
) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        Column(
            Modifier.verticalScroll(rememberScrollState()).heightIn(min = maxHeight).fillMaxWidth(),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            // An empty first item makes SpaceBetween centre the body between the top and actions.
            if (centerBody) Spacer(Modifier)
            body()
            actions()
        }
    }
}

/** Question and feedback steps: progress and [body] on top, [actions] below. */
@Composable
internal fun QuizStepLayout(
    progress: String,
    body: @Composable ColumnScope.() -> Unit,
    actions: @Composable ColumnScope.() -> Unit,
) {
    RevealStepFrame(
        body = {
            Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp)) {
                Text(
                    progress,
                    modifier = Modifier.padding(top = 16.dp),
                    fontSize = 15.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                body()
            }
        },
        actions = {
            Column(
                modifier = Modifier.padding(horizontal = 24.dp).padding(bottom = 20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                content = actions,
            )
        },
    )
}
