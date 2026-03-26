package com.stablechannels.app.ui.home

import android.Manifest
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.CallMade
import androidx.compose.material.icons.automirrored.filled.CallReceived
import androidx.compose.material.icons.filled.ShoppingCart
import androidx.compose.material.icons.filled.Sell
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.*
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.content.PermissionChecker
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.trade.BuyScreen
import com.stablechannels.app.ui.trade.SellScreen
import com.stablechannels.app.ui.transfer.ReceiveScreen
import com.stablechannels.app.ui.transfer.SendScreen
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(appState: AppState, modifier: Modifier = Modifier) {
    val totalSats by appState.totalBalanceSats.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val sc by appState.stableChannel.collectAsState()
    val statusMessage by appState.statusMessage.collectAsState()
    val onchainSats by appState.onchainBalanceSats.collectAsState()

    var showSend by remember { mutableStateOf(false) }
    var showReceive by remember { mutableStateOf(false) }
    var showBuy by remember { mutableStateOf(false) }
    var showSell by remember { mutableStateOf(false) }
    var prefillTradeAmount by remember { mutableDoubleStateOf(0.0) }

    val context = LocalContext.current
    var notificationsEnabled by remember { mutableStateOf(true) }
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                notificationsEnabled = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PermissionChecker.PERMISSION_GRANTED
                } else true
                appState.ensureLSPConnected()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    LaunchedEffect(Unit) { appState.ensureLSPConnected() }

    val totalUSD = (totalSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
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
            // Notification warning
            if (!notificationsEnabled) {
                Card(
                    onClick = {
                        val intent = Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).apply {
                            putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                        }
                        context.startActivity(intent)
                    },
                    colors = CardDefaults.cardColors(containerColor = Color(0xFFDC2626)),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Row(
                        modifier = Modifier.padding(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        Icon(Icons.Default.Warning, contentDescription = null, tint = Color.White)
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                "Notifications Disabled",
                                color = Color.White,
                                fontWeight = FontWeight.SemiBold,
                                fontSize = 14.sp
                            )
                            Text(
                                "Enable notifications for stability payments",
                                color = Color.White.copy(alpha = 0.9f),
                                fontSize = 12.sp
                            )
                        }
                    }
                }
                Spacer(Modifier.height(8.dp))
            }

            Spacer(Modifier.height(24.dp))

            // Balance
            Text(
                text = "Total Balance",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.height(4.dp))
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
            if (totalSats > 0) {
                BalanceBar(
                    stableUSD = sc.expectedUSD.amount,
                    nativeSats = run {
                        val stblSats = if (btcPrice > 0) (sc.expectedUSD.amount / btcPrice * Constants.SATS_IN_BTC).toLong() else 0L
                        if (totalSats > stblSats) totalSats - stblSats else 0L
                    },
                    totalSats = totalSats,
                    btcPrice = btcPrice,
                    modifier = Modifier.padding(horizontal = 24.dp),
                    onDragStarted = { appState.ensureLSPConnected() },
                    onTradeRequest = { direction, amountUSD ->
                        prefillTradeAmount = amountUSD
                        if (direction == TradeDirection.BUY) showBuy = true else showSell = true
                    }
                )
                Spacer(Modifier.height(16.dp))
            }

            // On-chain section
            if (onchainSats > 0) {
                val onchainUSD = (onchainSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
                val hasReadyChannel = appState.nodeService.channels.any { it.isChannelReady }
                val isSweeping = appState.isSpliceInFlight

                Card(
                    modifier = Modifier.fillMaxWidth(),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)
                ) {
                    Column(Modifier.padding(12.dp)) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text("On-chain", style = MaterialTheme.typography.labelMedium)
                            Text(
                                "${onchainSats.satsFormatted()} (${onchainUSD.usdFormatted()})",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        if (isSweeping) {
                            Spacer(Modifier.height(8.dp))
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(8.dp)
                            ) {
                                CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
                                Text("Moving to channel...", style = MaterialTheme.typography.labelSmall)
                            }
                        } else if (!appState.isOpeningChannel) {
                            Spacer(Modifier.height(8.dp))
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Column {
                                    Text("Move to Trading", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    Text("and Spending Account", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                                FilledTonalButton(
                                    onClick = {
                                        scope.launch(Dispatchers.IO) {
                                            if (hasReadyChannel) {
                                                appState.sweepToChannel()
                                            } else {
                                                appState.openChannelWithOnchainFunds()
                                            }
                                        }
                                    },
                                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp)
                                ) {
                                    Text("Swap", fontSize = 13.sp)
                                }
                            }
                        }
                    }
                }
                Spacer(Modifier.height(12.dp))
            }

            // Action buttons
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                ActionButton("Send", Icons.AutoMirrored.Filled.CallMade, Color(0xFF3B82F6), Modifier.weight(1f)) { showSend = true }
                ActionButton("Receive", Icons.AutoMirrored.Filled.CallReceived, Color(0xFF10B981), Modifier.weight(1f)) { showReceive = true }
            }

            Spacer(Modifier.height(8.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                ActionButton("Buy BTC", Icons.Default.ShoppingCart, Color(0xFFF59E0B), Modifier.weight(1f)) { showBuy = true }
                ActionButton("Sell BTC", Icons.Default.Sell, Color(0xFF8B5CF6), Modifier.weight(1f)) { showSell = true }
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
