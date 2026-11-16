package com.stablechannels.app.ui.settings.seed

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.SecureFlagPolicy
import com.stablechannels.app.domain.seed.SelfCustodyGuidelineItem
import kotlinx.coroutines.launch

/** The self-custody guidelines, opened from any Learn more link of the quiz. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun RevealQuizLearnMoreSheet(onDismiss: () -> Unit) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val close: () -> Unit = {
        scope.launch { sheetState.hide() }.invokeOnCompletion { onDismiss() }
    }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = RevealColors.groupedBackground,
        dragHandle = null,
        // Stops below the status bar like an iOS large sheet instead of running under it.
        modifier = Modifier.statusBarsPadding(),
        // The iOS 17 and 18 default sheet corner, not the Material 28 dp.
        shape = RoundedCornerShape(topStart = 10.dp, topEnd = 10.dp),
        // Stacked over the secure quiz window, so it stays out of screenshots too.
        properties = ModalBottomSheetProperties(securePolicy = SecureFlagPolicy.SecureOn),
    ) {
        Column(Modifier.fillMaxHeight()) {
            Row(Modifier.fillMaxWidth().padding(8.dp), horizontalArrangement = Arrangement.End) {
                IconButton(onClick = close) {
                    Icon(
                        Icons.Filled.Cancel,
                        contentDescription = "Close",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Column(
                modifier =
                    Modifier.weight(1f)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 24.dp)
                        .padding(bottom = 32.dp),
                verticalArrangement = Arrangement.spacedBy(32.dp),
            ) {
                Column(
                    Modifier.padding(top = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text("Self-Custody Guidelines", fontSize = 28.sp, fontWeight = FontWeight.Bold)
                    SecondaryText(
                        "Your Secret Recovery Phrase gives complete ownership of your Bitcoin and Lightning funds. Follow these core principles to stay secure."
                    )
                }
                HorizontalDivider()
                Column(verticalArrangement = Arrangement.spacedBy(28.dp)) {
                    SelfCustodyGuidelineItem.all.forEach { item ->
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text(item.title, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
                            SecondaryText(item.description)
                        }
                    }
                }
            }
            RevealCapsuleButton(
                text = "Done",
                onClick = close,
                modifier = Modifier.padding(horizontal = 20.dp).padding(bottom = 20.dp),
                minHeight = 50.dp,
            )
        }
    }
}

@Composable
private fun SecondaryText(text: String) {
    Text(
        text,
        fontSize = 15.sp,
        lineHeight = 24.sp,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
