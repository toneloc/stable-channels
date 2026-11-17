package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.domain.seed.MnemonicInput

/** Numbered word cells: 2 columns for 12 words, 3 for 24; read-only while they mirror the field. */
@Composable
internal fun SeedWordGrid(
    wordFields: List<String>,
    isReadOnly: Boolean,
    wordCount: Int,
    onWordChanged: (Int, String) -> Unit,
) {
    val columns = if (wordCount == MnemonicInput.WORD_COUNT_12) 2 else 3
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        (0 until wordCount).chunked(columns).forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                row.forEach { index ->
                    WordCell(
                        number = index + 1,
                        word = wordFields[index],
                        isReadOnly = isReadOnly,
                        onWordChanged = { onWordChanged(index, it) },
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun WordCell(
    number: Int,
    word: String,
    isReadOnly: Boolean,
    onWordChanged: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val style =
        TextStyle(
            fontFamily = FontFamily.Monospace,
            fontSize = 12.sp,
            color = LocalContentColor.current,
        )
    val cell =
        Modifier.semantics { contentDescription = "Word $number" }
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(RevealColors.tertiaryGroupedBackground)
            .padding(horizontal = 8.dp, vertical = 6.dp)
    Row(
        modifier.padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            "$number.",
            modifier = Modifier.widthIn(min = 18.dp),
            fontSize = 12.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.End,
        )
        if (isReadOnly) {
            Text(word, cell, style = style, maxLines = 1)
        } else {
            BasicTextField(
                value = word,
                onValueChange = onWordChanged,
                modifier = cell,
                textStyle = style,
                keyboardOptions = SeedKeyboardOptions,
                singleLine = true,
                cursorBrush = SolidColor(LocalContentColor.current),
            )
        }
    }
}
