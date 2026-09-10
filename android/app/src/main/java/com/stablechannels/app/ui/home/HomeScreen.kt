package com.stablechannels.app.ui.home

import android.Manifest
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.view.WindowCompat
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.CallMade
import androidx.compose.material.icons.automirrored.filled.CallReceived
import androidx.compose.material.icons.filled.ArrowCircleUp
import androidx.compose.material.icons.filled.ArrowCircleDown
import androidx.compose.material.icons.filled.TrendingUp
import androidx.compose.material.icons.filled.TrendingDown
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.automirrored.filled.OpenInNew
import androidx.compose.material3.*
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.scale
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import com.stablechannels.app.ui.theme.LocalDarkTheme
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.content.PermissionChecker
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.stablechannels.app.models.TradeRecord
import com.stablechannels.app.ui.history.OrderDetailBottomSheet
import com.stablechannels.app.models.PaymentRecord
import com.stablechannels.app.ui.history.PaymentDetailBottomSheet
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.components.StatusCapsule
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
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(appState: AppState, modifier: Modifier = Modifier) {
    val totalSats by appState.totalBalanceSats.collectAsState()
    val lightningSats by appState.lightningBalanceSats.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val sc by appState.stableChannel.collectAsState()
    val nativeSatsCached by appState.nativeSats.collectAsState()
    val lastRxTxid by appState.lastReceiveTxid.collectAsState()
    val lastCloseTxid by appState.lastCloseTxid.collectAsState()
    val statusMessage by appState.statusMessage.collectAsState()
    val onchainSats by appState.onchainBalanceSats.collectAsState()
    val hasReadyChannel by appState.hasReadyChannel.collectAsState()
    val spendableOnchainSats by appState.spendableOnchainSats.collectAsState()
    val isSyncing by appState.isSyncing.collectAsState()
    val isFlashing by appState.paymentFlash.collectAsState()
    val confirmationUpdateEpoch by appState.confirmationUpdateEpoch.collectAsState()
    val isChannelClosing by appState.isChannelClosingFlow.collectAsState()

    var showSend by remember { mutableStateOf(false) }
    var selectedTrade by remember { mutableStateOf<TradeRecord?>(null) }
    var selectedPayment by remember { mutableStateOf<PaymentRecord?>(null) }
    var showReceive by remember { mutableStateOf(false) }
    var showBuy by remember { mutableStateOf(false) }
    var showSell by remember { mutableStateOf(false) }
    var prefillTradeAmount by remember { mutableDoubleStateOf(0.0) }
    var showBTC by remember { mutableStateOf(false) }
    var latestPendingOnchainReceive by remember { mutableStateOf<PaymentRecord?>(null) }

    LaunchedEffect(isFlashing, confirmationUpdateEpoch, onchainSats, spendableOnchainSats) {
        latestPendingOnchainReceive = withContext(Dispatchers.IO) {
            appState.databaseService?.latestPendingOnchainReceive()
        }
    }

    // Auto-dismiss receive sheet when payment arrives
    val scrollState = rememberScrollState()
    LaunchedEffect(isFlashing) {
        if (isFlashing && showReceive) {
            showReceive = false
            scrollState.animateScrollTo(0)
        }
    }

    val context = LocalContext.current
    var notificationsEnabled by remember { mutableStateOf(true) }
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                notificationsEnabled = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PermissionChecker.PERMISSION_GRANTED
                } else true
                // Run blocking LDK calls off main thread
                kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.IO).launch {
                    // Pick up backing increments committed by the background stability
                    // service while this process was cached, before any save can clobber them.
                    appState.onForegroundResume()
                    appState.refreshBalances()
                    appState.detectOnchainDeposit()
                    appState.ensureLSPConnected()
                }
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    val totalUSD = (totalSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
    val scope = rememberCoroutineScope()

    var isRefreshing by remember { mutableStateOf(false) }
    val pullRefreshState = rememberPullToRefreshState()

    PullToRefreshBox(
        isRefreshing = isRefreshing,
        onRefresh = {
            scope.launch {
                isRefreshing = true
                val startTime = System.currentTimeMillis()
                appState.refreshBalances()
                appState.priceService.fetchPrice()
                appState.recordCurrentPrice()
                // Prevent spinner from flashing on instant fetches
                val elapsed = System.currentTimeMillis() - startTime
                if (elapsed < 500) kotlinx.coroutines.delay(500 - elapsed)
                isRefreshing = false
            }
        },
        state = pullRefreshState,
        indicator = {
            PullToRefreshDefaults.Indicator(
                modifier = Modifier.align(Alignment.TopCenter),
                isRefreshing = isRefreshing,
                state = pullRefreshState,
                color = MaterialTheme.colorScheme.primary,
                containerColor = MaterialTheme.colorScheme.surfaceVariant
            )
        },
        modifier = modifier.fillMaxSize()
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(scrollState)
                .padding(horizontal = 16.dp, vertical = 8.dp),
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
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.error),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Row(
                        modifier = Modifier.padding(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        Icon(Icons.Default.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.onError)
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                "Notifications Disabled",
                                color = MaterialTheme.colorScheme.onError,
                                fontWeight = FontWeight.SemiBold,
                                fontSize = 14.sp
                            )
                            Text(
                                "Enable notifications for stability payments",
                                color = MaterialTheme.colorScheme.onError.copy(alpha = 0.9f),
                                fontSize = 12.sp
                            )
                        }
                        Icon(
                            imageVector = Icons.Default.ChevronRight,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onError.copy(alpha = 0.7f),
                            modifier = Modifier.size(16.dp)
                        )
                    }
                }
                Spacer(Modifier.height(8.dp))
            }

            Spacer(Modifier.height(24.dp))

            // Balance (tap to toggle USD/BTC)
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier
                    .clickable { showBTC = !showBTC }
                    .paymentFlash(isFlashing)
            ) {
                Text(
                    text = "Total Balance",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(Modifier.height(4.dp))
                if (showBTC) {
                    RollingDigitText(
                        text = totalSats.btcSpacedFormatted() + " BTC",
                        style = MaterialTheme.typography.headlineLarge.copy(
                            fontSize = 32.sp,
                            fontWeight = FontWeight.Bold,
                            fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace
                        )
                    )
                } else {
                    if (btcPrice > 0) {
                        RollingDigitText(
                            text = totalUSD.usdFormatted(),
                            style = MaterialTheme.typography.headlineLarge.copy(
                                fontSize = 36.sp,
                                fontWeight = FontWeight.Bold
                            )
                        )
                    } else if (totalSats > 0) {
                        Text(
                            text = "Fetching price...",
                            fontSize = 24.sp,
                            fontWeight = FontWeight.Bold,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    } else {
                        Text(
                            text = "$0.00",
                            fontSize = 36.sp,
                            fontWeight = FontWeight.Bold
                        )
                    }
                }
                Text(
                    text = if (showBTC) {
                        if (btcPrice > 0) totalUSD.usdFormatted() else "—"
                    } else totalSats.btcSpacedFormatted() + " BTC",
                    fontSize = 14.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            Spacer(Modifier.height(8.dp))

            // Balance bar
            if (lightningSats > 0) {
                BalanceBar(
                    stableUSD = sc.expectedUSD.amount,
                    nativeSats = nativeSatsCached,
                    totalSats = lightningSats,
                    btcPrice = btcPrice,
                    maxSellUSD = (appState.tradeService?.maxSellCents(sc, appState.priceService.accountingPrice.value) ?: 0L) / 100.0,
                    showBtcFormat = showBTC,
                    modifier = Modifier.padding(horizontal = 18.dp),
                    onDragStarted = { appState.ensureLSPConnected() },
                    onTradeRequest = if (hasReadyChannel) { direction, amountUSD ->
                        prefillTradeAmount = amountUSD
                        if (direction == TradeDirection.BUY) showBuy = true else showSell = true
                    } else null
                )
                Spacer(Modifier.height(12.dp))
            }

            // Syncing indicator
            if (isSyncing) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(14.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.primary
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        "Syncing...",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Spacer(Modifier.height(8.dp))
            }

            // On-chain section
            if (onchainSats > 0) {
                val onchainUSD = (onchainSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
                val isSweeping by appState.isSpliceInFlightFlow.collectAsState()

                val pendingReceive = latestPendingOnchainReceive
                val pendingReceiveTxid = pendingReceive?.txid
                val hasPendingOnchainReceive = pendingReceive != null
                val receiveConfirmations = pendingReceive?.confirmations
                val receiveRequiredConfirmations = pendingReceive?.let {
                    AppState.requiredConfirmationsForType(it.paymentType)
                }
                // Only the "Move" + incoming-deposit combo is busy enough (3 rows) to warrant
                // collapsing; every other state is already a single compact row like iOS.
                val isBusyOnchainState = hasReadyChannel && spendableOnchainSats > 0 && hasPendingOnchainReceive
                var onchainExpanded by remember { mutableStateOf(false) }
                val chevronRotation by animateFloatAsState(if (onchainExpanded) 90f else 0f, label = "onchainChevron")

                Card(
                    modifier = Modifier.fillMaxWidth(),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
                    elevation = CardDefaults.cardElevation(defaultElevation = 0.dp)
                ) {
                    Column(
                        (if (isBusyOnchainState) Modifier.clickable { onchainExpanded = !onchainExpanded } else Modifier)
                            .padding(horizontal = 16.dp, vertical = 12.dp)
                    ) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text("Onchain Account", style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold)
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    onchainUSD.usdFormatted(),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                                if (isBusyOnchainState) {
                                    Icon(
                                        Icons.Filled.ChevronRight,
                                        contentDescription = if (onchainExpanded) "Collapse" else "Expand",
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                        modifier = Modifier
                                            .padding(start = 2.dp)
                                            .size(18.dp)
                                            .rotate(chevronRotation)
                                    )
                                }
                            }
                        }
                        if (isSweeping) {
                            // 1. Splice-in in progress
                            Spacer(Modifier.height(10.dp))
                            PendingRow("Move pending...", appState.spliceTxid, context)
                        } else if (isChannelClosing) {
                            // 2. Channel closing
                            Spacer(Modifier.height(10.dp))
                            PendingRow("Channel closing\u2026", lastCloseTxid, context)
                        } else if (hasReadyChannel && spendableOnchainSats > 0) {
                            // Has channel + confirmed funds — offer to sweep
                            if (hasPendingOnchainReceive) {
                                // Busy state — both "Move" and an incoming deposit at once.
                                // Collapsed: a one-line gist. Expanded: Move button + the full
                                // receiving-deposit detail (progress ring + explorer link).
                                if (!onchainExpanded) {
                                    Spacer(Modifier.height(4.dp))
                                    val suffix = if (receiveConfirmations != null && receiveRequiredConfirmations != null) {
                                        " ($receiveConfirmations/$receiveRequiredConfirmations)"
                                    } else ""
                                    Text(
                                        "Ready to move \u00b7 Receiving onchain$suffix",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 1,
                                        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis
                                    )
                                }
                                AnimatedVisibility(
                                    visible = onchainExpanded,
                                    enter = expandVertically(animationSpec = tween(200)) + fadeIn(animationSpec = tween(200)),
                                    exit = shrinkVertically(animationSpec = tween(200)) + fadeOut(animationSpec = tween(150))
                                ) {
                                    Column {
                                        Spacer(Modifier.height(10.dp))
                                        Row(
                                            modifier = Modifier.fillMaxWidth(),
                                            horizontalArrangement = Arrangement.SpaceBetween,
                                            verticalAlignment = Alignment.CenterVertically
                                        ) {
                                            Text("Move to Lightning Account", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                            FilledTonalButton(
                                                onClick = {
                                                    scope.launch(Dispatchers.IO) {
                                                        appState.sweepToChannel()
                                                    }
                                                },
                                                shape = RoundedCornerShape(12.dp),
                                                colors = ButtonDefaults.filledTonalButtonColors(
                                                    containerColor = MaterialTheme.colorScheme.secondary.copy(alpha = if (isSystemInDarkTheme()) 0.15f else 0.15f),
                                                    contentColor = MaterialTheme.colorScheme.secondary
                                                ),
                                                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 0.dp),
                                                modifier = Modifier.height(32.dp)
                                            ) {
                                                Text("Move", fontSize = 13.sp)
                                            }
                                        }
                                        Spacer(Modifier.height(8.dp))
                                        PendingRow(
                                            "Receiving onchain...",
                                            pendingReceiveTxid,
                                            context,
                                            amountSats = pendingReceive?.amountSats,
                                            confirmations = receiveConfirmations,
                                            requiredConfirmations = receiveRequiredConfirmations
                                        )
                                    }
                                }
                            } else {
                                // Simple case — nothing else pending, match iOS's plain layout.
                                Spacer(Modifier.height(10.dp))
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    horizontalArrangement = Arrangement.SpaceBetween,
                                    verticalAlignment = Alignment.CenterVertically
                                ) {
                                    Text("Move to Lightning Account", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    FilledTonalButton(
                                        onClick = {
                                            scope.launch(Dispatchers.IO) {
                                                appState.sweepToChannel()
                                            }
                                        },
                                        shape = RoundedCornerShape(12.dp),
                                        colors = ButtonDefaults.filledTonalButtonColors(
                                            // Same tonal treatment as the Send/Receive buttons below
                                            // (ActionButton), just tinted with the app's amber "native
                                            // BTC" accent so this on-chain action still reads as distinct.
                                            containerColor = MaterialTheme.colorScheme.secondary.copy(alpha = if (isSystemInDarkTheme()) 0.15f else 0.15f),
                                            contentColor = MaterialTheme.colorScheme.secondary
                                        ),
                                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 0.dp),
                                        modifier = Modifier.height(32.dp)
                                    ) {
                                        Text("Move", fontSize = 13.sp)
                                    }
                                }
                            }
                        } else if (spendableOnchainSats == 0L) {
                            // 3. Unconfirmed deposit (with or without channel)
                            Spacer(Modifier.height(10.dp))
                            val pendingCloseId = appState.pendingClosePaymentId
                            // Prefer close txid if known — pendingClosePaymentId may already be
                            // cleared by detectOnchainDeposit even while funds are still unconfirmed
                            val effectiveTxid = if (lastCloseTxid != null) {
                                lastCloseTxid
                            } else {
                                pendingReceiveTxid ?: lastRxTxid
                            }
                            val isClosePending = pendingCloseId != null || lastCloseTxid != null
                            // Use a receive-specific label when we have an explicit pending onchain row.
                            val text = when {
                                isClosePending && effectiveTxid != null -> "Channel closing\u2026"
                                isClosePending -> "Channel closed"
                                hasPendingOnchainReceive -> "Receiving onchain..."
                                else -> "Deposit confirming..."
                            }
                            PendingRow(
                                text,
                                effectiveTxid,
                                context,
                                amountSats = if (hasPendingOnchainReceive && !isClosePending) pendingReceive?.amountSats else null,
                                confirmations = if (hasPendingOnchainReceive && !isClosePending) receiveConfirmations else null,
                                requiredConfirmations = if (hasPendingOnchainReceive && !isClosePending) receiveRequiredConfirmations else null
                            )
                            if (!hasReadyChannel) {
                                Spacer(Modifier.height(4.dp))
                                Text("Receive a payment over Lightning to activate your account.",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        } else {
                            // 4. No channel, confirmed deposit — just needs Lightning
                            Spacer(Modifier.height(8.dp))
                            Text("Receive a payment over Lightning to activate your account.",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            }

            // Price chart
            if (btcPrice > 0) {
                PriceChart(
                    appState = appState,
                    databaseService = appState.databaseService,
                    currentPrice = btcPrice
                )
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
                val sendColor = if (isSystemInDarkTheme()) Color(0xFF0A84FF) else Color(0xFF007AFF)
                val receiveColor = if (isSystemInDarkTheme()) Color(0xFF30D158) else Color(0xFF34C759)
                ActionButton("Send", Icons.Default.ArrowCircleUp, sendColor, Modifier.weight(1f)) { showSend = true }
                ActionButton("Receive", Icons.Default.ArrowCircleDown, receiveColor, Modifier.weight(1f), pulse = !hasReadyChannel) { showReceive = true }
            }

            Spacer(Modifier.height(8.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                val buyColor = if (isSystemInDarkTheme()) Color(0xFFFF9F0A) else Color(0xFFFF9500)
                val sellColor = if (isSystemInDarkTheme()) Color(0xFFBF5AF2) else Color(0xFFAF52DE)
                ActionButton("USD → BTC", Icons.Default.ArrowCircleUp, buyColor, Modifier.weight(1f), rotation = 45f, enabled = hasReadyChannel) { showBuy = true }
                ActionButton("BTC → USD", Icons.Default.ArrowCircleDown, sellColor, Modifier.weight(1f), rotation = -45f, enabled = hasReadyChannel) { showSell = true }
            }

            // Status capsule
            if (statusMessage.isNotEmpty()) {
                StatusCapsule(
                    message = statusMessage,
                    onClick = {
                        val msg = statusMessage.lowercase()
                        val isTrade = msg.contains("buy") || msg.contains("sell") || msg.contains("trade") || msg.contains("order")
                        val isPayment = msg.contains("payment") || msg.contains("swap") || msg.contains("channel") || msg.contains("moving")
                        
                        if (isTrade || isPayment) {
                            scope.launch(kotlinx.coroutines.Dispatchers.IO) {
                                if (isTrade) {
                                    val trades = appState.databaseService?.getRecentTrades(1)
                                    if (!trades.isNullOrEmpty()) {
                                        selectedTrade = trades.first()
                                    }
                                } else {
                                    val payments = appState.databaseService?.getRecentPayments(1)
                                    if (!payments.isNullOrEmpty()) {
                                        selectedPayment = payments.first()
                                    }
                                }
                            }
                        } else {
                            appState.setStatus("")
                        }
                    }
                )
            }
            // Bottom padding for nav bar
            Spacer(Modifier.height(80.dp))
        }
    }

    // Bottom sheets
    if (showSend) {
        ModalBottomSheet(
            onDismissRequest = { showSend = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            containerColor = if (isSystemInDarkTheme()) Color.Black else Color.White,
            contentWindowInsets = @Composable { WindowInsets(0, 0, 0, 0) }
        ) {
            SheetEdgeToEdgeEffect()
            Box(modifier = Modifier.fillMaxHeight(0.9f)) {
                SendScreen(appState) { showSend = false }
            }
        }
    }
    if (showReceive) {
        ModalBottomSheet(
            onDismissRequest = { showReceive = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            containerColor = if (isSystemInDarkTheme()) Color.Black else Color.White,
            contentWindowInsets = @Composable { WindowInsets(0, 0, 0, 0) }
        ) {
            SheetEdgeToEdgeEffect()
            Box(modifier = Modifier.fillMaxHeight(0.9f)) {
                ReceiveScreen(appState) { showReceive = false }
            }
        }
    }
    if (showBuy) {
        ModalBottomSheet(
            onDismissRequest = { showBuy = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            containerColor = if (isSystemInDarkTheme()) Color.Black else Color.White,
            contentWindowInsets = @Composable { WindowInsets(0, 0, 0, 0) }
        ) {
            SheetEdgeToEdgeEffect()
            Box(modifier = Modifier.fillMaxHeight(0.9f)) {
                BuyScreen(appState, prefillAmountUSD = prefillTradeAmount) { showBuy = false; prefillTradeAmount = 0.0 }
            }
        }
    }
    if (showSell) {
        ModalBottomSheet(
            onDismissRequest = { showSell = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            containerColor = if (isSystemInDarkTheme()) Color.Black else Color.White,
            contentWindowInsets = @Composable { WindowInsets(0, 0, 0, 0) }
        ) {
            SheetEdgeToEdgeEffect()
            Box(modifier = Modifier.fillMaxHeight(0.9f)) {
                SellScreen(appState, prefillAmountUSD = prefillTradeAmount) { showSell = false; prefillTradeAmount = 0.0 }
            }
        }
    }

    selectedTrade?.let { trade ->
        OrderDetailBottomSheet(trade = trade, onDismiss = { selectedTrade = null })
    }
    selectedPayment?.let { payment ->
        PaymentDetailBottomSheet(
            payment = payment,
            currentPrice = btcPrice,
            onDismiss = { selectedPayment = null }
        )
    }
}

@Composable
fun ActionButton(title: String, icon: ImageVector, color: Color, modifier: Modifier = Modifier, rotation: Float = 0f, pulse: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    Box(modifier = modifier.defaultMinSize(minHeight = 52.dp).clip(RoundedCornerShape(12.dp))) {
        FilledTonalButton(
            onClick = onClick,
            modifier = Modifier.fillMaxWidth().defaultMinSize(minHeight = 52.dp),
            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 8.dp),
            shape = RoundedCornerShape(12.dp),
            enabled = enabled,
            colors = ButtonDefaults.filledTonalButtonColors(
                containerColor = color.copy(alpha = if (isSystemInDarkTheme()) 0.1f else 0.15f),
                contentColor = color,
                disabledContainerColor = color.copy(alpha = 0.05f),
                disabledContentColor = color.copy(alpha = 0.3f)
            )
        ) {
            Icon(icon, contentDescription = title, modifier = Modifier.size(20.dp).rotate(rotation))
            Spacer(Modifier.width(6.dp))
            Text(title)
        }
        if (pulse) {
            key(pulse) {
                val transition = rememberInfiniteTransition(label = "btnPulse")
                val alpha by transition.animateFloat(
                    initialValue = 0f,
                    targetValue = 0.2f,
                    animationSpec = infiniteRepeatable(animation = tween(800, easing = EaseInOut), repeatMode = RepeatMode.Reverse),
                    label = "btnAlpha"
                )
                Box(
                    Modifier
                        .matchParentSize()
                        .clip(RoundedCornerShape(12.dp))
                        .background(color.copy(alpha = alpha))
                )
            }
        }
    }
}

@Composable
private fun PendingRow(
    text: String,
    txid: String?,
    context: android.content.Context,
    amountSats: Long? = null,
    confirmations: Int? = null,
    requiredConfirmations: Int? = null
) {
    // A real confirmation count gives concrete progress ("2/6 confirmations") instead of
    // a static hourglass that never changes for up to an hour on a fresh onchain deposit.
    // Only show progress once a txid is known — before that the confirmation tracker has
    // nothing to count yet, and an empty ring next to "0/6" would read as stalled rather
    // than as not-yet-detected.
    val progress = if (txid != null && confirmations != null && requiredConfirmations != null && requiredConfirmations > 0) {
        (confirmations.toFloat() / requiredConfirmations.toFloat()).coerceIn(0f, 1f)
    } else null

    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth()
    ) {
        if (progress != null) {
            CircularProgressIndicator(
                progress = { progress },
                modifier = Modifier.size(14.dp),
                strokeWidth = 2.dp,
                color = MaterialTheme.colorScheme.primary,
                trackColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.15f)
            )
        } else {
            Text("\u231B", fontSize = 14.sp)
        }
        Spacer(Modifier.width(8.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            val amountPrefix = if (amountSats != null) "${amountSats.btcSpacedFormatted()} BTC \u00b7 " else ""
            val caption = when {
                progress != null -> "$amountPrefix$confirmations/$requiredConfirmations confirmations"
                txid == null -> "${amountPrefix}pending confirmation"
                amountSats != null -> amountPrefix.removeSuffix(" \u00b7 ")
                else -> null
            }
            if (caption != null) {
                Text(caption, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f))
            }
        }
        if (txid != null) {
            IconButton(
                onClick = {
                    val intent = android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse("https://mempool.space/tx/${txid.substringBefore(":")}"))
                    context.startActivity(intent)
                },
                modifier = Modifier.size(28.dp)
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.OpenInNew,
                    contentDescription = "View on explorer",
                    modifier = Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.primary
                )
            }
        }
    }
}

// Edge-to-edge for a ModalBottomSheet's dialog window. Android 15+ enforces
// transparent system bars, so the (deprecated) color setters only run on older
// versions, where they are still the only way to clear the bars.
@Composable
private fun SheetEdgeToEdgeEffect() {
    val view = LocalView.current
    val isDark = LocalDarkTheme.current
    DisposableEffect(view, isDark) {
        var context = view.context
        var dialog: android.app.Dialog? = null
        while (context is android.content.ContextWrapper) {
            if (context is android.app.Dialog) {
                dialog = context
                break
            }
            context = context.baseContext
        }
        val window = dialog?.window
        if (window != null) {
            WindowCompat.setDecorFitsSystemWindows(window, false)
            val insetsController = WindowCompat.getInsetsController(window, view)
            insetsController.isAppearanceLightStatusBars = !isDark
            insetsController.isAppearanceLightNavigationBars = !isDark
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) {
                @Suppress("DEPRECATION")
                window.navigationBarColor = android.graphics.Color.TRANSPARENT
                @Suppress("DEPRECATION")
                window.statusBarColor = android.graphics.Color.TRANSPARENT
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                window.isNavigationBarContrastEnforced = false
            }
            window.setLayout(
                android.view.ViewGroup.LayoutParams.MATCH_PARENT,
                android.view.ViewGroup.LayoutParams.MATCH_PARENT
            )
        }
        onDispose {}
    }
}
