package com.stablechannels.app

import android.content.Context
import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.google.firebase.messaging.FirebaseMessaging
import com.stablechannels.app.models.*
import com.stablechannels.app.push.FCMService
import com.stablechannels.app.push.StabilityProcessingService
import com.stablechannels.app.services.*
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import okhttp3.OkHttpClient
import okhttp3.Request
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

    private val _errorMessage = MutableStateFlow("")
    val errorMessage: StateFlow<String> = _errorMessage

    private val _stableChannel = MutableStateFlow(StableChannel.DEFAULT)
    val stableChannel: StateFlow<StableChannel> = _stableChannel

    private val _statusMessage = MutableStateFlow("")
    val statusMessage: StateFlow<String> = _statusMessage

    private val _lightningBalanceSats: MutableStateFlow<Long>
    val lightningBalanceSats: StateFlow<Long> get() = _lightningBalanceSats

    private val _onchainBalanceSats: MutableStateFlow<Long>
    val onchainBalanceSats: StateFlow<Long> get() = _onchainBalanceSats

    private val _totalBalanceSats: MutableStateFlow<Long>
    val totalBalanceSats: StateFlow<Long> get() = _totalBalanceSats

    init {
        val prefs = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
        val cachedLightning = prefs.getLong("cached_lightning_sats", 0L)
        val cachedOnchain = prefs.getLong("cached_onchain_sats", 0L)
        _lightningBalanceSats = MutableStateFlow(cachedLightning)
        _onchainBalanceSats = MutableStateFlow(cachedOnchain)
        _totalBalanceSats = MutableStateFlow(cachedLightning + cachedOnchain)
    }

    private val _pendingTradePayments = MutableStateFlow<Map<String, PendingTradePayment>>(emptyMap())
    val pendingTradePayments: StateFlow<Map<String, PendingTradePayment>> = _pendingTradePayments
    var pendingSplice: PendingSplice? = null
    var isChannelClosing = false
    var spliceTxid: String? = null
    var fundingTxid: String? = null
        set(value) {
            field = value
            context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                .putString("funding_txid", value).apply()
        }

    private val _paymentFlash = MutableStateFlow(false)
    val paymentFlash: StateFlow<Boolean> = _paymentFlash

    var onchainReceiveAddress: String? = null

    private var isSweeping = false
    /** True when any splice (in or out) is in flight — prevents concurrent splices. */
    val isSpliceInFlight: Boolean get() = isSweeping
    private var sweepOnchainStart: Long = 0
    private var prevOnchainSats: Long = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
        .getLong("cached_onchain_sats", 0L)
    private var stabilityJob: Job? = null

    /** Resolved esplora URL — Blockstream primary, mempool.space fallback. */
    var chainUrl: String = Constants.PRIMARY_CHAIN_URL
        private set

    private val httpClient = OkHttpClient()

    fun start() {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                // Resolve best esplora endpoint
                chainUrl = resolveChainUrl()

                databaseService = DatabaseService(context)
                databaseService?.seedHistoricalPrices()
                tradeService = TradeService(nodeService)

                val auditPath = File(Constants.userDataDir(context), "audit_log.txt").absolutePath
                AuditService.setLogPath(auditPath)

                loadChannelFromDB()
                priceService.startAutoRefresh()

                // Subscribe to LDK events
                launch { nodeService.events.collect { handleEvent(it) } }

                val seedFile = File(Constants.userDataDir(context), "keys_seed")
                val seedPhraseFile = File(Constants.userDataDir(context), "seed_phrase")
                if (seedFile.exists() || seedPhraseFile.exists()) {
                    _phase.value = Phase.SYNCING
                    waitForBackgroundService()
                    nodeService.start(Network.BITCOIN, chainUrl, null)
                    _phase.value = Phase.WALLET
                    refreshBalances()
                    // Restore fundingTxid
                    fundingTxid = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
                        .getString("funding_txid", null)
                    // Restore pending splice state
                    if (databaseService?.hasPendingSplice() == true) {
                        isSweeping = true
                        spliceTxid = databaseService?.getPendingSpliceTxid() ?: fundingTxid
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
                nodeService.start(Network.BITCOIN, chainUrl, mnemonic)
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
        stabilityJob?.cancel()
        priceService.stopAutoRefresh()
        nodeService.stop()
    }

    private fun handleEvent(event: Event) {
        viewModelScope.launch {
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
                    // Detect splice: same userChannelId, different channelId
                    val isSplice = sc.userChannelId == event.userChannelId && sc.channelId.isNotEmpty() && sc.channelId != event.channelId
                    sc.channelId = event.channelId
                    if (isSplice) {
                        isSweeping = false
                        spliceTxid = null
                        val price = priceService.currentPrice.value
                        val result = StabilityService.reconcileOutgoing(sc, price)
                        _stableChannel.value = result.first
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
                        AuditService.log("PAYMENT_FAILED", mapOf(
                            "payment_id" to (pid ?: ""),
                            "payment_hash" to (event.paymentHash ?: ""),
                            "reason" to reason
                        ))
                    }
                }
                is Event.SplicePending -> {
                    handleSplicePending(event.channelId, event.userChannelId, "${event.newFundingTxo.txid}:${event.newFundingTxo.vout}")
                }
                is Event.SpliceFailed -> {
                    isSweeping = false
                    spliceTxid = null
                    pendingSplice = null
                    AuditService.log("SPLICE_FAILED", mapOf("channel_id" to event.channelId))
                }
                is Event.ChannelClosed -> {
                    handleChannelClosed(event.channelId, event.userChannelId, event.reason?.toString())
                }
                else -> {}
            }
        }
    }

    private fun handlePaymentReceived(paymentId: String?, amountMsat: Long, paymentHash: String, customRecords: List<CustomTlvRecord>) {
        // Check for sync message
        if (handleSyncMessage(customRecords, paymentHash)) {
            refreshBalances()
            updateStableBalances()
            return
        }

        val price = priceService.currentPrice.value
        databaseService?.recordPayment(
            paymentId = paymentId, paymentType = "lightning", direction = "received",
            amountMsat = amountMsat, amountUSD = (amountMsat.toDouble() / 1000 / Constants.SATS_IN_BTC) * price,
            btcPrice = price, counterparty = _stableChannel.value.counterparty
        )
        refreshBalances()
        updateStableBalances()
        val sc = StabilityService.reconcileIncoming(_stableChannel.value)
        _stableChannel.value = sc
        saveChannelToDB()
        _statusMessage.value = "Payment received: ${amountMsat / 1000} sats"
        triggerPaymentFlash()
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
            refreshBalances()
            updateStableBalances()
            val price = priceService.currentPrice.value
            val result = StabilityService.reconcileOutgoing(_stableChannel.value, price)
            _stableChannel.value = result.first
            if (paymentId != null) {
                databaseService?.updatePaymentStatus(paymentId, "completed", feePaidMsat ?: 0)
            }
            saveChannelToDB()
            _statusMessage.value = "Payment confirmed"
        }
    }

    private fun handleSplicePending(channelId: String, userChannelId: String, newFundingTxo: String) {
        val txid = newFundingTxo.split(":").firstOrNull() ?: newFundingTxo
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
                        btcPrice = price, txid = newFundingTxo, address = splice.address
                    )
                }
            }
        }
        refreshBalances()
        updateStableBalances()
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
            val closeTxid = fundingTxid
            databaseService?.recordPayment(
                paymentId = closeTxid ?: channelId,
                paymentType = "channel_close",
                direction = "received",
                amountMsat = balanceSats * 1000,
                amountUSD = balanceUSD,
                btcPrice = if (price > 0) price else null,
                counterparty = sc.counterparty.ifEmpty { null },
                status = "completed",
                txid = closeTxid
            )

            databaseService?.deleteChannel(sc.userChannelId)
            _stableChannel.value = StableChannel.DEFAULT
        }

        // Refresh balances so lightning drops to 0 immediately
        refreshBalances()
        isChannelClosing = false
        _statusMessage.value = "Channel closed"
    }

    private fun startStabilityTimer() {
        stabilityJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) {
                delay(Constants.STABILITY_CHECK_INTERVAL_SECS * 1000)
                FCMService.updateHeartbeat(context)
                ensureLSPConnected()
                recordCurrentPrice()
                runStabilityCheck()
                detectOnchainDeposit()
            }
        }
    }

    private fun runStabilityCheck() {
        refreshBalances()
        updateStableBalances()
        val sc = _stableChannel.value
        val price = priceService.currentPrice.value

        // Do NOT recalculate backingSats here — it's set at trade time and stays fixed.
        // As price moves, the stability check detects drift and sends payments to rebalance.

        val result = StabilityService.checkStabilityAction(sc, price)

        if (result.action == StabilityService.StabilityAction.PAY) {
            val now = System.currentTimeMillis() / 1000
            if (now - sc.lastStabilityPayment < Constants.STABILITY_PAYMENT_COOLDOWN_SECS.toLong()) return

            val amountMsat = USD(abs(result.dollarsFromPar)).toMsats(price)
            if (amountMsat == 0L) return

            try {
                val paymentId = nodeService.sendKeysend(amountMsat, sc.counterparty)
                val updated = sc.copy(lastStabilityPayment = now)
                // Reset backing_sats to equilibrium after payment
                updated.backingSats = ((updated.expectedUSD.amount / price) * Constants.SATS_IN_BTC).toLong()
                StabilityService.recomputeNative(updated)
                _stableChannel.value = updated

                databaseService?.recordPayment(
                    paymentId = paymentId, paymentType = "stability", direction = "sent",
                    amountMsat = amountMsat,
                    amountUSD = (amountMsat.toDouble() / 1000 / Constants.SATS_IN_BTC) * price,
                    btcPrice = price, counterparty = sc.counterparty
                )
                AuditService.log("STABILITY_PAYMENT_SENT", mapOf("amount_msat" to amountMsat))
            } catch (e: Exception) {
                AuditService.log("STABILITY_PAYMENT_FAILED", mapOf("error" to (e.message ?: "")))
            }
        }
    }

    private fun detectOnchainDeposit() {
        val currentSats = nodeService.spendableOnchainSats()
        if (currentSats > prevOnchainSats && !isSweeping && pendingSplice == null) {
            val depositSats = currentSats - prevOnchainSats
            if (depositSats < 1000) {
                prevOnchainSats = currentSats
                return
            }
            val price = priceService.currentPrice.value
            val dedupId = "${System.currentTimeMillis() / 1000}_$depositSats"
            databaseService?.recordPayment(
                paymentId = dedupId, paymentType = "onchain", direction = "received",
                amountMsat = depositSats * 1000,
                amountUSD = (depositSats.toDouble() / Constants.SATS_IN_BTC) * price,
                btcPrice = price
            )
            AuditService.log("ONCHAIN_DEPOSIT_DETECTED", mapOf("sats" to depositSats))
        }
        prevOnchainSats = currentSats
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

        val feeRate = fetchFeeRate() ?: 2L
        val feeReserve = feeRate * 170  // ~170 vbytes for splice tx
        val spendable = nodeService.spendableOnchainSats()
        if (spendable <= feeReserve) {
            _statusMessage.value = "Insufficient on-chain balance"
            return
        }
        val sweepAmount = spendable - feeReserve

        try {
            nodeService.spliceIn(channel.userChannelId, channel.counterpartyNodeId, sweepAmount)
            isSweeping = true
            sweepOnchainStart = spendable
            pendingSplice = PendingSplice("in", sweepAmount)
            _statusMessage.value = "Moving $sweepAmount sats to channel..."

            databaseService?.recordPayment(
                paymentId = null, paymentType = "splice_in", direction = "received",
                amountMsat = sweepAmount * 1000,
                amountUSD = null, btcPrice = null, status = "pending"
            )
            AuditService.log("SWEEP_TO_CHANNEL", mapOf(
                "amount_sats" to sweepAmount,
                "fee_rate_sat_vb" to feeRate
            ))
        } catch (e: Exception) {
            _statusMessage.value = "Sweep failed: ${e.message}"
            AuditService.log("SWEEP_FAILED", mapOf("error" to (e.message ?: "")))
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
        try { node.connect(Constants.DEFAULT_LSP_PUBKEY, Constants.DEFAULT_LSP_ADDRESS, true) } catch (_: Exception) {}
    }

    fun refreshBalances() {
        nodeService.refreshChannels()
        val balances = nodeService.balances() ?: return
        val lightning = balances.totalLightningBalanceSats.toLong()
        val onchain = balances.totalOnchainBalanceSats.toLong()
        _lightningBalanceSats.value = lightning
        _onchainBalanceSats.value = onchain
        _totalBalanceSats.value = when {
            isChannelClosing -> onchain
            isSweeping -> lightning
            else -> lightning + onchain
        }

        // Cache for instant display on next launch
        context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
            .putLong("cached_lightning_sats", lightning)
            .putLong("cached_onchain_sats", onchain)
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

    fun saveChannelToDB() {
        val sc = _stableChannel.value
        if (sc.userChannelId.isEmpty()) return
        databaseService?.saveChannel(
            sc.channelId, sc.userChannelId, sc.expectedUSD.amount, sc.backingSats, sc.note,
            receiverSats = sc.stableReceiverBTC.sats,
            latestPrice = sc.latestPrice
        )
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

    private fun waitForBackgroundService() {
        if (!StabilityProcessingService.isRunning) return
        Log.d("AppState", "Waiting for background stability service to finish...")
        val deadline = System.currentTimeMillis() + 30_000
        while (StabilityProcessingService.isRunning && System.currentTimeMillis() < deadline) {
            Thread.sleep(500)
        }
        if (StabilityProcessingService.isRunning) {
            Log.w("AppState", "Background service still running after 30s, proceeding anyway")
        }
    }

    private fun reregisterPushTokenIfNeeded() {
        val nodeId = nodeService.nodeId
        if (nodeId.isEmpty()) return

        FCMService.saveNodeId(context, nodeId)

        FirebaseMessaging.getInstance().token.addOnSuccessListener { token ->
            FCMService.saveToken(context, token)
            viewModelScope.launch(Dispatchers.IO) {
                FCMService.registerTokenWithLSP(token, nodeId)
            }
        }
    }

    private fun processPendingPushPayment() {
        if (!FCMService.hasPendingPayment(context)) return
        Log.d("AppState", "Processing pending push payment")
        FCMService.clearPendingPayment(context)
        try {
            nodeService.node?.connect(Constants.DEFAULT_LSP_PUBKEY, Constants.DEFAULT_LSP_ADDRESS, true)
        } catch (_: Exception) {}
        refreshBalances()
        updateStableBalances()
    }
}
