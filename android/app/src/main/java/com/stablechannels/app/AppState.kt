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
import com.stablechannels.app.services.websocket.MempoolWebSocketClient
import com.stablechannels.app.services.websocket.MempoolWebSocketService
import com.stablechannels.app.services.websocket.WebSocketEvent
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.LspPreferencesManager
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit
import okhttp3.Request
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import org.lightningdevkit.ldknode.*
import java.io.File
import kotlin.math.abs
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.roundToLong

enum class Phase {
    LOADING, ONBOARDING, SYNCING, WALLET, ERROR
}

private class RetryableSyncException(message: String) : Exception(message)

class AppState(private val context: Context) : ViewModel() {

    companion object {
        /**
         * Set to true right before launching an in-app activity that backgrounds the app
         * (e.g. the log share sheet). [MainActivity] honors this only for a short grace window
         * (see `SHARE_SUPPRESS_WINDOW_MS`): if the app resumes within that window the node
         * stop/restart is skipped so returning doesn't visibly refresh the UI, but if the user
         * continues into another app past the window, [MainActivity] falls back to the normal
         * background stop so the node doesn't stay active indefinitely and the eventual
         * foreground resync still happens. Always cleared by [MainActivity] on the next
         * pause/resume.
         */
        @Volatile
        var suppressNextBackgroundCycle = false

        // Covers ordinary quick app-switches without keeping an unserviced cached Android
        // process in control of the node for longer than the common return window.
        private const val QUICK_SWITCH_GRACE_MS = 10_000L
    }

    val nodeService = NodeService(context)
    val priceService = PriceService(context)
    var databaseService: DatabaseService? = null
        private set
    var tradeService: TradeService? = null
        private set
    private val mempoolWebSocketService: MempoolWebSocketClient = MempoolWebSocketService()

    private val _phase = MutableStateFlow(Phase.LOADING)
    val phase: StateFlow<Phase> = _phase

    private val _isSyncing = MutableStateFlow(false)
    val isSyncing: StateFlow<Boolean> = _isSyncing

    private var isInitialized = false
    private var backgroundStopJob: Job? = null

    @Volatile
    var isWaitingForPayment = false

    // Set while an in-app system picker (e.g. photo picker) is open, so the transient onPause
    // it triggers doesn't tear down and resync the LDK node.
    @Volatile
    var isPickingMedia = false

    private val _errorMessage = MutableStateFlow("")
    val errorMessage: StateFlow<String> = _errorMessage

    private val _stableChannel = MutableStateFlow(StableChannel.defaultWithLsp(context))
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
    private var lastReceiveTxidAddress: String? = null

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

    private fun setLastReceiveTxid(txid: String?, address: String?) {
        _lastReceiveTxid.value = txid
        lastReceiveTxidAddress = address

        val editor = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
        if (txid.isNullOrBlank()) {
            editor.remove("last_receive_txid")
            editor.remove("last_receive_txid_address")
        } else {
            editor.putString("last_receive_txid", txid)
            if (!address.isNullOrBlank()) {
                editor.putString("last_receive_txid_address", address)
            } else {
                editor.remove("last_receive_txid_address")
            }
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
        lastReceiveTxidAddress = prefs.getString("last_receive_txid_address", null)

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
            _stableChannel.value = StableChannel.defaultWithLsp(context).copy(
                channelId = cachedChannelId ?: "",
                userChannelId = cachedUserChannelId,
                expectedUSD = USD(cachedExpectedUsd.toDouble())
            )
        }

        configureMempoolWebSocket()
    }

    private fun configureMempoolWebSocket() {
        mempoolWebSocketService.onBlockHeader = {
            viewModelScope.launch(Dispatchers.IO) {
                refreshBalances()
                pollPaymentConfirmations(force = true)
            }
        }
        mempoolWebSocketService.onTransactionDetected = { event ->
            viewModelScope.launch(Dispatchers.IO) {
                handleWebSocketTransactionDetected(event)
            }
        }
    }

    private fun connectMempoolWebSocket() {
        mempoolWebSocketService.connect()
        _onchainReceiveAddress.value?.takeIf { it.isNotBlank() }?.let {
            mempoolWebSocketService.trackAddress(it)
        }
    }

    private val _pendingTradePayments = MutableStateFlow<Map<String, PendingTradePayment>>(emptyMap())
    val pendingTradePayments: StateFlow<Map<String, PendingTradePayment>> = _pendingTradePayments

    /** Terminal result of a correlated trade, keyed by its fee payment id. Only a signed
     *  acceptance or rejection lands here — the trade sheets read this instead of inferring
     *  success from absence in the pending map (a rejection also clears pending). */
    data class TradeOutcome(val accepted: Boolean, val message: String)
    private val _tradeOutcomes = MutableStateFlow<Map<String, TradeOutcome>>(emptyMap())
    val tradeOutcomes: StateFlow<Map<String, TradeOutcome>> = _tradeOutcomes

    /** Rehydrate a trade's terminal outcome from SQLite. Background services commit
     *  accepted/rejected results directly to the database without touching the in-memory
     *  map, so the sheets poll this while pending and it runs for every known payment id
     *  on startup/foreground. */
    fun refreshTradeOutcome(paymentId: String) {
        if (_tradeOutcomes.value.containsKey(paymentId)) return
        viewModelScope.launch(Dispatchers.IO) {
            val terminal = try {
                databaseService?.terminalTradeOutcome(paymentId)
            } catch (_: Exception) { null } ?: return@launch
            val (accepted, reason) = terminal
            // update {} — this runs on an IO thread while the handler path writes from
            // the event loop, and a read-modify-write on .value could drop an entry.
            _tradeOutcomes.update { outcomes ->
                outcomes + (paymentId to if (accepted) {
                    TradeOutcome(true, "")
                } else {
                    TradeOutcome(false, TradeProtocol.rejectionMessage(reason ?: "internal_failure"))
                })
            }
            _pendingTradePayments.update { it - paymentId }
        }
    }

    private fun refreshAllTradeOutcomes(paymentIds: Collection<String>) {
        paymentIds.forEach { refreshTradeOutcome(it) }
    }
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
    private var trackedClosingFundingTxid: String? = null
    var spliceTxid: String? = null
    var fundingTxid: String? = null
        set(value) {
            field = value
            context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                .putString("funding_txid", value).apply()
        }

    private val _paymentFlash = MutableStateFlow(false)
    val paymentFlash: StateFlow<Boolean> = _paymentFlash

    private val _confirmationUpdateEpoch = MutableStateFlow(0)
    val confirmationUpdateEpoch: StateFlow<Int> = _confirmationUpdateEpoch


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
    private var nodeStartRetryAttempts: Int = 0
    private var spliceConfirmationJob: Job? = null
    private var monitoredSpliceTxid: String? = null
    @Volatile
    private var isConfirmationPolling = false
    @Volatile
    private var lastConfirmationPollAtMs = 0L
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
                val db = DatabaseService(context)
                databaseService = db
                launch { databaseService?.seedHistoricalPrices() }
                launch { backfillHourlyPrices() }
                tradeService = TradeService(nodeService, db)
                db.markExpiredTradesUncertain()
                _pendingTradePayments.value = db.unresolvedTradePayments()
                refreshAllTradeOutcomes(_tradeOutcomes.value.keys + _pendingTradePayments.value.keys)

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
                    nodeService.start(Network.BITCOIN, chainUrl, null)
                    resetNodeStartRetryState()
                    nodeStartRetryJob?.cancel()
                    nodeStartRetryJob = null
                    _phase.value = Phase.WALLET
                    _isSyncing.value = false
                    // Restore the known funding txid before the first live balance refresh so
                    // an ordinary cold start is not mistaken for a funding transition.
                    fundingTxid = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
                        .getString("funding_txid", null)
                    refreshBalances()
                    pollPaymentConfirmations(force = true)
                    connectMempoolWebSocket()
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
                                    trackedClosingFundingTxid = closeFundingTxid
                                    mempoolWebSocketService.trackTx(closeFundingTxid)
                                    val resolver = CloseTxidResolver(
                                        chainURLs = listOf(Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL),
                                        onResolved = { _, txid ->
                                            Log.d("AppState", "Close TX resolved on restart: $txid")
                                            setLastCloseTxid(txid)
                                            mempoolWebSocketService.untrackTx(closeFundingTxid)
                                            trackedClosingFundingTxid = null
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
                    nodeService.start(Network.BITCOIN, chainUrl, null)
                    resetNodeStartRetryState()
                    _phase.value = Phase.WALLET
                    refreshBalances()
                    pollPaymentConfirmations(force = true)
                    connectMempoolWebSocket()
                    reregisterPushTokenIfNeeded()
                    startStabilityTimer()
                    viewModelScope.launch(Dispatchers.IO) {
                        delay(3000)
                        ensureLSPConnected()
                    }
                }
            } catch (e: Exception) {
                handleNodeStartFailure(e, "Unknown error")
            }
        }
    }

    fun createWallet(mnemonic: String?) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                _phase.value = Phase.SYNCING
                nodeService.start(Network.BITCOIN, chainUrl, mnemonic)
                resetNodeStartRetryState()
                _phase.value = Phase.WALLET
                refreshBalances()
                pollPaymentConfirmations(force = true)
                connectMempoolWebSocket()
                reregisterPushTokenIfNeeded()
                startStabilityTimer()
            } catch (e: Exception) {
                handleNodeStartFailure(e, "Failed to create wallet")
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
        mempoolWebSocketService.disconnect()
        nodeService.stop()
    }

    fun stopNodeForBackground() {
        if (!isWaitingForPayment && !isPickingMedia) {
            // Defer the stop so a quick app-switch reconnects instantly instead of forcing a
            // full LDK restart + chain resync on every return. If the user stays away past the
            // window, the deferred stop below runs and the node is torn down as normal.
            Log.d("AppState", "Scheduling node stop after quick-switch grace period")
            launchBackgroundStop(delayMs = QUICK_SWITCH_GRACE_MS)
            return
        }

        // A payment wait or an open in-app picker both route through the existing bounded 60s
        // grace path rather than skipping the stop outright — so a stuck-true isPickingMedia
        // (e.g. launch() threw, or the composition was disposed) degrades to "stop after 60s"
        // instead of "never stop the node again".
        Log.d("AppState", "Scheduling node stop after 60s grace period")
        backgroundStopJob?.cancel()

        // Start Foreground Service to keep CPU and network active
        try {
            LdkBackgroundService.start(context)
        } catch (e: Exception) {
            Log.e("AppState", "Failed to start LdkBackgroundService", e)
        }

        launchBackgroundStop(delayMs = 60000L)
    }

    private fun launchBackgroundStop(delayMs: Long = 0L) {
        backgroundStopJob?.cancel()
        val job = viewModelScope.launch(Dispatchers.IO) {
            try {
                if (delayMs > 0L) {
                    delay(delayMs)
                }
                performBackgroundStop()
            } finally {
                if (backgroundStopJob === coroutineContext[Job]) {
                    backgroundStopJob = null
                }
            }
        }
        backgroundStopJob = job
    }

    fun cancelBackgroundStop() {
        if (backgroundStopJob != null) {
            backgroundStopJob?.cancel()
            Log.d("AppState", "Cancelled pending background stop")
        }
        try {
            LdkBackgroundService.stop(context)
        } catch (e: Exception) {
            Log.e("AppState", "Failed to stop LdkBackgroundService", e)
        }
    }

    /**
     * Cancels any pending background-stop job and *waits* for it to actually finish — including
     * an in-flight, non-cancellable performBackgroundStop() blocked on the native node.stop()
     * call — before returning. Callers can then trust nodeService.isRunning immediately after.
     * Plain cancel() alone doesn't suffice: it can't interrupt the blocking native call, so a
     * caller checking isRunning right after cancel() can race the stop finishing moments later.
     */
    private suspend fun cancelBackgroundStopAndAwait() {
        val job = backgroundStopJob
        backgroundStopJob = null
        job?.cancelAndJoin()
        Log.d("AppState", "Cancelled pending background stop")
        try {
            LdkBackgroundService.stop(context)
        } catch (e: Exception) {
            Log.e("AppState", "Failed to stop LdkBackgroundService", e)
        }
    }

    private fun performBackgroundStop() {
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
        mempoolWebSocketService.disconnect()
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
            cancelBackgroundStopAndAwait()
            if (nodeService.isRunning) {
                Log.d("AppState", "Node still running (grace period), reconnecting")
                loadChannelFromDB()
                ensureLSPConnected()
                refreshBalances()
                pollPaymentConfirmations(force = true)
                connectMempoolWebSocket()
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
                nodeService.start(Network.BITCOIN, chainUrl, null)
                resetNodeStartRetryState()
                nodeStartRetryJob?.cancel()
                nodeStartRetryJob = null
                _phase.value = Phase.WALLET
                refreshBalances()
                pollPaymentConfirmations(force = true)
                connectMempoolWebSocket()
                updateStableBalances()
                val sc = StabilityService.reconcileIncoming(_stableChannel.value)
                _stableChannel.value = sc
                saveChannelToDB()
                resumePendingSpliceConfirmation()
                reregisterPushTokenIfNeeded()
                startStabilityTimer()
            } catch (e: Exception) {
                Log.e("AppState", "Node restart failed", e)
                handleNodeStartFailure(e, "Restart failed")
            }
        }
    }

    private fun handleNodeStartFailure(e: Exception, fallbackMessage: String) {
        if (e is NodeService.AlreadyRunningException && nodeService.isRunning) {
            Log.w("AppState", "Ignoring duplicate node start after another start succeeded", e)
            _phase.value = Phase.WALLET
            _isSyncing.value = false
            _errorMessage.value = ""
            AuditService.log(
                "NODE_START_DUPLICATE",
                mapOf("error" to (e.message ?: fallbackMessage))
            )
            return
        }

        if (isRetryableNodeStartFailure(e)) {
            _phase.value = Phase.SYNCING
            _errorMessage.value = ""
            _statusMessage.value = "Network unstable. Retrying wallet sync..."
            scheduleNodeStartRetry()
            return
        }

        _errorMessage.value = e.message ?: fallbackMessage
        _phase.value = Phase.ERROR
    }

    // Matched by exception type (not message text) since LDK's Display strings aren't a stable
    // contract across ldk-node versions — only these variants indicate a transient chain-source
    // issue that a retry can plausibly fix.
    private fun isRetryableNodeStartFailure(e: Exception): Boolean {
        if (e is NodeException) {
            return e is NodeException.FeerateEstimationUpdateFailed ||
                e is NodeException.FeerateEstimationUpdateTimeout ||
                e is NodeException.TxSyncFailed ||
                e is NodeException.TxSyncTimeout ||
                e is NodeException.GossipUpdateFailed ||
                e is NodeException.GossipUpdateTimeout ||
                e is NodeException.WalletOperationFailed ||
                e is NodeException.WalletOperationTimeout ||
                e is NodeException.LiquiditySourceUnavailable ||
                e is NodeException.ConnectionFailed
        }
        val msg = e.message?.lowercase() ?: return false
        return msg.contains("fee rate estimates") ||
            msg.contains("timed out") ||
            msg.contains("network is unreachable") ||
            msg.contains("dns") ||
            msg.contains("connection refused")
    }

    /**
     * Validates and saves a custom LSP pubkey/address, then performs an in-process soft
     * restart of the LDK node so the new config takes effect immediately.
     *
     * A full node rebuild (not just a reconnect) is required because the LSP pubkey/address
     * is baked into LDK's `Config`, `AnchorChannelsConfig`, and LSPS2 liquidity source at
     * build time. This is only ever attempted with no open channels (re-checked here even
     * though the UI already gates it), which makes an in-process [NodeService.stop] +
     * [NodeService.start] the safest option: it reuses the already-hardened node lifecycle
     * (including [LdkNodeOwner] release/reacquire) without tearing down the Activity,
     * ViewModel, or other background jobs the way a full app-process restart via Intent would.
     *
     * @param onComplete called with `null` on success, or a human-readable error message.
     */
    fun switchLsp(pubkey: String, address: String, onComplete: (String?) -> Unit) {
        viewModelScope.launch(Dispatchers.IO) {
            lspChangeBlockedReason()?.let { onComplete(it); return@launch }
            // Check this *before* touching prefs — if the background stability service currently
            // owns the LDK node, nodeService.start() would throw for a reason unrelated to the new
            // LSP being invalid, and we don't want to misattribute that as a bad config and roll back.
            if (!waitForBackgroundService()) {
                onComplete("Background sync is in progress — try again in a moment.")
                return@launch
            }
            // Capture the currently-working config so a bad new LSP can be rolled back to it —
            // LDK validates the pubkey/address at node-build time, which happens *after* we've
            // already persisted the new values, so a failure here must not leave the node
            // permanently stopped on unusable config.
            val hadCustomLsp = LspPreferencesManager.hasCustomLsp(context)
            val previousPubkey = LspPreferencesManager.getLspPubkey(context)
            val previousAddress = LspPreferencesManager.getLspAddress(context)

            val validationError = LspPreferencesManager.saveCustomLsp(context, pubkey, address)
            if (validationError != null) {
                onComplete(validationError)
                return@launch
            }
            try {
                performLspNodeRestart()
                onComplete(null)
            } catch (e: Exception) {
                Log.e("AppState", "Failed to restart node after LSP switch — rolling back", e)
                AuditService.log("LSP_SWITCH_FAILED", mapOf("error" to (e.message ?: "")))
                if (hadCustomLsp) {
                    LspPreferencesManager.saveCustomLsp(context, previousPubkey, previousAddress)
                } else {
                    LspPreferencesManager.resetToDefault(context)
                }
                try {
                    performLspNodeRestart()
                } catch (rollbackError: Exception) {
                    Log.e("AppState", "Rollback restart also failed", rollbackError)
                    if (!nodeService.isRunning) scheduleNodeStartRetry()
                }
                onComplete("Invalid LSP — reverted to previous settings. (${e.message})")
            }
        }
    }

    /** Clears any custom LSP override and restarts the node against the default (stablechannels.com). */
    fun resetLspToDefault(onComplete: (String?) -> Unit) {
        viewModelScope.launch(Dispatchers.IO) {
            lspChangeBlockedReason()?.let { onComplete(it); return@launch }
            if (!waitForBackgroundService()) {
                onComplete("Background sync is in progress — try again in a moment.")
                return@launch
            }
            // Capture the previous custom config so a restart failure doesn't leave the user
            // stuck on default with their working custom LSP already erased (same rationale as
            // the rollback in switchLsp()).
            val hadCustomLsp = LspPreferencesManager.hasCustomLsp(context)
            val previousPubkey = LspPreferencesManager.getLspPubkey(context)
            val previousAddress = LspPreferencesManager.getLspAddress(context)

            LspPreferencesManager.resetToDefault(context)
            try {
                performLspNodeRestart()
                onComplete(null)
            } catch (e: Exception) {
                Log.e("AppState", "Failed to restart node after LSP reset — rolling back", e)
                AuditService.log("LSP_SWITCH_FAILED", mapOf("error" to (e.message ?: "")))
                if (hadCustomLsp) {
                    LspPreferencesManager.saveCustomLsp(context, previousPubkey, previousAddress)
                    try {
                        performLspNodeRestart()
                    } catch (rollbackError: Exception) {
                        Log.e("AppState", "Rollback restart also failed", rollbackError)
                    }
                }
                if (!nodeService.isRunning) scheduleNodeStartRetry()
                onComplete("Failed to reset LSP — reverted to previous settings. (${e.message})")
            }
        }
    }

    /** Gate for changing the LSP. A stopped/mid-restart node reports an empty channel list, so
     *  require the node running (making listChannels authoritative) and cross-check persisted
     *  channel state. Returns a user-facing reason to block, or null if the change is allowed. */
    private fun lspChangeBlockedReason(): String? {
        if (!nodeService.isRunning) return "Start the wallet before changing the LSP."
        nodeService.refreshChannels()
        val hasChannel = nodeService.channels.isNotEmpty() || (databaseService?.hasAnyChannel() ?: false)
        if (hasChannel) return "Close all channels before switching LSPs."
        return null
    }

    /** Stops and rebuilds the LDK node in-place so it picks up the current LSP prefs.
     *  Callers must confirm there are no open channels before invoking this. */
    private suspend fun performLspNodeRestart() {
        // cancelAndJoin (not cancel) — coroutine cancellation is cooperative, so if the periodic
        // stabilityJob tick is already inside a native LDK call (e.g. ensureLSPConnected ->
        // node.connect) when we ask it to cancel, it keeps running until it hits a suspension
        // point. Joining guarantees it has actually stopped before we touch nodeService.node below.
        stabilityJob?.cancelAndJoin()
        heartbeatJob?.cancelAndJoin()
        if (nodeService.isRunning) {
            nodeService.stop()
        }
        nodeService.start(Network.BITCOIN, chainUrl, null, strictLspConnect = true)
        // Refresh the in-memory counterparty from the new LSP pubkey. Only safe to overwrite when
        // there's no channel yet (which switchLsp/resetLspToDefault already require); an open
        // channel's counterparty is derived from the live channel in refreshBalances() instead.
        val sc = _stableChannel.value
        if (sc.channelId.isEmpty() && sc.userChannelId.isEmpty()) {
            _stableChannel.value = sc.copy(counterparty = LspPreferencesManager.getLspPubkey(context))
        }
        refreshBalances()
        ensureLSPConnected()
        reregisterPushTokenIfNeeded()
        startStabilityTimer()
        AuditService.log("LSP_SWITCHED", mapOf(
            "pubkey" to LspPreferencesManager.getLspPubkey(context),
            "address" to LspPreferencesManager.getLspAddress(context)
        ))
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
                    _statusMessage.value = "Move pending confirmation"
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
                var failedTrade = pid?.let(curPending::get)
                if (pid != null && failedTrade == null) {
                    val amountMsat = try {
                        nodeService.node?.payment(pid)?.amountMsat?.toLong()
                    } catch (_: Exception) {
                        null
                    }
                    if (amountMsat != null) {
                        failedTrade = try {
                            databaseService?.failUnattachedPreparedTrade(pid, amountMsat)
                        } catch (_: Exception) {
                            null
                        }
                    }
                }
                if (pid != null && failedTrade != null) {
                    val ptp = failedTrade
                    _pendingTradePayments.value = curPending - pid
                    databaseService?.markTradeSendFailed(ptp.tradeDbId)
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
                val paymentRowId = pendingSplice?.paymentRowId
                isSweeping = false
                spliceTxid = null
                spliceConfirmationJob?.cancel()
                spliceConfirmationJob = null
                monitoredSpliceTxid = null
                pendingSplice = null
                databaseService?.failPendingSplice(paymentRowId)
                AuditService.log("SPLICE_FAILED", mapOf("channel_id" to event.channelId))
            }
            is Event.ChannelClosed -> {
                handleChannelClosed(event.channelId, event.userChannelId, event.counterpartyNodeId, event.reason)
            }
            else -> {}
        }
    }

    private fun handlePaymentReceived(paymentId: String?, amountMsat: Long, paymentHash: String, customRecords: List<CustomTlvRecord>) {
        isWaitingForPayment = false
        // Check for sync message
        if (handleSyncMessage(customRecords, paymentHash, amountMsat)) {
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

    private fun handleSyncMessage(
        customRecords: List<CustomTlvRecord>,
        paymentHash: String,
        amountMsat: Long
    ): Boolean {
        val tlv = customRecords.find { it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() } ?: return false
        val data = tlv.value.map { it.toByte() }.toByteArray()
        if (data.contentEquals(byteArrayOf(1))) return false
        val message = TradeProtocol.parseSignedControl(data, _stableChannel.value.counterparty) { msg, sig, pk ->
            nodeService.verifySignature(msg, sig, pk)
        } ?: run {
            AuditService.log("TRADE_RESULT_INVALID", mapOf("payment_hash" to paymentHash))
            return true
        }
        val db = databaseService ?: throw RetryableSyncException("Trade database unavailable")
        val result = when (message) {
            is TradeControlMessage.Rejected -> {
                if (amountMsat != TradeProtocol.RESULT_CONTROL_AMOUNT_MSAT) {
                    AuditService.log("TRADE_REJECTED_V1_CONTEXT_INVALID", mapOf("amount_msat" to amountMsat))
                    return true
                }
                db.applyTradeRejection(message)
            }
            is TradeControlMessage.Sync -> {
                if (amountMsat != TradeProtocol.RESULT_CONTROL_AMOUNT_MSAT) {
                    AuditService.log("SYNC_V1_CONTROL_AMOUNT_INVALID", mapOf("amount_msat" to amountMsat))
                    return true
                }
                if (message.correlation != null) {
                    db.applyCorrelatedTradeAcceptance(message)
                } else {
                    val price = priceService.currentAccountingPrice()
                    if (price <= 0.0) {
                        AuditService.log("SYNC_V1_DEFERRED", mapOf("reason" to "untrusted_price"))
                        throw RetryableSyncException("Cannot apply SYNC_V1 without a trusted BTC price")
                    }
                    db.applyUncorrelatedSyncIfNewer(message, price)
                }
            }
        }
        when (result.status) {
            TradeControlApplyStatus.RETRY -> {
                try { db.markTradeResponseNotCommittable(message) } catch (_: Exception) {}
                AuditService.log("TRADE_RESULT_DEFERRED", mapOf("payment_hash" to paymentHash))
                throw RetryableSyncException("Signed trade result could not be committed")
            }
            TradeControlApplyStatus.INVALID -> {
                AuditService.log("TRADE_RESULT_INVALID", mapOf("payment_hash" to paymentHash))
                return true
            }
            TradeControlApplyStatus.DUPLICATE -> {
                result.paymentId?.let { paymentId ->
                    _pendingTradePayments.value = _pendingTradePayments.value - paymentId
                    _tradeOutcomes.value = _tradeOutcomes.value + (
                        paymentId to if (message is TradeControlMessage.Rejected) {
                            TradeOutcome(false, TradeProtocol.rejectionMessage(message.reasonCode))
                        } else {
                            TradeOutcome(true, "")
                        }
                    )
                }
                val channel = db.loadChannel(_stableChannel.value.userChannelId)
                    ?: throw RetryableSyncException("Duplicate result channel could not be reloaded")
                val updated = _stableChannel.value.copy(
                    channelId = channel.channelId,
                    expectedUSD = USD(channel.expectedUSD),
                    backingSats = channel.backingSats,
                    latestPrice = channel.latestPrice
                )
                StabilityService.recomputeNative(updated)
                _stableChannel.value = updated
                return true
            }
            TradeControlApplyStatus.APPLIED -> {
                result.paymentId?.let { paymentId ->
                    _pendingTradePayments.value = _pendingTradePayments.value - paymentId
                    _tradeOutcomes.value = _tradeOutcomes.value + (
                        paymentId to if (message is TradeControlMessage.Rejected) {
                            TradeOutcome(false, TradeProtocol.rejectionMessage(message.reasonCode))
                        } else {
                            TradeOutcome(true, "")
                        }
                    )
                }
                val channel = db.loadChannel(_stableChannel.value.userChannelId)
                    ?: throw RetryableSyncException("Applied result channel could not be reloaded")
                val updated = _stableChannel.value.copy(
                    channelId = channel.channelId,
                    expectedUSD = USD(channel.expectedUSD),
                    backingSats = channel.backingSats,
                    latestPrice = channel.latestPrice
                )
                StabilityService.recomputeNative(updated)
                _stableChannel.value = updated
                val divergence = result.localBackingSats != null &&
                    result.peerBackingSats != null &&
                    result.localBackingSats != result.peerBackingSats
                AuditService.log("TRADE_RESULT_APPLIED", mapOf(
                    "payment_hash" to paymentHash,
                    "local_backing_sats" to (result.localBackingSats ?: -1L),
                    "peer_backing_sats" to (result.peerBackingSats ?: -1L),
                    "allocation_diverged" to divergence,
                    "allocation_applied" to result.allocationApplied
                ))
                if (message is TradeControlMessage.Rejected) {
                    _statusMessage.value = TradeProtocol.rejectionMessage(message.reasonCode)
                } else if (result.paymentId != null) {
                    val verb = if (result.action == "buy") "Buy" else "Sell"
                    _statusMessage.value = "$verb confirmed"
                    triggerPaymentFlash()
                }
                return true
            }
        }
    }

    fun setStatus(message: String) {
        _statusMessage.value = message
    }

    fun addPendingTradePayment(paymentId: String, payment: PendingTradePayment): Boolean {
        _pendingTradePayments.value = _pendingTradePayments.value + (paymentId to payment)
        val unresolved = try {
            databaseService?.tradeIsUnresolved(payment.tradeDbId) == true
        } catch (_: Exception) {
            true
        }
        if (!unresolved) {
            _pendingTradePayments.value = _pendingTradePayments.value - paymentId
        }
        return unresolved
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
        if (paymentId != null) {
            val db = databaseService
            var pending = currentPending[paymentId]
            var recognizedTrade = pending != null
            if (db != null) {
                val marked = try {
                    if (pending != null) {
                        db.markKnownTradeFeePaid(pending.tradeDbId, paymentId)
                    } else {
                        db.markTradeFeePaid(paymentId)
                    }
                } catch (_: Exception) {
                    false
                }
                recognizedTrade = recognizedTrade || marked

                var eventAmountMsat: Long? = null
                if (!recognizedTrade) {
                    eventAmountMsat = try {
                        nodeService.node?.payment(paymentId)?.amountMsat?.toLong()
                    } catch (_: Exception) {
                        null
                    }
                    if (eventAmountMsat != null) {
                        pending = try {
                            db.adoptUnattachedPreparedTrade(paymentId, eventAmountMsat)
                        } catch (_: Exception) {
                            null
                        }
                        recognizedTrade = pending != null
                    }
                }
                if (!recognizedTrade) {
                    recognizedTrade = try { db.tradePaymentExists(paymentId) } catch (_: Exception) { false }
                }
                if (!recognizedTrade && eventAmountMsat == null &&
                    try { db.hasUnattachedPreparedTrade() } catch (_: Exception) { false }
                ) {
                    _statusMessage.value = "Payment confirmed; awaiting signed trade result"
                    AuditService.log("TRADE_FEE_ID_UNRESOLVED", mapOf("payment_id" to paymentId))
                    return
                }
            }
            if (recognizedTrade) {
                if (pending != null) {
                    _pendingTradePayments.value = currentPending +
                        (paymentId to pending.copy(status = "fee_paid"))
                }
                val verb = if (pending?.action == "buy") "Buy" else "Sell"
                _statusMessage.value = "$verb fee paid; awaiting signed result"
                AuditService.log("TRADE_FEE_PAID", mapOf(
                    "payment_id" to paymentId,
                    "action" to (pending?.action ?: "unknown"),
                    "fee_paid_msat" to (feePaidMsat ?: 0L)
                ))
                return
            }
        }

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
        val feeSuffix = feePaidMsat?.let { " (fee: ${(it / 1000).satsFormatted()} sats)" } ?: ""
        val successMsg = if (displayVal != null) "Payment sent: $displayVal$feeSuffix" else "Payment sent$feeSuffix"
        _statusMessage.value = successMsg
        _lastPaymentResult.value = successMsg
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
        // Prefer the exact in-memory row. After a process restart the LDK event can be replayed;
        // the database then accepts only one recent pending candidate and never a failed row.
        val assignedRowId = databaseService?.assignPendingSpliceTxid(
            txid = txid,
            paymentRowId = pendingSplice?.paymentRowId
        )
        if (assignedRowId == null) {
            AuditService.log("SPLICE_TXID_UNMATCHED", mapOf(
                "channel_id" to channelId,
                "user_channel_id" to userChannelId,
                "txid" to txid
            ))
        }
        refreshBalances()
        updateStableBalances()
        _statusMessage.value = "Move pending confirmation"
        startSpliceConfirmationMonitor(txid)
    }

    fun beginSpliceOut(amountSats: Long, address: String, accountingPrice: Double) {
        if (isSweeping) {
            throw IllegalStateException("A splice is already in progress — try again shortly")
        }
        val db = databaseService
            ?: throw IllegalStateException("Payment history is unavailable — splice not started")
        // Persist before the native call so the operation survives a process restart.
        val paymentRowId = db.recordPayment(
            paymentId = null, paymentType = "splice_out", direction = "sent",
            amountMsat = amountSats * 1000,
            amountUSD = if (accountingPrice > 0) {
                (amountSats.toDouble() / Constants.SATS_IN_BTC) * accountingPrice
            } else null,
            btcPrice = accountingPrice.takeIf { it > 0 },
            status = "pending",
            address = address
        )
        if (paymentRowId <= 0) {
            throw IllegalStateException("Could not save pending splice — splice not started")
        }
        isSweeping = true
        pendingSplice = PendingSplice("out", amountSats, address, paymentRowId)
        _statusMessage.value = "Move pending..."
    }

    fun cancelPendingSpliceStart() {
        if (spliceTxid == null) {
            val paymentRowId = pendingSplice?.paymentRowId
            isSweeping = false
            pendingSplice = null
            databaseService?.failPendingSplice(paymentRowId)
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
        spliceTxid = databaseService?.getPendingSpliceTxid() ?: spliceTxid
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
        _statusMessage.value = "Move confirmed"

        AuditService.log("SPLICE_CONFIRMED", mapOf(
            "txid" to txid,
            "completed_row" to completed
        ))
    }

    private fun closureReasonData(reason: ClosureReason?): JSONObject {
        val obj = JSONObject()
        if (reason == null) {
            obj.put("kind", "UNKNOWN")
            return obj
        }
        obj.put("kind", reason::class.simpleName ?: "UNKNOWN")
        when (reason) {
            is ClosureReason.CounterpartyForceClosed -> obj.put("peer_msg", reason.peerMsg)
            is ClosureReason.HolderForceClosed -> {
                obj.put("message", reason.message)
                reason.broadcastedLatestTxn?.let { obj.put("broadcasted_latest_txn", it) }
            }
            is ClosureReason.ProcessingError -> obj.put("err", reason.err)
            else -> {}
        }
        return obj
    }

    private fun handleChannelClosed(
        channelId: String,
        userChannelId: String,
        counterpartyNodeId: String?,
        reason: ClosureReason?
    ) {
        val sc = _stableChannel.value
        if (sc.channelId == channelId || sc.userChannelId == userChannelId || nodeService.channels.isEmpty()) {
            val balanceSats = sc.stableReceiverBTC.sats
            val price = priceService.currentPrice.value.let { if (it > 0) it else sc.latestPrice }
            val balanceUSD = if (price > 0) (balanceSats.toDouble() / Constants.SATS_IN_BTC) * price else null

            AuditService.log("CHANNEL_CLOSED", mapOf(
                "channel_id" to channelId,
                "counterparty_node_id" to counterpartyNodeId,
                "reason" to closureReasonData(reason),
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
                trackedClosingFundingTxid = closeFundingTxid
                mempoolWebSocketService.trackTx(closeFundingTxid)
                // Clear the pref now that we've consumed it
                context.getSharedPreferences("balance_cache", android.content.Context.MODE_PRIVATE)
                    .edit().remove("closing_funding_txid").apply()
                val resolver = CloseTxidResolver(
                    chainURLs = listOf(Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL),
                    onResolved = { _, txid ->
                        Log.d("AppState", "Close TX resolved: $txid")
                        setLastCloseTxid(txid)
                        mempoolWebSocketService.untrackTx(closeFundingTxid)
                        trackedClosingFundingTxid = null
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
            _stableChannel.value = StableChannel.defaultWithLsp(context)
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
                refreshTradeUncertainty()
                runStabilityCheck()
                detectOnchainDeposit()
                pollPaymentConfirmations()
            }
        }
    }

    private fun refreshTradeUncertainty() {
        val db = databaseService ?: return
        val changed = try { db.markExpiredTradesUncertain() } catch (_: Exception) { 0 }
        if (changed > 0) {
            _pendingTradePayments.value = try {
                db.unresolvedTradePayments()
            } catch (_: Exception) {
                _pendingTradePayments.value
            }
            _statusMessage.value = "Trade result delayed; it will still be accepted when received"
            AuditService.log("TRADE_RESULT_UNCERTAIN", mapOf(
                "reason" to "no_response",
                "count" to changed
            ))
        }
    }

    fun triggerConfirmationRefresh() {
        viewModelScope.launch(Dispatchers.IO) {
            pollPaymentConfirmations(force = true)
        }
    }

    private fun requiredConfirmationsForType(paymentType: String): Int {
        return when (paymentType) {
            "splice_in", "splice_out" -> 1
            else -> 6
        }
    }

    private data class TxConfirmationStatus(
        val confirmed: Boolean,
        val blockHeight: Int?
    )

    private fun fetchChainTipHeight(): Int? {
        val urls = listOf(chainUrl, Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL).distinct()
        for (baseUrl in urls) {
            try {
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/blocks/tip/height")
                    .build()
                httpClient.newCall(request).execute().use { response ->
                    if (!response.isSuccessful) return@use
                    val body = response.body?.string()?.trim() ?: return@use
                    body.toIntOrNull()?.let { return it }
                }
            } catch (_: Exception) {
            }
        }
        return null
    }

    private fun fetchTxConfirmationStatus(txid: String): TxConfirmationStatus? {
        val normalizedTxid = txid.substringBefore(":").trim()
        if (normalizedTxid.isEmpty()) return null

        val urls = listOf(chainUrl, Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL).distinct()
        for (baseUrl in urls) {
            try {
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/tx/$normalizedTxid/status")
                    .build()
                httpClient.newCall(request).execute().use { response ->
                    if (!response.isSuccessful) return@use
                    val body = response.body?.string() ?: return@use
                    val json = JSONObject(body)
                    val confirmed = json.optBoolean("confirmed", false)
                    val blockHeight = if (json.has("block_height") && !json.isNull("block_height")) {
                        json.optInt("block_height", 0).takeIf { it > 0 }
                    } else {
                        null
                    }
                    return TxConfirmationStatus(confirmed = confirmed, blockHeight = blockHeight)
                }
            } catch (_: Exception) {
            }
        }
        return null
    }

    private fun fetchTxPaysToAddress(txid: String, address: String): Boolean? {
        val normalizedTxid = txid.substringBefore(":").trim()
        if (normalizedTxid.isEmpty() || address.isBlank()) return null

        val urls = listOf(chainUrl, Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL).distinct()
        for (baseUrl in urls) {
            try {
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/tx/$normalizedTxid")
                    .build()
                httpClient.newCall(request).execute().use { response ->
                    if (!response.isSuccessful) return@use
                    val body = response.body?.string() ?: return@use
                    val txJson = JSONObject(body)
                    val vouts = txJson.optJSONArray("vout") ?: return@use
                    for (i in 0 until vouts.length()) {
                        val vout = vouts.optJSONObject(i) ?: continue
                        if (vout.optString("scriptpubkey_address", "") == address) {
                            return true
                        }
                    }
                    return false
                }
            } catch (_: Exception) {
            }
        }
        return null
    }

    /** Backfills txid for pending "received onchain" rows that never resolved one (e.g. a
     *  channel-close payout, which lands on an address we weren't watching). Without this,
     *  such rows are invisible to getPaymentsNeedingConfirmation() and stay at 0/6 forever. */
    private fun resolveMissingReceiveTxids() {
        val db = databaseService ?: return
        val unresolved = db.getPaymentsNeedingTxidResolution()
        if (unresolved.isEmpty()) return

        val onchainPayments = try {
            nodeService.node?.listPayments()
                ?.filter { it.direction == PaymentDirection.INBOUND && it.kind is PaymentKind.Onchain }
        } catch (e: Exception) {
            Log.w("AppState", "listPayments lookup failed during txid backfill: ${e.message}")
            null
        } ?: return

        unresolved.forEach { row ->
            val paymentId = row.paymentId ?: return@forEach
            val match = onchainPayments
                .filter { it.amountMsat?.toLong() == row.amountMsat }
                .maxByOrNull { it.latestUpdateTimestamp } ?: return@forEach
            val txid = (match.kind as PaymentKind.Onchain).txid
            db.updatePaymentTxid(paymentId, txid)
            AuditService.log("ONCHAIN_RECEIVE_TXID_BACKFILLED", mapOf(
                "payment_id" to paymentId,
                "txid" to txid
            ))
        }
    }

    private suspend fun pollPaymentConfirmations(force: Boolean = false) {
        val now = System.currentTimeMillis()
        if (!force && (now - lastConfirmationPollAtMs) < 15_000) {
            return
        }
        if (isConfirmationPolling) {
            return
        }

        val db = databaseService ?: return
        isConfirmationPolling = true
        try {
            resolveMissingReceiveTxids()
            val tipHeight = fetchChainTipHeight() ?: return
            val pending = db.getPaymentsNeedingConfirmation(limit = 100)
            var anyUpdated = false

            pending.forEach { payment ->
                val txid = payment.txid ?: return@forEach

                if (payment.paymentType == "onchain" && payment.direction == "received") {
                    val expectedAddress = payment.address?.trim().orEmpty()
                    if (expectedAddress.isNotEmpty()) {
                        when (fetchTxPaysToAddress(txid, expectedAddress)) {
                            false -> {
                                val cleared = db.clearPaymentTxidForRow(payment.id)
                                anyUpdated = anyUpdated || cleared
                                if (_lastReceiveTxid.value == txid) {
                                    setLastReceiveTxid(null, null)
                                }
                                AuditService.log("ONCHAIN_TXID_ADDRESS_MISMATCH", mapOf(
                                    "payment_id" to payment.id,
                                    "txid" to txid,
                                    "address" to expectedAddress
                                ))
                                return@forEach
                            }
                            null -> return@forEach
                            true -> {
                            }
                        }
                    }
                }

                val txStatus = fetchTxConfirmationStatus(txid) ?: return@forEach
                val required = requiredConfirmationsForType(payment.paymentType)

                val (newConfirmations, newStatus) = if (!txStatus.confirmed) {
                    0 to "pending"
                } else {
                    val blockHeight = txStatus.blockHeight
                    val confs = if (blockHeight != null) {
                        (tipHeight - blockHeight + 1).coerceAtLeast(0).coerceAtMost(required)
                    } else {
                        payment.confirmations.coerceAtLeast(1).coerceAtMost(required)
                    }
                    confs to if (confs >= required) "completed" else "pending"
                }

                if (payment.confirmations != newConfirmations || payment.status != newStatus) {
                    val updated = db.updatePaymentConfirmationState(
                        paymentRowId = payment.id,
                        confirmations = newConfirmations,
                        status = newStatus
                    )
                    anyUpdated = anyUpdated || updated
                }
            }

            if (anyUpdated) {
                _confirmationUpdateEpoch.value = _confirmationUpdateEpoch.value + 1
            }
            lastConfirmationPollAtMs = now
        } finally {
            isConfirmationPolling = false
        }
    }

    private fun runStabilityCheck() {
        if (!reconcilePendingOutgoingStabilityPayment()) return

        refreshBalances()
        updateStableBalances()
        val sc = _stableChannel.value
        val price = priceService.currentAccountingPrice()

        if (price <= 0.0) {
            AuditService.log("STABILITY_SKIP", mapOf("reason" to "untrusted_price", "price_age_ms" to (System.currentTimeMillis() - priceService.lastUpdate.value.time)))
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

            // Chain-freshness gate (see #243): never pay on a stale chain tip, and check
            // BEFORE claiming so a deferral leaves no claimed-but-unsent marker. The next
            // stability tick retries once LDK's background sync catches up.
            val syncAge = nodeService.lightningSyncAgeSecs()
            if (syncAge == null || syncAge > Constants.STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS) {
                AuditService.log(
                    "STABILITY_SKIP",
                    mapOf("reason" to "stale_lightning_sync", "sync_age_secs" to syncAge)
                )
                return
            }

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
                // Tag with the STABLE_CHANNEL_TLV [0x01] marker so the LSP classifies
                // this as a settlement (operator GUI) and runs reconcile_incoming_stability
                // immediately, matching every other sender. See issue #161.
                nodeService.sendStabilityPayment(
                    amountMsat,
                    sc.counterparty,
                    listOf(CustomTlvRecord(Constants.STABLE_CHANNEL_TLV_TYPE.toULong(), byteArrayOf(1)))
                )
            } catch (e: NodeService.StaleLightningSyncException) {
                // The wrapper's send-boundary gate fired (sync went stale after the precheck
                // above). Send never happened — release the claim and retry next tick.
                try { databaseService?.clearPendingSend() } catch (_: Exception) {}
                AuditService.log(
                    "STABILITY_SKIP",
                    mapOf("reason" to "stale_lightning_sync", "sync_age_secs" to e.syncAgeSecs)
                )
                return
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

    private fun clearOnchainDepositStatusIfNeeded() {
        if (_statusMessage.value.startsWith("Onchain deposit detected", ignoreCase = true)) {
            _statusMessage.value = ""
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
        val db = databaseService
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
                ?: db?.getPendingChannelClosePaymentId()
            if (closeId != null) {
                val knownCloseTxid = db?.getPaymentTxid(closeId) ?: _lastCloseTxid.value
                if (!knownCloseTxid.isNullOrBlank()) {
                    db?.updatePaymentTxid(closeId, knownCloseTxid)
                }
                db?.updatePaymentStatus(closeId, "completed")
                pendingClosePaymentId = null
                trackedClosingFundingTxid?.let { mempoolWebSocketService.untrackTx(it) }
                trackedClosingFundingTxid = null
                isChannelClosing = false
                AuditService.log("CHANNEL_CLOSE_CONFIRMED", mapOf("sats" to depositSats))
            } else {
                val receiveAddress = _onchainReceiveAddress.value
                val addressMatchedTxid = _lastReceiveTxid.value?.takeIf {
                    !it.isNullOrBlank() &&
                        !receiveAddress.isNullOrBlank() &&
                        lastReceiveTxidAddress == receiveAddress
                }
                // Address matching only finds deposits to our own tracked receive address —
                // it misses proceeds landing elsewhere (e.g. a channel-close output), leaving
                // txid permanently null and confirmations stuck at 0/6. LDK's own payment
                // list already knows the real txid for every inbound on-chain payment
                // regardless of address, so fall back to it (mirrors src/user.rs's sweep).
                val resolvedTxid = addressMatchedTxid ?: try {
                    nodeService.node?.listPayments()
                        ?.filter {
                            it.direction == PaymentDirection.INBOUND &&
                                it.kind is PaymentKind.Onchain &&
                                it.amountMsat?.toLong() == depositSats * 1000
                        }
                        ?.maxByOrNull { it.latestUpdateTimestamp }
                        ?.let { (it.kind as PaymentKind.Onchain).txid }
                } catch (e: Exception) {
                    Log.w("AppState", "listPayments lookup failed during deposit detection: ${e.message}")
                    null
                }

                // Always record the deposit, mirroring iOS. When the websocket and this
                // balance-delta path both see the same deposit, the pair is reconciled at
                // txid time instead of skipped up front: recordWebSocketReceive adopts a
                // txid-less placeholder, and reconcileResolvedReceiveTxid deletes it when
                // the websocket row already exists. A skip heuristic here silently omits a
                // second deposit arriving while any earlier receive is still confirming.
                val dedupId = if (!resolvedTxid.isNullOrBlank()) {
                    "onchain_receive_$resolvedTxid"
                } else {
                    "onchain_deposit_${java.util.UUID.randomUUID()}"
                }
                val rowId = db?.recordPayment(
                    paymentId = dedupId,
                    paymentType = "onchain",
                    direction = "received",
                    amountMsat = depositSats * 1000,
                    amountUSD = (depositSats.toDouble() / Constants.SATS_IN_BTC) * price,
                    btcPrice = price,
                    status = "pending",
                    txid = resolvedTxid,
                    address = receiveAddress
                )

                if (rowId != null && rowId != -1L) {
                    triggerPaymentFlash()
                    AuditService.log("ONCHAIN_DEPOSIT_DETECTED", mapOf(
                        "sats" to depositSats,
                        "status" to "pending",
                        "txid_known" to (!resolvedTxid.isNullOrBlank())
                    ))
                }
            }
            // Home card now carries pending receive state; remove stale capsule text.
            clearOnchainDepositStatusIfNeeded()
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
            val shouldResolveTxid = address != null &&
                (_lastReceiveTxid.value == null || lastReceiveTxidAddress != address)
            if (shouldResolveTxid) {
                // Run txid resolution in the background so it doesn't block the polling loop
                launch {
                    val esploraUrl = com.stablechannels.app.util.Constants.PRIMARY_CHAIN_URL
                    val txid = com.stablechannels.app.services.OnchainTxidResolver.resolve(address, esploraUrl)
                    if (txid != null) {
                        setLastReceiveTxid(txid, address)
                        databaseService?.reconcileResolvedReceiveTxid(txid, address)
                    }
                }
            }

            while (isActive && _spendableOnchainSats.value == 0L && _onchainBalanceSats.value > 0) {
                delay(10_000)
                refreshBalances()
            }
            
            // Deposit confirmed — aggressively clear stale txid and address from state and cache
            if (isActive && _spendableOnchainSats.value > 0L) {
                setLastReceiveTxid(null, null)
                setOnchainReceiveAddress(null)
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

        val db = databaseService ?: run {
            _statusMessage.value = "Payment history is unavailable — move not started"
            return
        }
        val price = priceService.currentAccountingPrice()
        val amountUSD = if (price > 0) {
            (sweepAmount.toDouble() / Constants.SATS_IN_BTC) * price
        } else null
        // Persist before the native call so SpliceNegotiated always has a row to update,
        // even if the event is delivered before spliceInWithAll returns.
        val paymentRowId = db.recordPayment(
            paymentId = null, paymentType = "splice_in", direction = "received",
            amountMsat = sweepAmount * 1000,
            amountUSD = amountUSD, btcPrice = price.takeIf { it > 0 }, status = "pending"
        )
        if (paymentRowId <= 0) {
            _statusMessage.value = "Could not save pending move — move not started"
            return
        }
        isSweeping = true
        pendingSplice = PendingSplice("in", sweepAmount, paymentRowId = paymentRowId)

        try {
            nodeService.spliceInWithAll(channel.userChannelId, channel.counterpartyNodeId)
            sweepOnchainStart = spendable
            _statusMessage.value = "Moving all onchain funds to channel..."
            AuditService.log("SWEEP_TO_CHANNEL", mapOf(
                "amount_sats" to sweepAmount,
                "mode" to "splice_in_with_all"
            ))
        } catch (e: Exception) {
            isSweeping = false
            pendingSplice = null
            db.failPendingSplice(paymentRowId)
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

    /** Blocking fee-rate lookup for pre-send UI estimates. Call from Dispatchers.IO. */
    fun currentFeeRateSatVb(): Long? = fetchFeeRate()

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
            node.connect(LspPreferencesManager.getLspPubkey(context), LspPreferencesManager.getLspAddress(context), true)
        } catch (e: Exception) {
            AuditService.log("LSP_CONNECT_FAILED", mapOf("error" to (e.message ?: "")))
        }
    }

    fun setOnchainReceiveAddress(address: String?) {
        val oldAddress = _onchainReceiveAddress.value
        _onchainReceiveAddress.value = address
        val editor = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
        if (address == null) {
            editor.remove("onchain_receive_address")
        } else {
            editor.putString("onchain_receive_address", address)
        }
        editor.apply()

        if (!oldAddress.isNullOrBlank() && oldAddress != address) {
            mempoolWebSocketService.untrackAddress(oldAddress)
        }

        if (!address.isNullOrBlank() && oldAddress != address) {
            // New receive request: drop stale txid from previous address/session.
            setLastReceiveTxid(null, null)
        }

        if (address == null) {
            return
        }

        mempoolWebSocketService.trackAddress(address)
        
        // Start polling for this address to be hit
        viewModelScope.launch {
            val esploraUrl = com.stablechannels.app.util.Constants.PRIMARY_CHAIN_URL
            val txid = com.stablechannels.app.services.OnchainTxidResolver.resolve(address, esploraUrl)
            if (txid != null) {
                setLastReceiveTxid(txid, address)
                databaseService?.reconcileResolvedReceiveTxid(txid, address)
            }
        }
    }

    fun prepareChannelCloseTracking(userChannelId: String) {
        setLastCloseTxid(null)
        val liveTxid = nodeService.channels
            .firstOrNull { it.userChannelId == userChannelId || it.isChannelReady }
            ?.fundingTxo?.txid

        if (!liveTxid.isNullOrBlank()) {
            fundingTxid = liveTxid
            trackedClosingFundingTxid = liveTxid
            mempoolWebSocketService.trackTx(liveTxid)
            context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
                .edit().putString("closing_funding_txid", liveTxid).apply()
        }
    }

    private fun handleWebSocketTransactionDetected(event: WebSocketEvent) {
        val db = databaseService ?: return

        when (event) {
            is WebSocketEvent.Receive -> {
                if (isChannelClosing || isSweeping || pendingSplice != null) {
                    return
                }
                if (event.amountSats < 1000) {
                    return
                }

                val price = priceService.currentPrice.value
                val amountUsd = if (price > 0) {
                    (event.amountSats.toDouble() / Constants.SATS_IN_BTC) * price
                } else {
                    null
                }

                val paymentId = "onchain_receive_${event.txid}"
                val rowId = db.recordWebSocketReceive(
                    paymentId = paymentId,
                    amountMsat = event.amountSats * 1000,
                    amountUSD = amountUsd,
                    btcPrice = price.takeIf { it > 0 },
                    txid = event.txid,
                    address = event.target
                )

                if (rowId != -1L) {
                    setLastReceiveTxid(event.txid, event.target)
                    clearOnchainDepositStatusIfNeeded()
                    triggerPaymentFlash()
                    AuditService.log(
                        "WEBSOCKET_INSTANT_PAYMENT_RECORDED",
                        mapOf("txid" to event.txid, "sats" to event.amountSats)
                    )
                }
            }

            is WebSocketEvent.Removed -> {
                try {
                    db.failPaymentByTxid(event.txid)
                    AuditService.log(
                        "WEBSOCKET_RBF_FAILED_PAYMENT",
                        mapOf("target" to event.target, "txid" to event.txid)
                    )
                } catch (e: Exception) {
                    AuditService.log(
                        "WEBSOCKET_RBF_FAIL_FAILED",
                        mapOf("txid" to event.txid, "error" to (e.message ?: ""))
                    )
                }
            }

            is WebSocketEvent.TrackedOutspend -> {
                if (!isChannelClosing) {
                    return
                }

                val expectedFundingTxid = trackedClosingFundingTxid
                    ?: fundingTxid
                    ?: context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
                        .getString("closing_funding_txid", null)

                if (!expectedFundingTxid.isNullOrBlank() && expectedFundingTxid != event.trackedTxid) {
                    return
                }

                val closeId = pendingClosePaymentId ?: db.getPendingChannelClosePaymentId()
                if (!closeId.isNullOrBlank()) {
                    db.updatePaymentTxid(closeId, event.spendingTxid)
                    setLastCloseTxid(event.spendingTxid)
                }

                mempoolWebSocketService.untrackTx(event.trackedTxid)
                trackedClosingFundingTxid = null
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
                if (currentTxid != fundingTxid) {
                    fundingTxid = currentTxid
                }
            }
            // Derive the authoritative counterparty from the live channel. For an open channel
            // this is the ground truth — it defends against sc.counterparty drifting from the
            // node the channel is actually with (the channels table doesn't persist the
            // counterparty pubkey, so a relaunch would otherwise fall back to the LSP-pref
            // default and could target the wrong node for trades/keysends).
            val liveCounterparty = channel.counterpartyNodeId
            if (liveCounterparty.isNotEmpty() && _stableChannel.value.counterparty != liveCounterparty) {
                _stableChannel.value = _stableChannel.value.copy(counterparty = liveCounterparty)
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
        val delayMs = min((2.0.pow(nodeStartRetryAttempts.toDouble()) * 1000.0).toLong(), 60_000L)
        nodeStartRetryAttempts = min(nodeStartRetryAttempts + 1, 6)
        nodeStartRetryJob = viewModelScope.launch(Dispatchers.IO) {
            delay(delayMs)
            while (isActive && backgroundServiceOwnsLdk()) {
                delay(1_000)
            }
            if (!isActive || nodeService.isRunning) return@launch
            // Re-check primary/fallback health so a retry doesn't keep hammering the same
            // degraded esplora endpoint that just failed the fee-rate/chain-sync fetch.
            chainUrl = resolveChainUrl()
            Log.d("AppState", "Retrying node start after LDK owner released (chainUrl=$chainUrl)")
            _statusMessage.value = "Syncing wallet..."
            restartNodeFromForeground()
        }
    }

    private fun resetNodeStartRetryState() {
        nodeStartRetryAttempts = 0
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
            nodeService.node?.connect(LspPreferencesManager.getLspPubkey(context), LspPreferencesManager.getLspAddress(context), true)
        } catch (e: Exception) {
            Log.w("AppState", "LSP connect failed in processPendingPushPayment: ${e.message}")
            AuditService.log("LSP_CONNECT_FAILED", mapOf("error" to (e.message ?: "")))
        }
        refreshBalances()
        updateStableBalances()
        runStabilityCheck()
    }
}
