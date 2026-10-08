package com.stablechannels.app.ui.home

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.AppState
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.ui.history.PaymentRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private const val MAX_ROWS = 4
private const val FOUR_ROWS_MIN_SPACE_DP = 252f
private const val THREE_ROWS_MIN_SPACE_DP = 200f
private const val TWO_ROWS_MIN_SPACE_DP = 148f

/** One row when the card sits near the bottom of the screen, up to four when there is room. */
internal fun recentActivityRowCount(spaceBelowDp: Float): Int =
    when {
        spaceBelowDp >= FOUR_ROWS_MIN_SPACE_DP -> MAX_ROWS
        spaceBelowDp >= THREE_ROWS_MIN_SPACE_DP -> 3
        spaceBelowDp >= TWO_ROWS_MIN_SPACE_DP -> 2
        else -> 1
    }

/** Latest payments on Home. [reloadKey] should change when something may have been recorded. */
@Composable
fun RecentActivity(
    appState: AppState,
    reloadKey: Any,
    maxRows: Int,
    onViewAll: () -> Unit,
    onPaymentClick: (PaymentRecord) -> Unit,
    onLoaded: (List<PaymentRecord>) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    var payments by remember { mutableStateOf<List<PaymentRecord>>(emptyList()) }
    val currentPrice by appState.priceService.currentPrice.collectAsState()
    val epoch by appState.confirmationUpdateEpoch.collectAsState()
    val isFlashing by appState.paymentFlash.collectAsState()

    LaunchedEffect(epoch, isFlashing, reloadKey) {
        payments =
            withContext(Dispatchers.IO) {
                appState.databaseService?.getRecentPayments(MAX_ROWS) ?: emptyList()
            }
        onLoaded(payments)
    }

    val shown = payments.take(maxRows)
    if (shown.isEmpty()) return

    Column(modifier = modifier.fillMaxWidth().padding(horizontal = 4.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                "Recent activity",
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                "View all",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.clickable(onClick = onViewAll).padding(vertical = 4.dp),
            )
        }
        shown.forEach { payment ->
            PaymentRow(payment, currentPrice, compact = true) { onPaymentClick(payment) }
        }
    }
}
