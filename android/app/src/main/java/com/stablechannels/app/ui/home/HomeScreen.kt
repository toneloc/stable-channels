package com.stablechannels.app.ui.home

import android.Manifest
import android.content.Intent
import android.os.Build
import android.provider.Settings
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
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.content.PermissionChecker
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(appState: AppState, modifier: Modifier = Modifier) {
    val totalSats by appState.totalBalanceSats.collectAsState()
    val lightningSats by appState.lightningBalanceSats.collectAsState()
    val btcPrice by appState.priceService.currentPrice.collectAsState()
    val sc by appState.stableChannel.collectAsState()
    val nativeSatsCached by appState.nativeSats.collectAsState()
    val statusMessage by appState.statusMessage.collectAsState()
    val onchainSats by appState.onchainBalanceSats.collectAsState()
    val hasReadyChannel by appState.hasReadyChannel.collectAsState()
    val spendableOnchainSats by appState.spendableOnchainSats.collectAsState()
    val isSyncing by appState.isSyncing.collectAsState()
    val isFlashing by appState.paymentFlash.collectAsState()
    val isChannelClosing by appState.isChannelClosingFlow.collectAsState()

    var showSend by remember { mutableStateOf(false) }
    var showReceive by remember { mutableStateOf(false) }
    var showBuy by remember { mutableStateOf(false) }
    var showSell by remember { mutableStateOf(false) }
    var prefillTradeAmount by remember { mutableDoubleStateOf(0.0) }
    var showBTC by remember { mutableStateOf(false) }

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
                .padding(16.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            // Title
            Text(
                text = "Stable Channels",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
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

            Spacer(Modifier.height(16.dp))

            // Balance bar
            if (lightningSats > 0) {
                BalanceBar(
                    stableUSD = sc.expectedUSD.amount,
                    nativeSats = nativeSatsCached,
                    totalSats = lightningSats,
                    btcPrice = btcPrice,
                    modifier = Modifier.padding(horizontal = 24.dp),
                    onDragStarted = { appState.ensureLSPConnected() },
                    onTradeRequest = if (hasReadyChannel) { direction, amountUSD ->
                        prefillTradeAmount = amountUSD
                        if (direction == TradeDirection.BUY) showBuy = true else showSell = true
                    } else null
                )
                Spacer(Modifier.height(16.dp))
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
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(Modifier.width(6.dp))
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
                val isSweeping = appState.isSpliceInFlight

                Card(
                    modifier = Modifier.fillMaxWidth(),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
                    elevation = CardDefaults.cardElevation(defaultElevation = 0.dp)
                ) {
                    Column(Modifier.padding(12.dp)) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text("Onchain", style = MaterialTheme.typography.labelMedium)
                            Text(
                                onchainUSD.usdFormatted(),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        if (isSweeping) {
                            // 1. Splice-in in progress
                            Spacer(Modifier.height(8.dp))
                            PendingRow("Swap pending...", appState.spliceTxid, context)
                        } else if (isChannelClosing) {
                            // 2. Channel closing
                            Spacer(Modifier.height(8.dp))
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                modifier = Modifier.fillMaxWidth()
                            ) {
                                Icon(
                                    Icons.Default.Warning,
                                    contentDescription = "Closing",
                                    tint = Color(0xFFF59E0B),
                                    modifier = Modifier.size(16.dp)
                                )
                                Spacer(Modifier.width(6.dp))
                                Text(
                                    "Channel closing — funds will arrive onchain",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                        } else if (hasReadyChannel && spendableOnchainSats > 0) {
                            // Has channel + confirmed funds — offer to sweep
                            Spacer(Modifier.height(8.dp))
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Column {
                                    Text("Move to Stability", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    Text("and Spending Account", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                                Button(
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
                    appState = appState,
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
                ActionButton("Send", Icons.Default.ArrowCircleUp, Color(0xFF3B82F6), Modifier.weight(1f)) { showSend = true }
                ActionButton("Receive", Icons.Default.ArrowCircleDown, Color(0xFF10B981), Modifier.weight(1f), pulse = !hasReadyChannel) { showReceive = true }
            }

            Spacer(Modifier.height(8.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                ActionButton("USD → BTC", Icons.Default.ArrowCircleUp, Color(0xFFF59E0B), Modifier.weight(1f), rotation = 45f, enabled = hasReadyChannel) { showBuy = true }
                ActionButton("BTC → USD", Icons.Default.ArrowCircleDown, Color(0xFF8B5CF6), Modifier.weight(1f), rotation = -45f, enabled = hasReadyChannel) { showSell = true }
            }

            // Status capsule
            if (statusMessage.isNotEmpty()) {
                Spacer(Modifier.height(12.dp))
                StatusCapsule(
                    message = statusMessage
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
            val view = LocalView.current
            DisposableEffect(view) {
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
                    window.navigationBarColor = android.graphics.Color.TRANSPARENT
                    window.statusBarColor = android.graphics.Color.TRANSPARENT
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
            val view = LocalView.current
            DisposableEffect(view) {
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
                    window.navigationBarColor = android.graphics.Color.TRANSPARENT
                    window.statusBarColor = android.graphics.Color.TRANSPARENT
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
            val view = LocalView.current
            DisposableEffect(view) {
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
                    window.navigationBarColor = android.graphics.Color.TRANSPARENT
                    window.statusBarColor = android.graphics.Color.TRANSPARENT
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
            val view = LocalView.current
            DisposableEffect(view) {
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
                    window.navigationBarColor = android.graphics.Color.TRANSPARENT
                    window.statusBarColor = android.graphics.Color.TRANSPARENT
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
            Box(modifier = Modifier.fillMaxHeight(0.9f)) {
                SellScreen(appState, prefillAmountUSD = prefillTradeAmount) { showSell = false; prefillTradeAmount = 0.0 }
            }
        }
    }
}

@Composable
fun ActionButton(title: String, icon: ImageVector, color: Color, modifier: Modifier = Modifier, rotation: Float = 0f, pulse: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    Box(modifier = modifier.height(56.dp).clip(RoundedCornerShape(12.dp))) {
        FilledTonalButton(
            onClick = onClick,
            modifier = Modifier.fillMaxSize(),
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
                    val intent = android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse("https://mempool.space/tx/${it.substringBefore(":")}"))
                    context.startActivity(intent)
                },
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp)
            ) {
                Text("View on explorer", fontSize = 12.sp)
            }
        }
    }
}
