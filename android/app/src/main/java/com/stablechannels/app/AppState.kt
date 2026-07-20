package com.stablechannels.app

import android.content.Context
import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.google.firebase.messaging.FirebaseMessaging
import com.stablechannels.app.models.*
import com.stablechannels.app.push.FCMService
import com.stablechannels.app.push.StabilityProcessingService
import com.stablechannels.app.services.CloseTxidResolver
import com.stablechannels.app.services.*
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit
import okhttp3.Request
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import org.lightningdevkit.ldknode.*
import java.io.File
import kotlin.math.abs
import kotlin.math.roundToLong

enum class Phase {
    LOADING, ONBOARDING, SYNCING, WALLET, ERROR
}

class AppState(private val context: Context) : ViewModel() {

    val nodeService = NodeService(context)
    val priceService = PriceService()
    var databaseService: DatabaseService? = null
        private set
    var tradeService: TradeService? = null
        private set

    private val _phase = MutableStateFlow(Phase.LOADING)
    val phase: StateFlow<Phase> = _phase

    private val _isSyncing = MutableStateFlow(false)
    val isSyncing: StateFlow<Boolean> = _isSyncing

    private var isInitialized = false
    private var backgroundStopJob: Job? = null

    @Volatile
    var isWaitingForPayment = false

    private val _errorMessage = MutableStateFlow("")
    val errorMessage: StateFlow<String> = _errorMessage

    private val _stableChannel = MutableStateFlow(StableChannel.DEFAULT)
    val stableChannel: StateFlow<StableChannel> = _stableChannel

    private val _statusMessage = MutableStateFlow("")
    val statusMessage: StateFlow<String> = _statusMessage

    // Track last payment result for SendScreen UI updates
    private val _lastPaymentResult = MutableStateFlow<String?>(null)
    val lastPaymentResult: StateFlow<String?> = _lastPaymentResult

    fun clearLastPaymentResult() {
        _lastPaymentResult.value = null
    }

    private val _lightningBalanceSats: MutableStateFlow<Long>
    val lightningBalanceSats: StateFlow<Long> get() = _lightningBalanceSats

    private val _onchainBalanceSats: MutableStateFlow<Long>
    val onchainBalanceSats: StateFlow<Long> get() = _onchainBalanceSats

    private val _totalBalanceSats: MutableStateFlow<Long>
    val totalBalanceSats: StateFlow<Long> get() = _totalBalanceSats
    private val _hasReadyChannel = MutableStateFlow(false)
    val hasReadyChannel: StateFlow<Boolean> get() = _hasReadyChannel

    private val _onchainReceiveAddress = MutableStateFlow<String?>(null)
    val onchainReceiveAddress: StateFlow<String?> get() = _onchainReceiveAddress

    private val _lastReceiveTxid = MutableStateFlow<String?>(null)
    val lastReceiveTxid: StateFlow<String?> get() = _lastReceiveTxid

    private val _lastCloseTxid = MutableStateFlow<String?>(null)
    val lastCloseTxid: StateFlow<String?> get() = _lastCloseTxid

    fun setLastCloseTxid(txid: String?) {
        _lastCloseTxid.value = txid
        val editor = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
        if (txid != null) {
            editor.putString("last_close_txid", txid)
            editor.putLong("last_close_txid_at", System.currentTimeMillis())
        } else {
            editor.remove("last_close_txid")
            editor.remove("last_close_txid_at")
        }
        editor.apply()
    }


    private val _spendableOnchainSats = MutableStateFlow(
        context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).getLong("cached_spendable_sats", 0L)
    )
    val spendableOnchainSats: StateFlow<Long> = _spendableOnchainSats

    private val _nativeSats: MutableStateFlow<Long>
    val nativeSats: StateFlow<Long> get() = _nativeSats

    init {
        val prefs = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
        val cachedLightning = prefs.getLong("cached_lightning_sats", 0L)
        val cachedOnchain = prefs.getLong("cached_onchain_sats", 0L)
        _lightningBalanceSats = MutableStateFlow(cachedLightning)
        _onchainBalanceSats = MutableStateFlow(cachedOnchain)
        _totalBalanceSats = MutableStateFlow(cachedLightning + cachedOnchain)
                _nativeSats = MutableStateFlow(prefs.getLong("cached_native_sats", 0L))
        _onchainReceiveAddress.value = prefs.getString("onchain_receive_address", null)
        _lastReceiveTxid.value = prefs.getString("last_receive_txid", null)

        val closeAt = prefs.getLong("last_close_txid_at", 0L)
        if (System.currentTimeMillis() - closeAt < 7 * 86400 * 1000L) {
            _lastCloseTxid.value = prefs.getString("last_close_txid", null)
        } else {
            prefs.edit()
                .remove("last_close_txid")
                .remove("last_close_txid_at")
                .apply()
        }


        // Restore cached channel state so UI shows correct slider position immediately
        val cachedChannelId = prefs.getString("cached_channel_id", null)
        val cachedUserChannelId = prefs.getString("cached_user_channel_id", null)
        val cachedExpectedUsd = prefs.getFloat("cached_expected_usd", 0f)
        if (cachedUserChannelId != null) {
            _stableChannel.value = StableChannel.DEFAULT.copy(
                channelId = cachedChannelId ?: "",
                userChannelId = cachedUserChannelId,
                expectedUSD = USD(cachedExpectedUsd.toDouble())
            )
        }
    }

    private val _pendingTradePayments = MutableStateFlow<Map<String, PendingTradePayment>>(emptyMap())
    val pendingTradePayments: StateFlow<Map<String, PendingTradePayment>> = _pendingTradePayments
    var pendingSplice: PendingSplice? = null
    private val _isChannelClosing = MutableStateFlow(false)
    val isChannelClosingFlow: StateFlow<Boolean> = _isChannelClosing
    var isChannelClosing: Boolean
        get() = _isChannelClosing.value
        set(value) { 
            _isChannelClosing.value = value
            if (value) {
                channelCloseJob?.cancel()
                channelCloseJob = viewModelScope.launch(Dispatchers.IO) {
                    while (isActive && _isChannelClosing.value) {
                        delay(10_000)
                        refreshBalances()
                    }
                }
            } else {
                channelCloseJob?.cancel()
            }
        }
    var pendingClosePaymentId: String? = null
    var spliceTxid: String? = null
    var fundingTxid: String? = null
        set(value) {
            field = value
            context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                .putString("funding_txid", value).apply()
        }

    private val _paymentFlash = MutableStateFlow(false)
    val paymentFlash: StateFlow<Boolean> = _paymentFlash


    private val _isSpliceInFlight = MutableStateFlow(false)
    val isSpliceInFlightFlow: StateFlow<Boolean> get() = _isSpliceInFlight
    /** True when any splice (in or out) is in flight — prevents concurrent splices. */
    val isSpliceInFlight: Boolean get() = _isSpliceInFlight.value
    private var isSweeping: Boolean
        get() = _isSpliceInFlight.value
        set(value) { _isSpliceInFlight.value = value }

    private var sweepOnchainStart: Long = 0
    private var prevOnchainSats: Long = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
        .getLong("cached_onchain_sats", 0L)
    private var stabilityJob: Job? = null
    private var heartbeatJob: Job? = null
    private var pendingDepositJob: Job? = null
    private var channelCloseJob: Job? = null
    private var nodeStartRetryJob: Job? = null
    private var spliceConfirmationJob: Job? = null
    private var monitoredSpliceTxid: String? = null
    /** Resolved esplora URL — Blockstream primary, mempool.space fallback. */
    var chainUrl: String = Constants.PRIMARY_CHAIN_URL
        private set

    /** Cached chart data — survives tab switches since AppState is a ViewModel. */
    var cachedChartHourly: List<com.stablechannels.app.models.PriceRecord> = emptyList()
    var cachedChartDaily: List<com.stablechannels.app.models.PriceRecord> = emptyList()
    var chartDataLoaded = false

    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(4, TimeUnit.SECONDS)
        .readTimeout(4, TimeUnit.SECONDS)
        .callTimeout(6, TimeUnit.SECONDS)
        .build()

    fun start() {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                databaseService = DatabaseService(context)
                launch { databaseService?.seedHistoricalPrices() }
                launch { backfillHourlyPrices() }
                tradeService = TradeService(nodeService)

                val auditPath = File(Constants.userDataDir(context), "audit_log.txt").absolutePath
                AuditService.setLogPath(auditPath)

                // Load cached channel state so UI has correct slider/values immediately
                loadChannelFromDB()
                priceService.startAutoRefresh()

                // Resolve best esplora endpoint before starting node
                chainUrl = resolveChainUrl()

                // Consume LDK events. Each event carries a CompletableDeferred; completing it
                // unblocks NodeService so it can call n.eventHandled() and fetch the next event.
                launch {
                    for ((event, ack) in nodeService.eventChannel) {
                        var succeeded = false
                        try {
                            handleEvent(event)
                            succeeded = true
                        } catch (e: Exception) {
                            Log.e("AppState", "Event handler threw — not acknowledging", e)
                        } finally {
                            ack.complete(succeeded)
                        }
                    }
                }

                val seedFile = File(Constants.userDataDir(context), "keys_seed")
                val seedPhraseFile = File(Constants.userDataDir(context), "seed_phrase")
                if (seedFile.exists() || seedPhraseFile.exists()) {
                    val hasCachedChannel = _stableChannel.value.userChannelId.isNotEmpty()
                    if (hasCachedChannel) {
                        _phase.value = Phase.WALLET
                        _isSyncing.value = true
                    } else {
                        _phase.value = Phase.SYNCING
                    }
                    if (!waitForBackgroundService()) {
                        _isSyncing.value = false
                        scheduleNodeStartRetry()
                        return@launch
                    }
                    loadChannelFromDB()  // reload — SPS may have incremented backingSats while we waited
                    nodeService.start(ldkNetwork(), chainUrl, null)
                    nodeStartRetryJob?.cancel()
                    nodeStartRetryJob = null
                    _phase.value = Phase.WALLET
                    _isSyncing.value = false
                    refreshBalances()
                    // Restore fundingTxid
                    fundingTxid = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
                        .getString("funding_txid", null)
                    resumePendingSpliceConfirmation()
                    // Restore channel-closing state if a close is still pending on-chain
                    val pendingCloseId = databaseService?.getPendingChannelClosePaymentId()
                    if (pendingCloseId != null) {
                        pendingClosePaymentId = pendingCloseId
                        isChannelClosing = true
                        if (_lastCloseTxid.value == null) {
                            val dbTxid = databaseService?.getPaymentTxid(pendingCloseId)
                            if (!dbTxid.isNullOrEmpty()) {
                                setLastCloseTxid(dbTxid)
                            } else {
                                // Resume background resolver if it hasn't found the TX yet
                                val closeFundingTxid = fundingTxid
                                if (closeFundingTxid != null && databaseService != null) {
                                    val resolver = CloseTxidResolver(
                                        chainURLs = listOf(Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL),
                                        onResolved = { _, txid ->
                                            Log.d("AppState", "Close TX resolved on restart: $txid")
                                            setLastCloseTxid(txid)
                                        }
                                    )
                                    viewModelScope.launch(Dispatchers.IO) {
                                        resolver.resolve(
                                            paymentId = pendingCloseId,
                                            fundingTxid = closeFundingTxid,
                                            vout = 0,
                                            databaseService = databaseService!!
                                        )
                                    }
                                }
                            }
                        }
                    }
                    detectOnchainDeposit()
                    
                    // Resume pending deposit polling if an unconfirmed deposit exists from a previous session
                    if (_onchainBalanceSats.value > 0L && _spendableOnchainSats.value == 0L) {
                        startPendingDepositPolling()
                    }
                    
                    reregisterPushTokenIfNeeded()
                    processPendingPushPayment()
                    startStabilityTimer()
                    // Ensure LSP connection after startup settles
                    viewModelScope.launch(Dispatchers.IO) {
                        delay(3000)
                        ensureLSPConnected()
                    }
                } else {
                    // New wallet — auto-create
                    _phase.value = Phase.SYNCING
                    nodeService.start(ldkNetwork(), chainUrl, null)
                    _phase.value = Phase.WALLET
                    refreshBalances()
                    reregisterPushTokenIfNeeded()
                    startStabilityTimer()
                    viewModelScope.launch(Dispatchers.IO) {
                        delay(3000)
                        ensureLSPConnected()
                    }
                }
            } catch (e: Exception) {
                _errorMessage.value = e.message ?: "Unknown error"
                _phase.value = Phase.ERROR
            }
        }
    }

    fun createWallet(mnemonic: String?) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                _phase.value = Phase.SYNCING
                nodeService.start(ldkNetwork(), chainUrl, mnemonic)
                _phase.value = Phase.WALLET
                refreshBalances()
                reregisterPushTokenIfNeeded()
                startStabilityTimer()
            } catch (e: Exception) {
                _errorMessage.value = e.message ?: "Failed to create wallet"
                _phase.value = Phase.ERROR
            }
        }
    }

    fun stop() {
        cancelBackgroundStop()
        stabilityJob?.cancel()
        heartbeatJob?.cancel()
        pendingDepositJob?.cancel()
        nodeStartRetryJob?.cancel()
        nodeStartRetryJob = null
        spliceConfirmationJob?.cancel()
        spliceConfirmationJob = null
        monitoredSpliceTxid = null
        priceService.stopAutoRefresh()
        nodeService.stop()
    }

    fun stopNodeForBackground() {
        if (!isWaitingForPayment) {
            Log.d("AppState", "Stopping node immediately (no active payment request)")
            performBackgroundStop()
            return
        }

        Log.d("AppState", "Scheduling node stop after 60s grace period")
        backgroundStopJob?.cancel()

        // Start Foreground Service to keep CPU and network active
        try {
            LdkBackgroundService.start(context)
        } catch (e: Exception) {
            Log.e("AppState", "Failed to start LdkBackgroundService", e)
        }

        backgroundStopJob = viewModelScope.launch(Dispatchers.IO) {
            delay(60000L) // 60 seconds delay
            performBackgroundStop()
        }
    }

    fun cancelBackgroundStop() {
        if (backgroundStopJob != null) {
            backgroundStopJob?.cancel()
            backgroundStopJob = null
            Log.d("AppState", "Cancelled pending background stop")
        }
        try {
            LdkBackgroundService.stop(context)
        } catch (e: Exception) {
            Log.e("AppState", "Failed to stop LdkBackgroundService", e)
        }
    }

    private fun performBackgroundStop() {
        backgroundStopJob = null
        try {
            LdkBackgroundService.stop(context)
        } catch (e: Exception) {
            Log.e("AppState", "Failed to stop LdkBackgroundService", e)
        }
        heartbeatJob?.cancel()
        heartbeatJob = null
        stabilityJob?.cancel()
        stabilityJob = null
        pendingDepositJob?.cancel()
        pendingDepositJob = null
        nodeStartRetryJob?.cancel()
        nodeStartRetryJob = null
        spliceConfirmationJob?.cancel()
        spliceConfirmationJob = null
        monitoredSpliceTxid = null
        if (!nodeService.isRunning) return
        Log.d("AppState", "Stopping node for background")
        nodeService.stop()
    }

    fun restartNodeFromForeground() {
        isWaitingForPayment = false
        viewModelScope.launch(Dispatchers.IO) {
            if (!isInitialized) {
                isInitialized = true
                start()
                return@launch
            }
            cancelBackgroundStop()
            if (nodeService.isRunning) {
                Log.d("AppState", "Node still running (grace period), reconnecting")
                loadChannelFromDB()
                ensureLSPConnected()
                refreshBalances()
                updateStableBalances()
                resumePendingSpliceConfirmation()
                return@launch
            }
            Log.d("AppState", "Restarting node from foreground")
            if (!waitForBackgroundService()) {
                scheduleNodeStartRetry()
                return@launch
            }
            try {
                loadChannelFromDB()
                _phase.value = Phase.SYNCING
                nodeService.start(ldkNetwork(), chainUrl, null)
                nodeStartRetryJob?.cancel()
                nodeStartRetryJob = null
                _phase.value = Phase.WALLET
                refreshBalances()
                updateStableBalances()
                val sc = StabilityService.reconcileIncoming(_stableChannel.value)
                _stableChannel.value = sc
                saveChannelToDB()
                resumePendingSpliceConfirmation()
                reregisterPushTokenIfNeeded()
                startStabilityTimer()
            } catch (e: Exception) {
                Log.e("AppState", "Node restart failed", e)
                _phase.value = Phase.ERROR
                _errorMessage.value = e.message ?: "Restart failed"
            }
        }
    }

    private fun handleEvent(event: Event) {
        when (event) {
            is Event.ChannelPending -> {
                val sc = _stableChannel.value.copy()
                sc.userChannelId = event.userChannelId
                _stableChannel.value = sc
                fundingTxid = event.fundingTxo.txid
                refreshBalances()
                AuditService.log("CHANNEL_PENDING", mapOf(
                    "channel_id" to event.channelId,
                    "user_channel_id" to event.userChannelId,
                    "funding_txid" to event.fundingTxo.txid
                ))
            }
            is Event.ChannelReady -> {
                val sc = _stableChannel.value.copy()
                // In 0-conf channels, ChannelReady can fire before the splice tx confirms.
                // Treat it as metadata only; the splice stays pending until the tx has 1 conf.
                val channelIdChanged = sc.userChannelId == event.userChannelId && sc.channelId.isNotEmpty() && sc.channelId != event.channelId
                sc.channelId = event.channelId
                var pendingSpliceCandidate: String? = null
                if (sc.userChannelId == event.userChannelId) {
                    nodeService.refreshChannels()
                    val channelFundingTxid = nodeService.channels
                        .firstOrNull { it.userChannelId == event.userChannelId }
                        ?.fundingTxo?.txid
                    pendingSpliceCandidate = listOfNotNull(
                        databaseService?.getPendingSpliceTxid(),
                        spliceTxid
                    ).firstOrNull { candidate ->
                        candidate.isNotEmpty() && candidate == channelFundingTxid
                    }
                }
                val isSplice = pendingSpliceCandidate != null || channelIdChanged
                if (isSplice) {
                    isSweeping = true
                    val txid = pendingSpliceCandidate ?: spliceTxid ?: fundingTxid
                    spliceTxid = txid
                    if (txid != null && txid.isNotBlank()) {
                        startSpliceConfirmationMonitor(txid)
                    }

                    _stableChannel.value = sc
                    _statusMessage.value = "Swap pending confirmation"
                } else {
                    _stableChannel.value = sc
                }
                refreshBalances()
                saveChannelToDB()
                AuditService.log("CHANNEL_READY", mapOf("channel_id" to event.channelId))
            }
            is Event.PaymentReceived -> {
                handlePaymentReceived(
                    event.paymentId, event.amountMsat.toLong(),
                    event.paymentHash, event.customRecords
                )
            }
            is Event.PaymentSuccessful -> {
                handlePaymentSuccessful(
                    event.paymentId, event.paymentHash,
                    event.feePaidMsat?.toLong()
                )
            }
            is Event.PaymentFailed -> {
                val pid = event.paymentId
                if (pid != null) {
                    // If this is the in-flight stability send, release the marker — the send
                    // failed so there is no debit, and future sends must not stay blocked.
                    val pendingSend = try { databaseService?.loadPendingSend() } catch (_: Exception) { null }
                    if (pendingSend != null && pendingSend.paymentId == pid) {
                        databaseService?.clearPendingSend()
                        AuditService.log("STABILITY_PAYMENT_FAILED", mapOf(
                            "payment_id" to pid,
                            "error" to "payment_failed_event_cleared_pending_send"
                        ))
                    }
                }
                val curPending = _pendingTradePayments.value
                if (pid != null && curPending.containsKey(pid)) {
                    val ptp = curPending[pid]!!
                    _pendingTradePayments.value = curPending - pid
                    databaseService?.updateTradeStatus(ptp.tradeDbId, "failed")
                    val verb = if (ptp.action == "buy") "Buy" else "Sell"
                    _statusMessage.value = "$verb trade failed"
                    AuditService.log("TRADE_PAYMENT_FAILED", mapOf("payment_id" to pid))
                } else {
                    if (pid != null) {
                        databaseService?.updatePaymentStatus(pid, "failed")
                    }
                    val reason = event.reason?.toString() ?: "unknown"
                    _statusMessage.value = "Payment failed: $reason"
                    _lastPaymentResult.value = "Payment failed: $reason"
                    AuditService.log("PAYMENT_FAILED", mapOf(
                        "payment_id" to (pid ?: ""),
                        "payment_hash" to (event.paymentHash ?: ""),
                        "reason" to reason
                    ))
                }
            }
            is Event.SpliceNegotiated -> {
                handleSplicePending(event.channelId, event.userChannelId, "${event.newFundingTxo.txid}:${event.newFundingTxo.vout}")
            }
            is Event.SpliceNegotiationFailed -> {
                isSweeping = false
                spliceTxid = null
                spliceConfirmationJob?.cancel()
                spliceConfirmationJob = null
                monitoredSpliceTxid = null
                pendingSplice = null
                databaseService?.failLatestPendingSplice()
                AuditService.log("SPLICE_FAILED", mapOf("channel_id" to event.channelId))
            }
            is Event.ChannelClosed -> {
                handleChannelClosed(event.channelId, event.userChannelId, event.reason?.toString())
            }
            else -> {}
        }
    }

    private fun handlePaymentReceived(paymentId: String?, amountMsat: Long, paymentHash: String, customRecords: List<CustomTlvRecord>) {
        isWaitingForPayment = false
        // Check for sync message
        if (handleSyncMessage(customRecords, paymentHash)) {
            refreshBalances()
            updateStableBalances()
            return
        }

        val price = priceService.currentPrice.value
        val isStabilityPayment = customRecords.any { it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() && it.value.contentEquals(byteArrayOf(1)) }
        val hasStableControlMessage = customRecords.any {
            it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() &&
                !it.value.contentEquals(byteArrayOf(1))
        }
        if (hasStableControlMessage || amountMsat < 1000L) {
            AuditService.log("PAYMENT_RECEIVED_IGNORED", mapOf(
                "payment_id" to (paymentId ?: ""),
                "payment_hash" to paymentHash,
                "amount_msat" to amountMsat,
                "reason" to if (hasStableControlMessage) "unhandled_stable_control" else "sub_sat_amount"
            ))
            return
        }
        val paymentType = if (isStabilityPayment) "stability" else "lightning"
        var sc0 = _stableChannel.value
        // Always use paymentHash as fallback so dedup check runs even when paymentId is null.
        val effectiveId = paymentId ?: paymentHash
        if (isStabilityPayment && sc0.userChannelId.isEmpty()) {
            // Inline discovery from the node's channel list (mirrors StabilityService.updateBalances)
            // before giving up on the backing update.
            nodeService.refreshChannels()
            val discovered = nodeService.channels.firstOrNull()
            if (discovered != null) {
                val recovered = sc0.copy()
                recovered.userChannelId = discovered.userChannelId
                recovered.channelId = discovered.channelId
                _stableChannel.value = recovered
                sc0 = recovered
                AuditService.log("CHANNEL_ID_DISCOVERED", mapOf(
                    "user_channel_id" to discovered.userChannelId,
                    "channel_id" to discovered.channelId
                ))
            }
        }
        val userChannelId = if (isStabilityPayment) sc0.userChannelId.ifEmpty { null } else null
        if (isStabilityPayment && userChannelId == null) {
            throw Exception("Stability payment received but userChannelId is empty — cannot update backing, not acknowledging")
        }
        val backingDelta: Long? = if (isStabilityPayment) amountMsat / 1000 else null
        // Atomically insert payment row and increment backing sats in one SQLite transaction.
        // Throws on DB failure — propagates to the collector which gates ack on success.
        val record = {
            databaseService?.recordPaymentAndMaybeUpdateBacking(
                paymentId = effectiveId, paymentType = paymentType, direction = "received",
                amountMsat = amountMsat,
                amountUSD = (amountMsat.toDouble() / 1000 / Constants.SATS_IN_BTC) * price,
                btcPrice = price, counterparty = sc0.counterparty,
                userChannelId = userChannelId,
                backingDeltaSats = backingDelta
            ) ?: throw Exception("DB service unavailable")
        }
        val persistence = try {
            record()
        } catch (e: MissingChannelRowException) {
            // The channels row vanished (e.g. DB recreated) — rebuild it from in-memory state
            // via the full save, then retry once. If it still fails, rethrow to nack.
            Log.w("AppState", "Channel row missing during payment persist — recreating and retrying: ${e.message}")
            AuditService.log("CHANNEL_ROW_RECREATED", mapOf("user_channel_id" to (userChannelId ?: "")))
            saveChannelToDB()
            record()
        }
        refreshBalances()
        updateStableBalances()
        if (isStabilityPayment) {
            val backing = persistence.backingSats
                ?: throw Exception("DB did not return backing after stability payment")
            _stableChannel.value = _stableChannel.value.copy(backingSats = backing)
        }
        val sc = StabilityService.reconcileIncoming(_stableChannel.value)
        _stableChannel.value = sc
        saveChannelToDB(preserveBacking = isStabilityPayment)
        if (persistence.isNewPayment) {
            val usdVal = (amountMsat.toDouble() / 1000.0 / Constants.SATS_IN_BTC) * price
            _statusMessage.value = "Payment received: ${usdVal.usdFormatted()}"
            triggerPaymentFlash()
        }
    }

    private fun handleSyncMessage(customRecords: List<CustomTlvRecord>, paymentHash: String): Boolean {
        val tlv = customRecords.find { it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() } ?: return false
        val data = tlv.value.map { it.toByte() }.toByteArray()
        val parsed = TradeService.parseIncomingTLV(data, _stableChannel.value.counterparty) { msg, sig, pk ->
            nodeService.verifySignature(msg, sig, pk)
        } ?: return false

        val (type, expectedUsd, _) = parsed
        if (type != Constants.SYNC_MESSAGE_TYPE) return false

        val price = priceService.currentPrice.value
        val sc = StabilityService.applyTrade(_stableChannel.value, expectedUsd, price)
        _stableChannel.value = sc
        saveChannelToDB()
        AuditService.log("SYNC_V1_APPLIED", mapOf("expected_usd" to expectedUsd))
        return true
    }

    fun setStatus(message: String) {
        _statusMessage.value = message
    }

    fun addPendingTradePayment(paymentId: String, payment: PendingTradePayment) {
        _pendingTradePayments.value = _pendingTradePayments.value + (paymentId to payment)
    }

    fun triggerPaymentFlash() {
        _paymentFlash.value = true
        viewModelScope.launch {
            delay(1500)
            _paymentFlash.value = false
        }
    }

    private fun handlePaymentSuccessful(paymentId: String?, paymentHash: String, feePaidMsat: Long?) {
        val currentPending = _pendingTradePayments.value
        if (paymentId != null && currentPending.containsKey(paymentId)) {
            val ptp = currentPending[paymentId]!!
            _pendingTradePayments.value = currentPending - paymentId
            val sc = StabilityService.applyTrade(_stableChannel.value, ptp.newExpectedUSD, ptp.price)
            _stableChannel.value = sc
            saveChannelToDB()
            databaseService?.updateTradeStatus(ptp.tradeDbId, "completed")
            refreshBalances()
            updateStableBalances()
            val verb = if (ptp.action == "buy") "Buy" else "Sell"
            _statusMessage.value = "$verb confirmed"
            triggerPaymentFlash()
            AuditService.log("TRADE_COMPLETED", mapOf("payment_id" to paymentId, "action" to ptp.action))
        } else {
            if (handleStabilityPaymentSuccessful(paymentId, feePaidMsat)) return

            refreshBalances()
            updateStableBalances()
            val price = priceService.currentPrice.value
            val result = StabilityService.reconcileOutgoing(_stableChannel.value, price)
            val reconciled = result.first
            if (result.second != null) {
                reconciled.lastStabilityPayment = System.currentTimeMillis() / 1000
            }
            _stableChannel.value = reconciled
            var displayVal: String? = null
            if (paymentId != null) {
                databaseService?.updatePaymentStatus(paymentId, "completed", feePaidMsat ?: 0)
                try {
                    val db = databaseService?.readableDatabase
                    val cursor = db?.rawQuery("SELECT amount_msat, amount_usd FROM payments WHERE payment_id = ?", arrayOf(paymentId))
                    cursor?.use {
                        if (it.moveToFirst()) {
                            val amountMsat = it.getLong(0)
                            val amountUsd = if (!it.isNull(1)) it.getDouble(1) else 0.0
                            val usdVal = if (amountUsd > 0.0) amountUsd else ((amountMsat.toDouble() / 1000.0 / Constants.SATS_IN_BTC) * price)
                            displayVal = usdVal.usdFormatted()
                        }
                    }
                } catch (e: Exception) {
                    Log.w("AppState", "Failed to retrieve amount for status message: ${e.message}")
                }
            }
            saveChannelToDB(preserveBacking = true)
            val successMsg = if (displayVal != null) "Payment sent: $displayVal" else "Payment confirmed"
            _statusMessage.value = successMsg
            _lastPaymentResult.value = successMsg
        }
    }

    private fun handleStabilityPaymentSuccessful(paymentId: String?, feePaidMsat: Long?): Boolean {
        var pending = try { databaseService?.loadPendingSend() } catch (_: Exception) { null }
        if (pending != null && pending.paymentId.isEmpty() && !paymentId.isNullOrEmpty()) {
            // The previous sender died before persisting the payment ID. Adopt this event if
            // its amount matches the marker's, then reconcile through the normal replay path.
            val eventAmountMsat = try {
                nodeService.node?.payment(paymentId)?.amountMsat?.toLong()
            } catch (_: Exception) {
                null
            }
            if (eventAmountMsat != null && eventAmountMsat == pending.amountMsat) {
                try {
                    databaseService?.setPendingSendPaymentId(paymentId)
                    pending = pending.copy(paymentId = paymentId)
                    AuditService.log("STABILITY_PAYMENT_MARKER_ADOPTED", mapOf(
                        "payment_id" to paymentId,
                        "amount_msat" to pending.amountMsat
                    ))
                } catch (e: Exception) {
                    Log.w("AppState", "Could not adopt payment id for pending send marker: ${e.message}")
                }
            }
        }
        if (pending != null) {
            if (pending.paymentId.isEmpty()) {
                // Still unresolved — the reconcile path will resolve it against LDK's payment
                // store later. Avoid flushing in-memory backing through the normal
                // outgoing-payment path in the meantime.
                FCMService.flagPendingPayment(context)
                if (!paymentId.isNullOrEmpty()) {
                    databaseService?.updatePaymentStatus(paymentId, "completed", feePaidMsat ?: 0)
                }
                saveChannelToDB(preserveBacking = true)
                _statusMessage.value = "Payment confirmed; syncing stability payment"
                _lastPaymentResult.value = _statusMessage.value
                return true
            }

            val matchesPendingStabilityPayment = !paymentId.isNullOrEmpty() && pending.paymentId == paymentId
            val reconciled = reconcilePendingOutgoingStabilityPayment()
            if (matchesPendingStabilityPayment) {
                if (reconciled) {
                    databaseService?.updatePaymentStatus(paymentId!!, "completed", feePaidMsat ?: 0)
                    refreshBalances()
                    updateStableBalances()
                    _statusMessage.value = "Payment confirmed"
                    _lastPaymentResult.value = "Payment confirmed"
                } else {
                    FCMService.flagPendingPayment(context)
                    saveChannelToDB(preserveBacking = true)
                    _statusMessage.value = "Payment confirmed; syncing stability payment"
                    _lastPaymentResult.value = _statusMessage.value
                }
                return true
            }

            if (!reconciled) {
                if (!paymentId.isNullOrEmpty()) {
                    databaseService?.updatePaymentStatus(paymentId, "completed", feePaidMsat ?: 0)
                }
                saveChannelToDB(preserveBacking = true)
                _statusMessage.value = "Payment confirmed; syncing stability payment"
                _lastPaymentResult.value = _statusMessage.value
                return true
            }
        }

        val isRecordedStabilityPayment = !paymentId.isNullOrEmpty() &&
            (databaseService?.isOutgoingStabilityPayment(paymentId) == true)
        if (!isRecordedStabilityPayment) return false

        databaseService?.updatePaymentStatus(paymentId!!, "completed", feePaidMsat ?: 0)
        refreshBalances()
        updateStableBalances()
        saveChannelToDB(preserveBacking = true)
        _statusMessage.value = "Payment confirmed"
        _lastPaymentResult.value = "Payment confirmed"
        return true
    }

    private fun handleSplicePending(channelId: String, userChannelId: String, newFundingTxo: String) {
        val txid = newFundingTxo.split(":").firstOrNull() ?: newFundingTxo
        isSweeping = true
        spliceTxid = txid
        fundingTxid = txid
        val splice = pendingSplice
        if (splice != null) {
            when (splice.direction) {
                "in" -> databaseService?.setPendingSpliceTxid(txid)
                "out" -> {
                    val price = priceService.currentPrice.value
                    databaseService?.recordPayment(
                        paymentId = null, paymentType = "splice_out", direction = "sent",
                        amountMsat = splice.amountSats * 1000,
                        amountUSD = (splice.amountSats.toDouble() / Constants.SATS_IN_BTC) * price,
                        // bare txid (not the "txid:vout" outpoint) so completeSplice
                        // txid lookups match this row
                        btcPrice = price, status = "pending", txid = txid, address = splice.address
                    )
                }
            }
        } else {
            // pendingSplice is in-memory and lost across relaunch. If this event
            // is a restart replay, the latest NULL-txid splice row is this
            // splice's initiation row — stamp it so ChannelReady can complete it
            // and the no-txid expiry can't mark it failed.
            databaseService?.setPendingSpliceTxid(txid)
        }
        refreshBalances()
        updateStableBalances()
        _statusMessage.value = "Swap pending confirmation"
        startSpliceConfirmationMonitor(txid)
    }

    fun beginSpliceOut(amountSats: Long, address: String) {
        if (isSweeping) {
            throw IllegalStateException("A splice is already in progress — try again shortly")
        }
        isSweeping = true
        pendingSplice = PendingSplice("out", amountSats, address)
        _statusMessage.value = "Swap pending..."
    }

    fun cancelPendingSpliceStart() {
        if (spliceTxid == null) {
            isSweeping = false
            pendingSplice = null
            _statusMessage.value = ""
        }
    }

    private fun startSpliceConfirmationMonitor(txid: String) {
        val normalizedTxid = txid.trim()
        if (normalizedTxid.isEmpty()) return
        if (spliceConfirmationJob?.isActive == true && monitoredSpliceTxid == normalizedTxid) return

        spliceConfirmationJob?.cancel()
        monitoredSpliceTxid = normalizedTxid
        spliceConfirmationJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) {
                if (isTxConfirmed(normalizedTxid)) {
                    completeConfirmedSplice(normalizedTxid)
                    break
                }
                delay(30_000)
            }
        }
    }

    private fun resumePendingSpliceConfirmation() {
        if (databaseService?.hasPendingSplice() != true) return
        isSweeping = true
        spliceTxid = databaseService?.getPendingSpliceTxid() ?: spliceTxid ?: fundingTxid
        spliceTxid?.takeIf { it.isNotBlank() }?.let { startSpliceConfirmationMonitor(it) }
    }

    private fun isTxConfirmed(txid: String): Boolean {
        val urls = listOf(chainUrl, Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL).distinct()
        for (baseUrl in urls) {
            try {
                val normalizedTxid = txid.substringBefore(":")
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/tx/$normalizedTxid/status")
                    .build()
                httpClient.newCall(request).execute().use { response ->
                    if (!response.isSuccessful) return@use
                    val body = response.body?.string() ?: return@use
                    if (JSONObject(body).optBoolean("confirmed", false)) return true
                }
            } catch (e: Exception) {
                Log.w("AppState", "Splice confirmation check failed: ${e.message}")
            }
        }
        return false
    }

    private fun completeConfirmedSplice(txid: String) {
        val completed = databaseService?.completeSplice(txid) == true
        if (completed) {
            refreshBalances()
            updateStableBalances()

            val price = priceService.currentPrice.value
            val result = StabilityService.reconcileOutgoing(_stableChannel.value, price)
            val reconciled = result.first
            if (result.second != null) {
                reconciled.lastStabilityPayment = System.currentTimeMillis() / 1000
            }
            _stableChannel.value = reconciled
            saveChannelToDB()
        }

        isSweeping = false
        pendingSplice = null
        sweepOnchainStart = 0
        if (spliceTxid == txid) spliceTxid = null
        monitoredSpliceTxid = null
        spliceConfirmationJob = null
        _statusMessage.value = "Swap confirmed"

        AuditService.log("SPLICE_CONFIRMED", mapOf(
            "txid" to txid,
            "completed_row" to completed
        ))
    }

    private fun handleChannelClosed(channelId: String, userChannelId: String, reason: String?) {
        val sc = _stableChannel.value
        if (sc.channelId == channelId || sc.userChannelId == userChannelId || nodeService.channels.isEmpty()) {
            val balanceSats = sc.stableReceiverBTC.sats
            val price = priceService.currentPrice.value.let { if (it > 0) it else sc.latestPrice }
            val balanceUSD = if (price > 0) (balanceSats.toDouble() / Constants.SATS_IN_BTC) * price else null

            AuditService.log("CHANNEL_CLOSED", mapOf(
                "channel_id" to channelId,
                "reason" to (reason ?: "unknown"),
                "balance_sats" to balanceSats
            ))

            // Record in payment history before clearing state
            // If user initiated close, mark pending until on-chain confirms.
            // If force-closed by counterparty, mark completed immediately.
            // Use channelId as paymentId to avoid collision with splice txids.
            // Set txid to null — the close txid is not available from LDK event.
            val paymentId = channelId
            val initialStatus = if (isChannelClosing) {
                pendingClosePaymentId = paymentId
                "pending"
            } else {
                "completed"
            }
            databaseService?.recordPayment(
                paymentId = paymentId,
                paymentType = "channel_close",
                direction = "received",
                amountMsat = balanceSats * 1000,
                amountUSD = balanceUSD,
                btcPrice = if (price > 0) price else null,
                counterparty = sc.counterparty.ifEmpty { null },
                status = initialStatus,
                txid = null
            )

            // Start background resolver to find the close TX
            // Fall back to the prefs-persisted value in case in-memory fundingTxid raced to null
            val closeFundingTxid = fundingTxid
                ?: context.getSharedPreferences("balance_cache", android.content.Context.MODE_PRIVATE)
                    .getString("closing_funding_txid", null)
            if (closeFundingTxid != null && databaseService != null) {
                // Clear the pref now that we've consumed it
                context.getSharedPreferences("balance_cache", android.content.Context.MODE_PRIVATE)
                    .edit().remove("closing_funding_txid").apply()
                val resolver = CloseTxidResolver(
                    chainURLs = listOf(Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL),
                    onResolved = { _, txid ->
                        Log.d("AppState", "Close TX resolved: $txid")
                        setLastCloseTxid(txid)
                    }
                )
                viewModelScope.launch(Dispatchers.IO) {
                    resolver.resolve(
                        paymentId = paymentId,
                        fundingTxid = closeFundingTxid,
                        vout = 0,
                        databaseService = databaseService!!
                    )
                }
            }

            databaseService?.deleteChannel(sc.userChannelId)
            _stableChannel.value = StableChannel.DEFAULT
            // Clear cached channel state
            context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                .remove("cached_channel_id")
                .remove("cached_user_channel_id")
                .remove("cached_expected_usd")
                .apply()
        }

        // Keep isChannelClosing = true until lightning balance actually drains to 0
        // to avoid double-counting with on-chain. refreshBalances() clears it when ready.
        refreshBalances()
        _statusMessage.value = if (isChannelClosing) "Channel closing…" else "Channel closed"
    }

    private fun startStabilityTimer() {
        heartbeatJob?.cancel()
        FCMService.updateHeartbeat(context)
        heartbeatJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) {
                delay(5_000)
                FCMService.updateHeartbeat(context)
            }
        }

        stabilityJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) {
                delay(Constants.STABILITY_CHECK_INTERVAL_SECS * 1000)
                ensureLSPConnected()
                recordCurrentPrice()
                runStabilityCheck()
                detectOnchainDeposit()
            }
        }
    }

    private fun runStabilityCheck() {
        if (!reconcilePendingOutgoingStabilityPayment()) return

        refreshBalances()
        updateStableBalances()
        val sc = _stableChannel.value
        val price = priceService.currentPrice.value

        if (priceService.isPriceStale()) {
            AuditService.log("STABILITY_SKIP", mapOf("reason" to "stale_price", "price_age_ms" to (System.currentTimeMillis() - priceService.lastUpdate.value.time)))
            return
        }

        // Do NOT recalculate backingSats here — it's set at trade time and stays fixed.
        // As price moves, the stability check detects drift and sends payments to rebalance.

        val result = StabilityService.checkStabilityAction(sc, price)

        if (result.action == StabilityService.StabilityAction.PAY) {
            val now = System.currentTimeMillis() / 1000
            if (now - sc.lastStabilityPayment < Constants.STABILITY_PAYMENT_COOLDOWN_SECS.toLong()) return

            val amountMsat = USD(abs(result.dollarsFromPar)).toMsats(price)
            if (amountMsat == 0L) return

            // Atomically claim the send. A denied claim means another sender (e.g. the
            // background push service) already owns an in-flight send — skip this tick.
            val claimed = try {
                databaseService?.claimPendingSend(amountMsat, price) ?: false
            } catch (e: Exception) {
                AuditService.log("STABILITY_PAYMENT_FAILED", mapOf("error" to "could_not_persist_send_guard: ${e.message}"))
                return
            }
            if (!claimed) {
                AuditService.log("STABILITY_SKIP", mapOf("reason" to "pending_send_already_claimed"))
                return
            }

            val paymentId = try {
                nodeService.sendKeysend(amountMsat, sc.counterparty)
            } catch (e: Exception) {
                // Send never happened — release the claim.
                try { databaseService?.clearPendingSend() } catch (_: Exception) {}
                AuditService.log("STABILITY_PAYMENT_FAILED", mapOf("error" to (e.message ?: "")))
                return
            }

            val paymentIdString = paymentId.toString()
            val guardSaved = try {
                databaseService?.setPendingSendPaymentId(paymentIdString)
                true
            } catch (e: Exception) {
                false
            }
            FCMService.getPrefs(context).edit().putLong("bg_last_stability_sent", now).commit()
            if (!guardSaved) {
                // The payment left the device but the marker still has an empty id — the
                // reconcile path resolves it against LDK's payment store.
                FCMService.flagPendingPayment(context)
                AuditService.log(
                    "STABILITY_PAYMENT_PERSISTENCE_FAILED",
                    mapOf("error" to "payment_sent_but_id_guard_update_failed")
                )
                return
            }

            try {
                val persistence = databaseService?.recordPaymentAndMaybeUpdateBacking(
                    paymentId = paymentIdString,
                    paymentType = "stability",
                    direction = "sent",
                    amountMsat = amountMsat,
                    amountUSD = (amountMsat.toDouble() / 1000 / Constants.SATS_IN_BTC) * price,
                    btcPrice = price,
                    counterparty = sc.counterparty,
                    userChannelId = sc.userChannelId,
                    backingDeltaSats = -(amountMsat / 1000)
                ) ?: throw IllegalStateException("DB service unavailable")
                val backing = persistence.backingSats
                    ?: throw IllegalStateException("DB did not return backing after outgoing stability payment")
                val updated = sc.copy(lastStabilityPayment = now, backingSats = backing)
                _stableChannel.value = updated
                saveChannelToDB(preserveBacking = true)
                databaseService?.clearPendingSend()
                AuditService.log("STABILITY_PAYMENT_SENT", mapOf("amount_msat" to amountMsat))
            } catch (e: Exception) {
                // The send already succeeded. Keep the durable marker and block all later sends
                // until the payment row and backing delta can be committed together.
                _stableChannel.value = sc.copy(lastStabilityPayment = now)
                FCMService.flagPendingPayment(context)
                AuditService.log(
                    "STABILITY_PAYMENT_PERSISTENCE_FAILED",
                    mapOf("error" to (e.message ?: ""))
                )
            }
        }
    }

    private fun reconcilePendingOutgoingStabilityPayment(): Boolean {
        val db = databaseService ?: return false
        val pending = try { db.loadPendingSend() } catch (_: Exception) { return false } ?: return true
        var pendingPaymentId = pending.paymentId

        if (pendingPaymentId.isEmpty()) {
            // The previous sender died before persisting the payment ID. Resolve the outcome
            // against LDK's payment store instead of blocking forever.
            val node = nodeService.node ?: run {
                FCMService.flagPendingPayment(context)
                return false
            }
            val now = System.currentTimeMillis() / 1000
            val candidates = try {
                node.listPayments()
            } catch (e: Exception) {
                Log.w("AppState", "listPayments failed during reconcile: ${e.message}")
                return false
            }.filter {
                it.direction == PaymentDirection.OUTBOUND &&
                    it.kind is PaymentKind.Spontaneous &&
                    it.amountMsat?.toLong() == pending.amountMsat &&
                    it.latestUpdateTimestamp.toLong() >= pending.createdAt - 10
            }
            val succeeded = candidates.firstOrNull { it.status == PaymentStatus.SUCCEEDED }
            val stillPending = candidates.firstOrNull { it.status == PaymentStatus.PENDING }
            val failed = candidates.firstOrNull { it.status == PaymentStatus.FAILED }
            when {
                succeeded != null -> {
                    db.setPendingSendPaymentId(succeeded.id)
                    pendingPaymentId = succeeded.id
                    AuditService.log("STABILITY_PAYMENT_MARKER_ADOPTED", mapOf(
                        "payment_id" to succeeded.id,
                        "amount_msat" to pending.amountMsat
                    ))
                }
                stillPending != null -> return false  // in flight — wait
                failed != null -> {
                    db.clearPendingSend()
                    AuditService.log("STABILITY_PAYMENT_RECONCILE_CLEARED", mapOf(
                        "reason" to "send_failed",
                        "payment_id" to failed.id
                    ))
                    return true
                }
                now - pending.createdAt > 120 -> {
                    db.clearPendingSend()
                    AuditService.log("STABILITY_PAYMENT_RECONCILE_CLEARED", mapOf(
                        "reason" to "send_never_left_device",
                        "amount_msat" to pending.amountMsat
                    ))
                    return true
                }
                else -> return false  // young marker — another process may be mid-send
            }
        }

        val sc = _stableChannel.value
        if (sc.userChannelId.isEmpty()) {
            FCMService.flagPendingPayment(context)
            return false
        }

        return try {
            val persistence = db.recordPaymentAndMaybeUpdateBacking(
                paymentId = pendingPaymentId,
                paymentType = "stability",
                direction = "sent",
                amountMsat = pending.amountMsat,
                amountUSD = (pending.amountMsat.toDouble() / 1000 / Constants.SATS_IN_BTC) * pending.price,
                btcPrice = pending.price,
                counterparty = sc.counterparty,
                userChannelId = sc.userChannelId,
                backingDeltaSats = -(pending.amountMsat / 1000)
            )
            val backing = persistence.backingSats
                ?: throw IllegalStateException("DB did not return backing during outgoing reconciliation")
            _stableChannel.value = sc.copy(backingSats = backing)
            saveChannelToDB(preserveBacking = true)
            db.clearPendingSend()
            true
        } catch (e: Exception) {
            FCMService.flagPendingPayment(context)
            AuditService.log(
                "STABILITY_PAYMENT_RECONCILE_FAILED",
                mapOf("error" to (e.message ?: ""))
            )
            false
        }
    }

    internal fun detectOnchainDeposit() {
        // Use already-updated value — refreshBalances() was just called before this
        val currentSats = _onchainBalanceSats.value
        if (currentSats > prevOnchainSats && !isSweeping && pendingSplice == null) {
            val depositSats = currentSats - prevOnchainSats
            if (depositSats < 1000) {
                prevOnchainSats = currentSats
                return
            }
            val price = priceService.currentPrice.value

            // Check for pending channel close (in-memory or DB) to avoid duplicate entries
            val closeId = pendingClosePaymentId
                ?: databaseService?.getPendingChannelClosePaymentId()
            if (closeId != null) {
                databaseService?.updatePaymentStatus(closeId, "completed")
                pendingClosePaymentId = null
                isChannelClosing = false
                AuditService.log("CHANNEL_CLOSE_CONFIRMED", mapOf("sats" to depositSats))
            } else {
                // No pending close — record as new on-chain deposit
                val dedupId = "${System.currentTimeMillis() / 1000}_$depositSats"
                databaseService?.recordPayment(
                    paymentId = dedupId, paymentType = "onchain", direction = "received",
                    amountMsat = depositSats * 1000,
                    amountUSD = (depositSats.toDouble() / Constants.SATS_IN_BTC) * price,
                    btcPrice = price
                )
                AuditService.log("ONCHAIN_DEPOSIT_DETECTED", mapOf("sats" to depositSats))
            }
            // Start faster polling until deposit confirms
            startPendingDepositPolling()
        }
        prevOnchainSats = currentSats
    }

    /** Poll every 10s until spendable on-chain balance updates (deposit confirmed). */
    private fun startPendingDepositPolling() {
        pendingDepositJob?.cancel()
        pendingDepositJob = viewModelScope.launch(Dispatchers.IO) {
            // Attempt to resolve txid if we have an address but no txid yet (handles app restarts)
            val address = _onchainReceiveAddress.value
            if (address != null && _lastReceiveTxid.value == null) {
                // Run txid resolution in the background so it doesn't block the polling loop
                launch {
                    val esploraUrl = com.stablechannels.app.util.Constants.PRIMARY_CHAIN_URL
                    val txid = com.stablechannels.app.services.OnchainTxidResolver.resolve(address, esploraUrl)
                    if (txid != null) {
                        _lastReceiveTxid.value = txid
                        context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                            .putString("last_receive_txid", txid).apply()
                    }
                }
            }

            while (isActive && _spendableOnchainSats.value == 0L && _onchainBalanceSats.value > 0) {
                delay(10_000)
                refreshBalances()
            }
            
            // Deposit confirmed — aggressively clear stale txid and address from state and cache
            if (isActive && _spendableOnchainSats.value > 0L) {
                _lastReceiveTxid.value = null
                _onchainReceiveAddress.value = null
                context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                    .remove("last_receive_txid")
                    .remove("onchain_receive_address")
                    .apply()
            }
        }
    }

    fun sweepToChannel() {
        if (isSweeping) {
            _statusMessage.value = "Sweep already in progress"
            return
        }

        val channel = nodeService.channels.find { it.isChannelReady } ?: run {
            _statusMessage.value = "No ready channel"
            return
        }

        val spendable = nodeService.spendableOnchainSats()
        if (spendable <= 0) {
            _statusMessage.value = "Insufficient onchain balance"
            return
        }
        val sweepAmount = spendable

        // Set isSweeping=true BEFORE calling spliceInWithAll so that if LDK fires
        // a ChannelReady event synchronously during the call, the event handler
        // correctly identifies it as still in-flight and does not prematurely clear
        // the sweep state and re-show the Swap button.
        isSweeping = true
        pendingSplice = PendingSplice("in", sweepAmount)

        try {
            nodeService.spliceInWithAll(channel.userChannelId, channel.counterpartyNodeId)
            sweepOnchainStart = spendable
            _statusMessage.value = "Moving all onchain funds to channel..."
            val price = priceService.currentPrice.value
            val amountUSD = if (price > 0) (sweepAmount.toDouble() / Constants.SATS_IN_BTC) * price else null
            databaseService?.recordPayment(
                paymentId = null, paymentType = "splice_in", direction = "received",
                amountMsat = sweepAmount * 1000,
                amountUSD = amountUSD, btcPrice = price.takeIf { it > 0 }, status = "pending"
            )
            AuditService.log("SWEEP_TO_CHANNEL", mapOf(
                "amount_sats" to sweepAmount,
                "mode" to "splice_in_with_all"
            ))
        } catch (e: Exception) {
            isSweeping = false
            pendingSplice = null
            _statusMessage.value = "Sweep failed: ${e.message}"
            AuditService.log("SWEEP_FAILED", mapOf("error" to (e.message ?: "")))
            return
        }
    }

    /**
     * Ask the LSP whether this node_id still has channels open with it.
     * Restore guard: called before a seed-only restore wipes LDK state (which
     * would force-close a live channel at the next reestablish).
     * Returns null (unknown) on any failure — callers fail open.
     * Blocking; call from Dispatchers.IO.
     */
    fun lspChannelExists(nodeId: String): Boolean? {
        return try {
            val body = JSONObject(mapOf("node_id" to nodeId)).toString()
                .toRequestBody("application/json".toMediaType())
            val request = Request.Builder()
                .url(Constants.LSP_CHANNEL_EXISTS_URL)
                .post(body)
                .build()
            httpClient.newCall(request).execute().use { response ->
                if (!response.isSuccessful) return null
                val json = JSONObject(response.body?.string() ?: return null)
                if (!json.has("exists")) return null
                json.getBoolean("exists")
            }
        } catch (_: Exception) {
            null
        }
    }

    private fun fetchFeeRate(): Long? {
        val urls = listOf(Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL)
        for (baseUrl in urls) {
            try {
                val request = Request.Builder().url("$baseUrl/fee-estimates").build()
                val response = httpClient.newCall(request).execute()
                val body = response.body?.string() ?: continue
                val json = JSONObject(body)
                val rate = json.optDouble("6", -1.0)
                if (rate > 0) return rate.roundToLong()
            } catch (_: Exception) { /* try next */ }
        }
        return null
    }

    /** Network from Constants.DEFAULT_NETWORK — "regtest" only via TestOverrides (E2E). */
    private fun ldkNetwork(): Network = Constants.LDK_NETWORK

    /** Test Blockstream connectivity; fall back to mempool.space if unreachable. */
    private suspend fun resolveChainUrl(): String {
        return withContext(Dispatchers.IO) {
            try {
                val request = Request.Builder()
                    .url("${Constants.PRIMARY_CHAIN_URL}/blocks/tip/height")
                    .build()
                val response = httpClient.newCall(request).execute()
                if (response.isSuccessful) {
                    Constants.PRIMARY_CHAIN_URL
                } else {
                    AuditService.log("CHAIN_SOURCE_FALLBACK", mapOf(
                        "primary" to Constants.PRIMARY_CHAIN_URL,
                        "using" to Constants.FALLBACK_CHAIN_URL
                    ))
                    Constants.FALLBACK_CHAIN_URL
                }
            } catch (_: Exception) {
                AuditService.log("CHAIN_SOURCE_FALLBACK", mapOf(
                    "primary" to Constants.PRIMARY_CHAIN_URL,
                    "using" to Constants.FALLBACK_CHAIN_URL
                ))
                Constants.FALLBACK_CHAIN_URL
            }
        }
    }

    fun ensureLSPConnected() {
        val node = nodeService.node ?: return
        nodeService.refreshChannels()
        val allUsable = nodeService.channels.isNotEmpty() && nodeService.channels.all { it.isUsable }
        if (allUsable) return
        try {
            node.connect(Constants.DEFAULT_LSP_PUBKEY, Constants.DEFAULT_LSP_ADDRESS, true)
        } catch (e: Exception) {
            AuditService.log("LSP_CONNECT_FAILED", mapOf("error" to (e.message ?: "")))
        }
    }

    fun setOnchainReceiveAddress(address: String?) {
        if (address == null) return
        _onchainReceiveAddress.value = address
        context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
            .putString("onchain_receive_address", address).apply()
        
        // Start polling for this address to be hit
        viewModelScope.launch {
            val esploraUrl = com.stablechannels.app.util.Constants.PRIMARY_CHAIN_URL
            val txid = com.stablechannels.app.services.OnchainTxidResolver.resolve(address, esploraUrl)
            if (txid != null) {
                _lastReceiveTxid.value = txid
                context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                    .putString("last_receive_txid", txid).apply()
            }
        }
    }

    fun refreshBalances() {
        nodeService.refreshChannels()
        val balances = nodeService.balances() ?: return
        val lightning = balances.totalLightningBalanceSats.toLong()
        val onchain = balances.totalOnchainBalanceSats.toLong()
        val hasReady = nodeService.channels.any { it.isChannelReady }

        // Sync fundingTxid directly from the LDK node's channel details
        // to gracefully handle out-of-band splices (e.g. LSP-initiated)
        val channel = nodeService.channels.firstOrNull()
        if (channel != null) {
            val txo = channel.fundingTxo
            if (txo != null) {
                val currentTxid = txo.txid
                if (currentTxid != null && currentTxid != fundingTxid) {
                    fundingTxid = currentTxid
                }
            }
        }
        _lightningBalanceSats.value = lightning
        _onchainBalanceSats.value = onchain
        _hasReadyChannel.value = hasReady
        val spendable = balances.spendableOnchainBalanceSats.toLong()
        _spendableOnchainSats.value = spendable


        // Clear closing flag once lightning balance fully resolves, or if a new channel is opened
        // Don't clear pendingClosePaymentId here — let detectOnchainDeposit()
        // handle it when the on-chain funds arrive
        if (isChannelClosing && lightning == 0L) {
            isChannelClosing = false
        }

        _totalBalanceSats.value = when {
            isChannelClosing -> onchain
            isSweeping -> lightning
            // No open channel but both balances present: lightning is pending-close claimable
            // that overlaps with on-chain — avoid double-count
            !hasReady && lightning > 0 && onchain > 0 -> onchain
            else -> lightning + onchain
        }

        // Calculate native sats (lightning minus stable portion) for slider position
        // On-chain funds excluded — they're not in the channel yet
        val sc = _stableChannel.value
        val btcPrice = priceService.currentPrice.value
        val stableSats = if (btcPrice > 0) (sc.expectedUSD.amount / btcPrice * Constants.SATS_IN_BTC).toLong() else 0L
        val native = (lightning - stableSats).coerceAtLeast(0L)
        _nativeSats.value = native

        // Cache for instant display on next launch
        context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
            .putLong("cached_lightning_sats", lightning)
            .putLong("cached_onchain_sats", onchain)
            .putLong("cached_spendable_sats", spendable)
            .putLong("cached_native_sats", native)
            .apply()
    }

    fun updateStableBalances() {
        val price = priceService.currentPrice.value
        val sc = StabilityService.updateBalances(
            _stableChannel.value, nodeService.channels,
            _onchainBalanceSats.value, price
        )
        _stableChannel.value = sc
    }

    private fun currentChannelFundingTxidMatches(txid: String): Boolean {
        nodeService.refreshChannels()
        return nodeService.channels.any { channel ->
            channel.isChannelReady && channel.fundingTxo?.txid == txid
        }
    }

    fun saveChannelToDB(preserveBacking: Boolean = false) {
        val sc = _stableChannel.value
        if (sc.userChannelId.isEmpty()) return
        if (preserveBacking) {
            databaseService?.saveChannelPreservingBacking(
                sc.channelId, sc.userChannelId, sc.expectedUSD.amount, sc.note,
                receiverSats = sc.stableReceiverBTC.sats,
                latestPrice = sc.latestPrice
            )
        } else {
            databaseService?.saveChannel(
                sc.channelId, sc.userChannelId, sc.expectedUSD.amount, sc.backingSats, sc.note,
                receiverSats = sc.stableReceiverBTC.sats,
                latestPrice = sc.latestPrice
            )
        }
        // Cache in SharedPreferences so UI has correct state on next launch
        context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
            .putString("cached_channel_id", sc.channelId)
            .putString("cached_user_channel_id", sc.userChannelId)
            .putFloat("cached_expected_usd", sc.expectedUSD.amount.toFloat())
            .apply()
    }

    /** Called when the UI returns to the foreground. Reloads channel state from the DB so
     *  backing increments committed by StabilityProcessingService while this process was
     *  cached are picked up before any save can clobber them. Cheap and safe to call repeatedly. */
    fun onForegroundResume() {
        loadChannelFromDB()
    }

    private fun loadChannelFromDB() {
        val sc = _stableChannel.value
        if (sc.userChannelId.isEmpty()) return
        val record = databaseService?.loadChannel(sc.userChannelId) ?: return
        val updated = sc.copy(
            channelId = record.channelId,
            userChannelId = record.userChannelId,
            expectedUSD = USD(record.expectedUSD),
            backingSats = record.backingSats,
            note = record.note
        )
        if (record.receiverSats > 0) {
            updated.stableReceiverBTC = Bitcoin(record.receiverSats)
            updated.stableReceiverUSD = if (record.latestPrice > 0) {
                USD.fromBitcoin(Bitcoin(record.receiverSats), record.latestPrice)
            } else USD.ZERO
            StabilityService.recomputeNative(updated)
        }
        if (record.latestPrice > 0) {
            updated.latestPrice = record.latestPrice
            priceService.seedPrice(record.latestPrice)
        }
        _stableChannel.value = updated
    }

    fun recordCurrentPrice() {
        val price = priceService.currentPrice.value
        if (price > 0) {
            databaseService?.recordPrice(price, "median")
        }
    }

    private suspend fun backfillHourlyPrices() {
        val db = databaseService ?: return
        val thirtyDaysAgo = System.currentTimeMillis() / 1000 - 30 * 24 * 3600
        val oldest = db.getOldestPriceHistoryTimestamp()
        if (oldest != null && oldest < thirtyDaysAgo) return
        val since = oldest ?: thirtyDaysAgo
        val candles = priceService.fetchKrakenOHLC(since)
        if (candles.isEmpty()) return
        val count = db.backfillHourlyPrices(candles)
        if (count > 0) {
            AuditService.log("CHART_BACKFILL", mapOf("points" to count))
        }
    }

    private fun backgroundServiceOwnsLdk(): Boolean =
        StabilityProcessingService.isRunning ||
            LdkNodeOwner.isOwnedBy(LdkNodeOwner.STABILITY_SERVICE)

    private fun waitForBackgroundService(): Boolean {
        if (!backgroundServiceOwnsLdk()) return true
        Log.d("AppState", "Waiting for background stability service to finish...")
        val deadline = System.currentTimeMillis() + 30_000
        while (backgroundServiceOwnsLdk() && System.currentTimeMillis() < deadline) {
            Thread.sleep(500)
        }
        if (backgroundServiceOwnsLdk()) {
            val owner = LdkNodeOwner.currentOwner() ?: "background service"
            Log.w("AppState", "Background service still owns LDK after 30s (owner=$owner); skipping node start")
            _statusMessage.value = "Finishing background sync..."
            FCMService.flagPendingPayment(context)
            return false
        }
        return true
    }

    private fun scheduleNodeStartRetry() {
        if (nodeStartRetryJob?.isActive == true) return
        nodeStartRetryJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive && backgroundServiceOwnsLdk()) {
                delay(1_000)
            }
            if (!isActive || nodeService.isRunning) return@launch
            Log.d("AppState", "Retrying node start after LDK owner released")
            _statusMessage.value = "Syncing wallet..."
            restartNodeFromForeground()
        }
    }

    private fun reregisterPushTokenIfNeeded() {
        val nodeId = nodeService.nodeId
        if (nodeId.isEmpty()) return

        FCMService.saveNodeId(context, nodeId)

        try {
            FirebaseMessaging.getInstance().token.addOnSuccessListener { token ->
                FCMService.saveToken(context, token)
                viewModelScope.launch(Dispatchers.IO) {
                    FCMService.registerTokenWithLSP(token, nodeId)
                }
            }
        } catch (_: Exception) {
            // Firebase not configured — push notifications disabled
        }
    }

    private fun processPendingPushPayment() {
        if (!FCMService.hasPendingPayment(context)) return
        Log.d("AppState", "Processing pending push payment")
        FCMService.clearPendingPayment(context)
        try {
            nodeService.node?.connect(Constants.DEFAULT_LSP_PUBKEY, Constants.DEFAULT_LSP_ADDRESS, true)
        } catch (e: Exception) {
            Log.w("AppState", "LSP connect failed in processPendingPushPayment: ${e.message}")
            AuditService.log("LSP_CONNECT_FAILED", mapOf("error" to (e.message ?: "")))
        }
        refreshBalances()
        updateStableBalances()
        runStabilityCheck()
    }
}
