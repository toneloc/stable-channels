package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.domain.seed.MnemonicInput
import com.stablechannels.app.ui.settings.RestoreFlow

/** The restore sheet body: warning, phrase field, word cells and the Restore button. */
@Composable
internal fun RestoreSeedForm(
    restore: RestoreFlow,
    onRestore: () -> Unit,
    modifier: Modifier = Modifier,
) {
    NoLearningTextInput {
        Column(
            modifier.verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            RestoreHeader()
            PhraseField(restore.phrase, enabled = !restore.isBusy, restore::onPhraseChanged)
            SeedWordGrid(
                wordFields = restore.wordFields,
                isReadOnly = restore.isWordFieldsReadOnly || restore.isBusy,
                wordCount = MnemonicInput.detectWordCount(restore.phrase),
                onWordChanged = restore::onWordChanged,
            )
            (restore.formatError ?: restore.error)?.let {
                Text(it, fontSize = 12.sp, color = RevealColors.red)
            }
            RestoreButton(restore.canRestore, isBusy = restore.isBusy, onClick = onRestore)
        }
    }
}

@Composable
private fun RestoreHeader() {
    Column(
        Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Icon(Icons.Filled.Restore, null, Modifier.size(48.dp), tint = RevealColors.orange)
        Text("Restore Seed", fontSize = 22.sp, fontWeight = FontWeight.Bold)
        WarningBanner()
        Text(
            "Enter your 12 or 24 word seed phrase",
            fontSize = 16.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
private fun WarningBanner() {
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(RevealColors.orange.copy(alpha = 0.1f))
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Warning, null, tint = RevealColors.orange)
            Text(
                "Partial Recovery Warning",
                Modifier.padding(start = 8.dp),
                fontSize = 17.sp,
                fontWeight = FontWeight.SemiBold,
            )
        }
        Text(
            "This recovery will restore onchain funds but NOT Lightning channel state. Lightning " +
                "funds will be lost and may require LSP force-close.",
            fontSize = 12.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            "Please withdraw all BTC before proceeding. Existing wallet data will be completely " +
                "overwritten.",
            fontSize = 12.sp,
            fontWeight = FontWeight.SemiBold,
            color = RevealColors.red,
        )
    }
}

@Composable
private fun PhraseField(phrase: String, enabled: Boolean, onPhraseChanged: (String) -> Unit) {
    val style = TextStyle(fontFamily = FontFamily.Monospace, fontSize = 17.sp)
    BasicTextField(
        value = phrase,
        onValueChange = onPhraseChanged,
        modifier =
            Modifier.fillMaxWidth()
                .clip(RoundedCornerShape(12.dp))
                .background(RevealColors.secondaryGroupedBackground)
                .padding(16.dp),
        enabled = enabled,
        textStyle = style.copy(color = LocalContentColor.current),
        keyboardOptions = SeedKeyboardOptions,
        minLines = 5,
        maxLines = 10,
        cursorBrush = SolidColor(LocalContentColor.current),
        decorationBox = { field ->
            if (phrase.isEmpty()) {
                Text(
                    "word1 word2 word3...",
                    style = style,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            field()
        },
    )
}

@Composable
private fun RestoreButton(isValid: Boolean, isBusy: Boolean, onClick: () -> Unit) {
    val shape = RoundedCornerShape(14.dp)
    Row(
        Modifier.fillMaxWidth()
            .clip(shape)
            .background(RevealColors.secondaryGroupedBackground)
            .border(
                1.dp,
                if (isValid) RevealColors.blue.copy(alpha = 0.3f) else Color.Transparent,
                shape,
            )
            .clickable(enabled = isValid && !isBusy, role = Role.Button, onClick = onClick)
            .padding(vertical = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val color = if (isValid) RevealColors.blue else MaterialTheme.colorScheme.onSurfaceVariant
        if (isBusy) {
            CircularProgressIndicator(Modifier.size(16.dp), color = color, strokeWidth = 2.dp)
            Text("Restoring...", fontSize = 17.sp, color = color)
        } else {
            Text("Restore", fontSize = 17.sp, color = color)
        }
    }
}
