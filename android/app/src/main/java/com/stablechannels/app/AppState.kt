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

    private val _totalBalanceSats = MutableStateFlow(0L)
    val totalBalanceSats: StateFlow<Long> = _totalBalanceSats

    private val _lightningBalanceSats = MutableStateFlow(0L)
    val lightningBalanceSats: StateFlow<Long> = _lightningBalanceSats

    private val _onchainBalanceSats = MutableStateFlow(0L)
    val onchainBalanceSats: StateFlow<Long> = _onchainBalanceSats

    private val _pendingTradePayments = MutableStateFlow<Map<String, PendingTradePayment>>(emptyMap())
    val pendingTradePayments: StateFlow<Map<String, PendingTradePayment>> = _pendingTradePayments
    var pendingSplice: PendingSplice? = null

    private val _paymentFlash = MutableStateFlow(false)
    val paymentFlash: StateFlow<Boolean> = _paymentFlash

    private var isSweeping = false
    /** True when any splice (in or out) is in flight — prevents concurrent splices. */
    val isSpliceInFlight: Boolean get() = isSweeping
    private var sweepOnchainStart: Long = 0
    private var prevOnchainSats: Long = 0
    private var stabilityJob: Job? = null

    private val httpClient = OkHttpClient()

    fun start() {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                databaseService = DatabaseService(context)
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
                    nodeService.start(Network.BITCOIN, Constants.DEFAULT_CHAIN_URL, null)
                    _phase.value = Phase.WALLET
                    refreshBalances()
                    prevOnchainSats = nodeService.spendableOnchainSats()
                    reregisterPushTokenIfNeeded()
                    processPendingPushPayment()
                    startStabilityTimer()
                } else {
                    // New wallet — auto-create
                    _phase.value = Phase.SYNCING
                    nodeService.start(Network.BITCOIN, Constants.DEFAULT_CHAIN_URL, null)
                    _phase.value = Phase.WALLET
                    refreshBalances()
                    prevOnchainSats = nodeService.spendableOnchainSats()
                    reregisterPushTokenIfNeeded()
                    startStabilityTimer()
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
                nodeService.start(Network.BITCOIN, Constants.DEFAULT_CHAIN_URL, mnemonic)
                _phase.value = Phase.WALLET
                refreshBalances()
                prevOnchainSats = nodeService.spendableOnchainSats()
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
                    refreshBalances()
                    AuditService.log("CHANNEL_PENDING", mapOf(
                        "channel_id" to event.channelId,
                        "user_channel_id" to event.userChannelId
                    ))
                }
                is Event.ChannelReady -> {
                    val sc = _stableChannel.value.copy()
                    // Detect splice: same userChannelId, different channelId
                    val isSplice = sc.userChannelId == event.userChannelId && sc.channelId.isNotEmpty() && sc.channelId != event.channelId
                    sc.channelId = event.channelId
                    if (isSplice) {
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
                        _statusMessage.value = "Payment failed"
                        AuditService.log("PAYMENT_FAILED", mapOf("payment_hash" to (event.paymentHash ?: "")))
                    }
                }
                is Event.SplicePending -> {
                    handleSplicePending(event.channelId, event.userChannelId, event.newFundingTxo)
                }
                is Event.SpliceFailed -> {
                    isSweeping = false
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
        val tlv = customRecords.find { it.typeId == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() } ?: return false
        val data = tlv.value.toByteArray()
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
        val splice = pendingSplice
        if (splice != null) {
            when (splice.direction) {
                "in" -> databaseService?.setPendingSpliceTxid(newFundingTxo)
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
            databaseService?.deleteChannel(sc.userChannelId)
            _stableChannel.value = StableChannel.DEFAULT
            AuditService.log("CHANNEL_CLOSED", mapOf(
                "channel_id" to channelId,
                "reason" to (reason ?: "unknown")
            ))
        }
    }

    private fun startStabilityTimer() {
        stabilityJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) {
                delay(Constants.STABILITY_CHECK_INTERVAL_SECS * 1000)
                FCMService.updateHeartbeat(context)
                recordCurrentPrice()
                runStabilityCheck()
                detectOnchainDeposit()
                runAutoSweep()
            }
        }
    }

    private fun runStabilityCheck() {
        refreshBalances()
        updateStableBalances()
        val sc = _stableChannel.value
        val price = priceService.currentPrice.value
        val result = StabilityService.checkStabilityAction(sc, price)

        if (result.action == StabilityService.StabilityAction.PAY) {
            val now = System.currentTimeMillis() / 1000
            if (now - sc.lastStabilityPayment < Constants.STABILITY_PAYMENT_COOLDOWN_SECS.toLong()) return

            val amountMsat = USD(abs(result.dollarsFromPar)).toMsats(price)
            if (amountMsat == 0L) return

            try {
                val paymentId = nodeService.sendKeysend(amountMsat, sc.counterparty)
                val updated = sc.copy(lastStabilityPayment = now)
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
        if (currentSats > prevOnchainSats && !isSweeping) {
            val depositSats = currentSats - prevOnchainSats
            if (depositSats < 1000) {
                prevOnchainSats = currentSats
                return
            }
            val price = priceService.currentPrice.value
            val dedupId = "onchain_${System.currentTimeMillis() / 1000}_$depositSats"
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

    private fun runAutoSweep() {
        if (isSweeping) {
            val currentOnchain = nodeService.spendableOnchainSats()
            if (currentOnchain < sweepOnchainStart) {
                isSweeping = false
                AuditService.log("AUTO_SWEEP_CONFIRMED")
            }
            return
        }

        val readyChannel = nodeService.channels.find { it.isChannelReady } ?: return
        val totalOnchain = nodeService.totalOnchainSats()
        if (totalOnchain < Constants.AUTO_SWEEP_MIN_SATS) return

        val feeRate = fetchFeeRate() ?: return
        val feeReserve = feeRate * 170  // ~170 vbytes for splice tx
        val spendable = nodeService.spendableOnchainSats()
        if (spendable <= feeReserve) return

        val sweepAmount = spendable - feeReserve
        try {
            isSweeping = true
            sweepOnchainStart = spendable
            val sc = _stableChannel.value
            pendingSplice = PendingSplice("in", sweepAmount)
            nodeService.spliceIn(sc.userChannelId, sc.counterparty, sweepAmount)

            val price = priceService.currentPrice.value
            databaseService?.recordPayment(
                paymentId = null, paymentType = "splice_in", direction = "sent",
                amountMsat = sweepAmount * 1000,
                amountUSD = (sweepAmount.toDouble() / Constants.SATS_IN_BTC) * price,
                btcPrice = price, status = "pending"
            )
            AuditService.log("AUTO_SWEEP_STARTED", mapOf("sats" to sweepAmount))
        } catch (e: Exception) {
            isSweeping = false
            pendingSplice = null
            AuditService.log("AUTO_SWEEP_FAILED", mapOf("error" to (e.message ?: "")))
        }
    }

    /** Manual sweep-to-channel (called from SettingsScreen).
     *  Guards against concurrent splices using the same isSweeping flag as auto-sweep. */
    fun manualSweepToChannel() {
        if (isSweeping) {
            _statusMessage.value = "Sweep already in progress"
            return
        }

        val channel = nodeService.channels.find { it.isChannelReady } ?: run {
            _statusMessage.value = "No ready channel"
            return
        }

        val spendable = nodeService.spendableOnchainSats()
        val feeReserve = 340L * 10  // conservative
        val sweepAmount = spendable - feeReserve
        if (sweepAmount <= 0) {
            _statusMessage.value = "Insufficient on-chain balance"
            return
        }

        try {
            isSweeping = true
            sweepOnchainStart = spendable
            pendingSplice = PendingSplice("in", sweepAmount)
            val sc = _stableChannel.value
            nodeService.spliceIn(sc.userChannelId, sc.counterparty, sweepAmount)
            _statusMessage.value = "Sweep initiated ($sweepAmount sats)"
            AuditService.log("MANUAL_SWEEP_INITIATED", mapOf("sats" to sweepAmount))
        } catch (e: Exception) {
            isSweeping = false
            pendingSplice = null
            _statusMessage.value = "Sweep failed: ${e.message}"
            AuditService.log("MANUAL_SWEEP_FAILED", mapOf("error" to (e.message ?: "")))
        }
    }

    private fun fetchFeeRate(): Long? {
        return try {
            val url = "${Constants.DEFAULT_CHAIN_URL}/fee-estimates"
            val request = Request.Builder().url(url).build()
            val response = httpClient.newCall(request).execute()
            val body = response.body?.string() ?: return null
            val json = JSONObject(body)
            json.optDouble("6", -1.0).let { if (it > 0) it.roundToLong() else null }
        } catch (_: Exception) { null }
    }

    fun refreshBalances() {
        nodeService.refreshChannels()
        val balances = nodeService.balances() ?: return
        val lightning = balances.totalLightningBalanceSats.toLong()
        val onchain = balances.totalOnchainBalanceSats.toLong()
        _totalBalanceSats.value = lightning + onchain
        _lightningBalanceSats.value = lightning
        _onchainBalanceSats.value = onchain
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
