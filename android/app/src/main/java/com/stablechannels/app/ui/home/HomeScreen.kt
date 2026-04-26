package com.stablechannels.app.ui.home

import android.Manifest
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
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
import com.stablechannels.app.util.btcSpacedFormatted
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
    val hasReadyChannel by appState.hasReadyChannel.collectAsState()
    val spendableOnchainSats by appState.spendableOnchainSats.collectAsState()

    var showSend by remember { mutableStateOf(false) }
    var showReceive by remember { mutableStateOf(false) }
    var showBuy by remember { mutableStateOf(false) }
    var showSell by remember { mutableStateOf(false) }
    var prefillTradeAmount by remember { mutableDoubleStateOf(0.0) }
    var showBTC by remember { mutableStateOf(false) }

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
            // Title
            Text(
                text = "Stable Channels",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.fillMaxWidth()
            )
            Spacer(Modifier.height(8.dp))

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

            // Balance (tap to toggle USD/BTC)
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.clickable { showBTC = !showBTC }
            ) {
                Text(
                    text = "Total Balance",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(4.dp))
                if (showBTC) {
                    Text(
                        text = totalSats.btcSpacedFormatted(),
                        fontSize = 32.sp,
                        fontWeight = FontWeight.Bold,
                        fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace
                    )
                } else {
                    Text(
                        text = totalUSD.usdFormatted(),
                        fontSize = 36.sp,
                        fontWeight = FontWeight.Bold
                    )
                }
                Text(
                    text = if (showBTC) totalUSD.usdFormatted() else totalSats.btcSpacedFormatted(),
                    fontSize = 14.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

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
                            // 1. Splice-in in progress
                            Spacer(Modifier.height(8.dp))
                            PendingRow("Swap pending...", appState.spliceTxid, context)
                        } else if (hasReadyChannel && spendableOnchainSats > 0) {
                            // Has channel + confirmed funds — offer to sweep
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
                                            appState.sweepToChannel()
                                        }
                                    },
                                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp)
                                ) {
                                    Text("Swap", fontSize = 13.sp)
                                }
                            }
                        } else if (spendableOnchainSats == 0L) {
                            // 3. Unconfirmed deposit (with or without channel)
                            Spacer(Modifier.height(8.dp))
                            PendingRow("Deposit confirming...", appState.fundingTxid, context)
                            if (!hasReadyChannel) {
                                Text("Receive over Lightning to create your Trading and Spending Account",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        } else {
                            // 4. No channel, confirmed deposit — just needs Lightning
                            Spacer(Modifier.height(8.dp))
                            Text("Receive over Lightning to create your Trading and Spending Account",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
                Spacer(Modifier.height(12.dp))
            }

            // Price chart
            if (btcPrice > 0) {
                PriceChart(
                    databaseService = appState.databaseService,
                    currentPrice = btcPrice
                )
                Spacer(Modifier.height(12.dp))
            }

            // Hint text when no channel
            if (!hasReadyChannel) {
                Text("Receive BTC to get started",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(4.dp))
            }

            // Action buttons
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                ActionButton("Send", Icons.AutoMirrored.Filled.CallMade, Color(0xFF3B82F6), Modifier.weight(1f)) { showSend = true }
                ActionButton("Receive", Icons.AutoMirrored.Filled.CallReceived, Color(0xFF10B981), Modifier.weight(1f), pulse = !hasReadyChannel) { showReceive = true }
            }

            Spacer(Modifier.height(8.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                ActionButton("Buy BTC", Icons.Default.ShoppingCart, Color(0xFFF59E0B), Modifier.weight(1f)) { showBuy = true }
                ActionButton("Sell BTC", Icons.Default.Sell, Color(0xFF8B5CF6), Modifier.weight(1f)) { showSell = true }
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
        ModalBottomSheet(
            onDismissRequest = { showSend = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            modifier = Modifier.fillMaxHeight(0.9f)
        ) {
            SendScreen(appState) { showSend = false }
        }
    }
    if (showReceive) {
        ModalBottomSheet(
            onDismissRequest = { showReceive = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            modifier = Modifier.fillMaxHeight(0.9f)
        ) {
            ReceiveScreen(appState) { showReceive = false }
        }
    }
    if (showBuy) {
        ModalBottomSheet(
            onDismissRequest = { showBuy = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            modifier = Modifier.fillMaxHeight(0.9f)
        ) {
            BuyScreen(appState, prefillAmountUSD = prefillTradeAmount) { showBuy = false; prefillTradeAmount = 0.0 }
        }
    }
    if (showSell) {
        ModalBottomSheet(
            onDismissRequest = { showSell = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            modifier = Modifier.fillMaxHeight(0.9f)
        ) {
            SellScreen(appState, prefillAmountUSD = prefillTradeAmount) { showSell = false; prefillTradeAmount = 0.0 }
        }
    }
}

@Composable
fun ActionButton(title: String, icon: ImageVector, color: Color, modifier: Modifier = Modifier, pulse: Boolean = false, onClick: () -> Unit) {
    Box(modifier = modifier.height(56.dp)) {
        FilledTonalButton(
            onClick = onClick,
            modifier = Modifier.fillMaxSize(),
            colors = ButtonDefaults.filledTonalButtonColors(
                containerColor = color.copy(alpha = 0.1f),
                contentColor = color
            )
        ) {
            Icon(icon, contentDescription = title, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(6.dp))
            Text(title)
        }
        if (pulse) {
            key(pulse) {
                val transition = rememberInfiniteTransition(label = "btnPulse")
                val alpha by transition.animateFloat(
                    initialValue = 0f,
                    targetValue = 0.15f,
                    animationSpec = infiniteRepeatable(animation = tween(800, easing = EaseInOut), repeatMode = RepeatMode.Reverse),
                    label = "btnAlpha"
                )
                val pulseScale by transition.animateFloat(
                    initialValue = 1f,
                    targetValue = 1.04f,
                    animationSpec = infiniteRepeatable(animation = tween(800, easing = EaseInOut), repeatMode = RepeatMode.Reverse),
                    label = "btnScale"
                )
                Box(
                    Modifier
                        .matchParentSize()
                        .scale(pulseScale)
                        .clip(RoundedCornerShape(12.dp))
                        .background(color.copy(alpha = alpha))
                )
            }
        }
    }
}

@Composable
private fun PendingRow(text: String, txid: String?, context: android.content.Context) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth()
    ) {
        Text("\u231B", fontSize = 14.sp)
        Spacer(Modifier.width(6.dp))
        Text(text, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.weight(1f))
        txid?.let {
            TextButton(
                onClick = {
                    val intent = android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse("https://mempool.space/tx/$it"))
                    context.startActivity(intent)
                },
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp)
            ) {
                Text("View on explorer", fontSize = 12.sp)
            }
        }
    }
}
