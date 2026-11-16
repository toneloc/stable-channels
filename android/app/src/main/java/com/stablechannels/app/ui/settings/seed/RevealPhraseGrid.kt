package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private const val HIDDEN_WORD = "•••••"

private const val WORD_FONT_SP = 13f
private const val WORD_SCALE_STEP = 0.05f

/** iOS minimumScaleFactor(0.8): a word may shrink to 80% before it wraps. */
internal const val MIN_WORD_SCALE = 0.8f

/** One shrink step per overflowing layout pass, stopping at [MIN_WORD_SCALE]. */
internal fun nextWordScale(scale: Float, overflowed: Boolean): Float =
    if (overflowed) maxOf(scale - WORD_SCALE_STEP, MIN_WORD_SCALE) else scale

/** Fewer columns as text grows, so the longest (8-letter) words never truncate. */
internal fun revealGridColumns(fontScale: Float): Int =
    when {
        fontScale >= 1.5f -> 1
        fontScale > 1.0f -> 2
        else -> 3
    }

@Composable
internal fun PhraseGrid(
    words: List<String>,
    columns: Int,
    isRevealed: Boolean,
    onReveal: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(18.dp)
    Box(
        modifier
            .fillMaxWidth()
            .clip(shape)
            .background(RevealColors.secondaryGroupedBackground)
            .border(1.dp, RevealColors.hairline, shape)
    ) {
        // The dots, not the blur, keep the words off screen: blur only works on API 31+.
        Column(
            modifier =
                Modifier.padding(14.dp)
                    .blur(if (isRevealed) 0.dp else 16.dp)
                    .then(if (isRevealed) Modifier else Modifier.clearAndSetSemantics {}),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            words.chunked(columns).forEachIndexed { row, rowWords ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (column in 0 until columns) {
                        val word = rowWords.getOrNull(column)
                        if (word == null) {
                            Spacer(Modifier.weight(1f))
                        } else {
                            WordCell(
                                number = row * columns + column + 1,
                                text = if (isRevealed) word else HIDDEN_WORD,
                                modifier = Modifier.weight(1f),
                            )
                        }
                    }
                }
            }
        }
        if (!isRevealed) RevealVeil(onReveal, Modifier.matchParentSize().clip(shape))
    }
}

@Composable
private fun WordCell(number: Int, text: String, modifier: Modifier = Modifier) {
    val shape = RoundedCornerShape(8.dp)
    Row(
        modifier =
            modifier
                .heightIn(min = 38.dp)
                .clip(shape)
                .background(RevealColors.groupedBackground)
                .border(1.dp, RevealColors.hairline, shape)
                .padding(horizontal = 8.dp, vertical = 4.dp)
                .semantics(mergeDescendants = true) {},
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            "$number.",
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Bold,
            fontSize = 12.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            softWrap = false,
        )
        var scale by remember(text) { mutableFloatStateOf(1f) }
        val atFloor = scale <= MIN_WORD_SCALE
        // A clipped word would be written down wrong; past the floor it wraps instead.
        Text(
            text,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Medium,
            fontSize = (WORD_FONT_SP * scale).sp,
            softWrap = atFloor,
            maxLines = if (atFloor) 2 else 1,
            overflow = TextOverflow.Visible,
            onTextLayout = { scale = nextWordScale(scale, it.hasVisualOverflow) },
        )
    }
}

@Composable
private fun RevealVeil(onReveal: () -> Unit, modifier: Modifier = Modifier) {
    Column(
        modifier =
            modifier
                .background(RevealColors.veil)
                .clickable(role = Role.Button, onClick = onReveal),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
    ) {
        Icon(Icons.Outlined.VisibilityOff, null, Modifier.size(36.dp))
        Text("Tap to reveal", fontSize = 17.sp, fontWeight = FontWeight.Bold)
        Text(
            "Make sure no one is watching your screen.",
            modifier = Modifier.padding(horizontal = 24.dp),
            fontSize = 15.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
