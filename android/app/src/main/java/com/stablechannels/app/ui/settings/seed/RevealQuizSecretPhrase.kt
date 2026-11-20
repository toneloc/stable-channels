package com.stablechannels.app.ui.settings.seed

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import com.stablechannels.app.domain.seed.AppLifecyclePhase
import com.stablechannels.app.ui.components.InkButton
import com.stablechannels.app.ui.components.NeutralTextButton
import com.stablechannels.app.util.ClipboardUtils
import kotlinx.coroutines.delay

private const val COPIED_CHECKMARK_MS = 2_500L

/** The recovery phrase behind a tap-to-reveal veil, with copy, hide and Done. */
@Composable
fun RevealQuizSecretPhrase(mnemonic: String, onDone: () -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val haptics = LocalHapticFeedback.current
    var isRevealed by remember { mutableStateOf(false) }
    var copyCount by remember { mutableIntStateOf(0) }
    var copiedSeed by remember { mutableStateOf(false) }
    var showCopyWarning by remember { mutableStateOf(false) }
    val words = remember(mnemonic) { mnemonic.split(' ').filter { it.isNotEmpty() } }
    val columns = revealGridColumns(LocalDensity.current.fontScale)

    // Hide again before the recents snapshot, as iOS does when the scene leaves active.
    ObserveSeedLifecyclePhase { if (it != AppLifecyclePhase.ACTIVE) isRevealed = false }
    // Keyed on the count so copying again restarts the checkmark timer.
    LaunchedEffect(copyCount) {
        if (copiedSeed) {
            delay(COPIED_CHECKMARK_MS)
            copiedSeed = false
        }
    }

    Column(modifier.fillMaxSize()) {
        Column(
            modifier =
                Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                "Your Secret Recovery Phrase gives full access to your wallet. Do not share it with anyone.",
                modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(top = 12.dp),
                fontSize = 15.sp,
                lineHeight = 23.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            PhraseGrid(
                words = words,
                columns = columns,
                isRevealed = isRevealed,
                onReveal = {
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    isRevealed = true
                },
                modifier = Modifier.padding(horizontal = 20.dp),
            )
            AnimatedVisibility(visible = isRevealed, enter = fadeIn(), exit = fadeOut()) {
                Row(
                    modifier = Modifier.padding(horizontal = 20.dp).padding(top = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    CapsuleAction(
                        onClick = { if (isRevealed) showCopyWarning = true },
                        Modifier.weight(1f),
                    ) {
                        Icon(
                            imageVector =
                                if (copiedSeed) Icons.Filled.Check else Icons.Outlined.ContentCopy,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                            tint =
                                if (copiedSeed) RevealColors.copiedCheck
                                else MaterialTheme.colorScheme.onSurface,
                        )
                        Text("Copy Seed", fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    }
                    CapsuleAction(
                        onClick = {
                            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                            isRevealed = false
                        },
                        Modifier.weight(1f),
                    ) {
                        Icon(Icons.Filled.VisibilityOff, null, Modifier.size(18.dp))
                        Text("Hide Seed", fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    }
                }
            }
        }
        InkButton(
            onClick = onDone,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 20.dp),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = RevealColors.buttonFill,
                    contentColor = RevealColors.buttonLabel,
                ),
            minHeight = 52.dp,
            restingRadius = 26.dp,
        ) {
            Text("Done", fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
        }
    }

    if (showCopyWarning) {
        AlertDialog(
            onDismissRequest = { showCopyWarning = false },
            properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn),
            title = { Text("Copy Secret Recovery Phrase?") },
            text = {
                Text(
                    "Your clipboard is accessible to other apps. For your protection, it will be automatically cleared in ${ClipboardUtils.CLEAR_DELAY_SECONDS} seconds."
                )
            },
            confirmButton = {
                NeutralTextButton(
                    onClick = {
                        showCopyWarning = false
                        ClipboardUtils.copySensitive(context, "Seed Phrase", mnemonic)
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        copiedSeed = true
                        copyCount++
                    }
                ) {
                    Text("Copy to Clipboard")
                }
            },
            dismissButton = {
                NeutralTextButton(onClick = { showCopyWarning = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun CapsuleAction(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable RowScope.() -> Unit,
) {
    Surface(
        onClick = onClick,
        modifier = modifier.heightIn(min = 48.dp),
        shape = CircleShape,
        color = RevealColors.secondaryGroupedBackground,
        border = BorderStroke(1.dp, RevealColors.hairline),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
            content = content,
        )
    }
}
