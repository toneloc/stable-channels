package com.stablechannels.app.ui.home

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.CallMade
import androidx.compose.material.icons.automirrored.filled.CallReceived
import androidx.compose.material.icons.filled.ShoppingCart
import androidx.compose.material.icons.filled.Sell
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.trade.BuyScreen
import com.stablechannels.app.ui.trade.SellScreen
import com.stablechannels.app.ui.transfer.ReceiveScreen
import com.stablechannels.app.ui.transfer.SendScreen
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(appState: AppState, modifier: Modifier = Modifier) {
    val totalSats by appState.totalBalanceSats.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val sc by appState.stableChannel.collectAsState()
    val statusMessage by appState.statusMessage.collectAsState()

    var showSend by remember { mutableStateOf(false) }
    var showReceive by remember { mutableStateOf(false) }
    var showBuy by remember { mutableStateOf(false) }
    var showSell by remember { mutableStateOf(false) }

    val totalUSD = (totalSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
    val hasChannel = appState.nodeService.channels.any { it.isChannelReady }
    val scope = rememberCoroutineScope()

    val pullRefreshState = rememberPullToRefreshState()

    PullToRefreshBox(
        isRefreshing = false,
        onRefresh = {
            scope.launch {
                appState.refreshBalances()
                appState.recordCurrentPrice()
            }
        },
        state = pullRefreshState,
        modifier = modifier.fillMaxSize()
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Spacer(Modifier.height(24.dp))

            // Balance
            Text(
                text = totalUSD.usdFormatted(),
                fontSize = 36.sp,
                fontWeight = FontWeight.Bold
            )
            Text(
                text = totalSats.satsFormatted(),
                fontSize = 14.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

            Spacer(Modifier.height(16.dp))

            // Balance bar
            if (sc.expectedUSD.amount > 0) {
                BalanceBar(
                    stableUSD = sc.expectedUSD.amount,
                    nativeSats = sc.nativeChannelBTC.sats,
                    totalSats = totalSats,
                    btcPrice = btcPrice
                )
                Spacer(Modifier.height(16.dp))
            }

            // Action buttons
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                ActionButton("Send", Icons.AutoMirrored.Filled.CallMade, Color(0xFF3B82F6), Modifier.weight(1f)) { showSend = true }
                ActionButton("Receive", Icons.AutoMirrored.Filled.CallReceived, Color(0xFF10B981), Modifier.weight(1f)) { showReceive = true }
            }

            if (hasChannel) {
                Spacer(Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    ActionButton("Buy BTC", Icons.Default.ShoppingCart, Color(0xFFF59E0B), Modifier.weight(1f)) { showBuy = true }
                    ActionButton("Sell BTC", Icons.Default.Sell, Color(0xFF8B5CF6), Modifier.weight(1f)) { showSell = true }
                }
            }

            Spacer(Modifier.height(24.dp))

            // Price
            if (btcPrice > 0) {
                Card(modifier = Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp)) {
                        Text("BTC Price", style = MaterialTheme.typography.labelMedium)
                        Text(btcPrice.usdFormatted(), style = MaterialTheme.typography.headlineSmall)
                    }
                }
            }

            // Status
            if (statusMessage.isNotEmpty()) {
                Spacer(Modifier.height(12.dp))
                Text(
                    text = statusMessage,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }

    // Bottom sheets
    if (showSend) {
        ModalBottomSheet(onDismissRequest = { showSend = false }) {
            SendScreen(appState) { showSend = false }
        }
    }
    if (showReceive) {
        ModalBottomSheet(onDismissRequest = { showReceive = false }) {
            ReceiveScreen(appState) { showReceive = false }
        }
    }
    if (showBuy) {
        ModalBottomSheet(onDismissRequest = { showBuy = false }) {
            BuyScreen(appState) { showBuy = false }
        }
    }
    if (showSell) {
        ModalBottomSheet(onDismissRequest = { showSell = false }) {
            SellScreen(appState) { showSell = false }
        }
    }
}

@Composable
fun ActionButton(title: String, icon: ImageVector, color: Color, modifier: Modifier = Modifier, onClick: () -> Unit) {
    FilledTonalButton(
        onClick = onClick,
        modifier = modifier.height(56.dp),
        colors = ButtonDefaults.filledTonalButtonColors(
            containerColor = color.copy(alpha = 0.1f),
            contentColor = color
        )
    ) {
        Icon(icon, contentDescription = title, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(6.dp))
        Text(title)
    }
}
