package com.stablechannels.app

import android.content.Context
import android.content.SharedPreferences
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
import com.stablechannels.app.util.QRCodeUtils
import com.stablechannels.app.util.satsFormatted
import com.stablechannels.app.util.usdFormatted
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
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
         * Whether resuming a pending splice confirmation on this call should skip bumping
         * [spliceGeneration]. True only when this process is already actively monitoring the
         * exact txid being resumed — in that case bumping would advance the counter past the
         * value the still-running monitor captured, and since [startSpliceConfirmationMonitor]
         * early-returns without re-arming a same-txid/active-job monitor, nothing would ever
         * hold the new generation, wedging `isSweeping` forever once that monitor confirms.
         * A pure function (no AppState/Android dependency) so it's directly unit-testable.
         */
        fun shouldSkipGenerationBumpOnResume(
            monitorActive: Boolean,
            monitoredTxid: String?,
            resumedTxid: String?
        ): Boolean {
            val normalizedResumed = resumedTxid?.trim()
            return monitorActive && monitoredTxid != null && monitoredTxid == normalizedResumed
        }

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

        data class ChannelState(
            val hasReady: Boolean,
            val hasAnyChannel: Boolean = false,
            val isChannelClosing: Boolean = false,
            val isOpeningChannel: Boolean = false,
            val isSweeping: Boolean = false
        )

        fun calculateTotalBalance(
            lightning: Long,
            onchain: Long,
            pendingSweep: Long = 0L,
            channelState: ChannelState
        ): Long {
            return when {
                channelState.isChannelClosing -> onchain
                channelState.isOpeningChannel -> if (lightning > 0) lightning else onchain
                channelState.isSweeping -> lightning
                !channelState.hasReady && !channelState.hasAnyChannel -> onchain + pendingSweep
                else -> lightning + onchain
            }
        }

        fun calculateTotalBalance(
            lightning: Long,
            onchain: Long,
            hasReady: Boolean,
            isChannelClosing: Boolean = false,
            isSweeping: Boolean = false,
            pendingSweep: Long = 0L,
            isOpeningChannel: Boolean = false,
            hasAnyChannel: Boolean = false
        ): Long = calculateTotalBalance(
            lightning = lightning,
            onchain = onchain,
            pendingSweep = pendingSweep,
            channelState = ChannelState(
                hasReady = hasReady,
                hasAnyChannel = hasAnyChannel,
                isChannelClosing = isChannelClosing,
                isOpeningChannel = isOpeningChannel,
                isSweeping = isSweeping
            )
        )

        fun requiredConfirmationsForType(paymentType: String): Int {
            return when (paymentType) {
                "splice_in", "splice_out" -> 1
                else -> 6
            }
        }

        object BalanceCacheKey {
            const val PREFS_NAME = "balance_cache"
            const val LIGHTNING = "cached_lightning_sats"
            const val ONCHAIN = "cached_onchain_sats"
            const val SPENDABLE = "cached_spendable_sats"
            const val NATIVE = "cached_native_sats"
            const val PENDING_AMOUNT = "pending_outbound_onchain_sats"
            const val PENDING_IS_SEND_ALL = "pending_outbound_is_send_all"
            const val PENDING_BASELINE = "pending_outbound_baseline_sats"
            const val PENDING_TIMESTAMP = "pending_outbound_timestamp_secs"
            const val RECEIVE_ADDRESS = "onchain_receive_address"
            const val LAST_RECEIVE_TXID = "last_receive_txid"
            const val LAST_RECEIVE_TXID_ADDRESS = "last_receive_txid_address"
            const val LAST_CLOSE_TXID = "last_close_txid"
            const val LAST_CLOSE_TXID_AT = "last_close_txid_at"
            const val FUNDING_TXID = "funding_txid"
            const val CLOSING_FUNDING_TXID = "closing_funding_txid"
            const val CACHED_CHANNEL_ID = "cached_channel_id"
            const val CACHED_USER_CHANNEL_ID = "cached_user_channel_id"
            const val CACHED_EXPECTED_USD = "cached_expected_usd"
            /** Payment id of the last trade failure already shown in the status capsule. */
            const val LAST_SHOWN_TRADE_FAILURE = "last_shown_trade_failure"
            const val PENDING_TXIDS = "pending_outbound_txids"

            fun clearPendingOutbound(context: Context) {
                context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit()
                    .remove(PENDING_AMOUNT)
                    .remove(PENDING_IS_SEND_ALL)
                    .remove(PENDING_BASELINE)
                    .remove(PENDING_TIMESTAMP)
                    .remove(PENDING_TXIDS)
                    .apply()
            }

            fun clearAll(context: Context) {
                context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit().clear().apply()
            }
        }

        /** A single pending broadcast entry pairing a transaction id with its sent amount.
         * Enables per-transaction resolution so mixed succeeded/failed batches release
         * only the resolved portion instead of blocking the entire aggregate. */
        data class TxEntry(val txid: String, val amountSats: Long)

        data class PendingOutboundSend(
            val isSendAll: Boolean = false,
            val baselineOnchainSats: Long = 0L,
            val timestampSecs: Long = System.currentTimeMillis() / 1000L,
            val entries: List<TxEntry> = emptyList()
        ) {
            /** Backward-compatible constructor accepting aggregate amount and flat txid list. */
            constructor(
                amountSats: Long,
                isSendAll: Boolean = false,
                baselineOnchainSats: Long = 0L,
                timestampSecs: Long = System.currentTimeMillis() / 1000L,
                txids: List<String> = emptyList()
            ) : this(
                isSendAll = isSendAll,
                baselineOnchainSats = baselineOnchainSats,
                timestampSecs = timestampSecs,
                entries = if (txids.isNotEmpty() && amountSats > 0L) {
                    val perTx = amountSats / txids.size
                    val remainder = amountSats % txids.size
                    txids.mapIndexed { i, tid ->
                        TxEntry(tid, perTx + if (i.toLong() < remainder) 1L else 0L)
                    }
                } else if (amountSats > 0L) {
                    listOf(TxEntry("", amountSats))
                } else {
                    emptyList()
                }
            )

            /** Aggregate pending amount across all unresolved entries. */
            val amountSats: Long get() = entries.sumOf { it.amountSats }
            /** All pending txids for predicate checks. */
            val txids: List<String> get() = entries.map { it.txid }
            /** First broadcast txid, if any. */
            val txid: String? get() = entries.firstOrNull()?.txid

            companion object {
                /** Backward-compatible factory: delegates to legacy constructor. */
                fun fromLegacy(
                    amountSats: Long,
                    isSendAll: Boolean,
                    baselineOnchainSats: Long,
                    timestampSecs: Long,
                    txids: List<String>
                ): PendingOutboundSend = PendingOutboundSend(
                    amountSats = amountSats,
                    isSendAll = isSendAll,
                    baselineOnchainSats = baselineOnchainSats,
                    timestampSecs = timestampSecs,
                    txids = txids
                )
            }
        }

        /**
         * Derives user-facing on-chain and spendable balances by subtracting any pending
         * outbound send that has not yet been incorporated into LDK/BDK's raw wallet view.
         */
        fun calculateEffectiveBalances(
            rawOnchain: Long,
            rawSpendable: Long,
            pending: PendingOutboundSend
        ): Pair<Long, Long> {
            if (pending.isSendAll) {
                return Pair(0L, 0L)
            }
            val amount = pending.amountSats
            if (amount > 0L) {
                val rawDrop = if (rawOnchain < pending.baselineOnchainSats) {
                    pending.baselineOnchainSats - rawOnchain
                } else {
                    0L
                }
                val pendingToDeduct = if (amount > rawDrop) {
                    amount - rawDrop
                } else {
                    0L
                }
                val onchain = (rawOnchain - pendingToDeduct).coerceAtLeast(0L)
                val spendable = (rawSpendable - pendingToDeduct).coerceAtLeast(0L)
                return Pair(onchain, spendable)
            }
            return Pair(rawOnchain, rawSpendable)
        }

        /**
         * Resolves pending outbound send state against a fresh raw on-chain balance observation.
         * Performs per-txid resolution: transactions whose authoritative status is known
         * (incorporated or failed) are removed individually, allowing partial clearing of
         * mixed-status batches instead of all-or-nothing.
         * Fails closed during extended indexer/node outages to prevent re-exposing spent funds.
         */
        fun resolvePendingOutboundSend(
            rawOnchain: Long,
            pending: PendingOutboundSend,
            currentTimestampSecs: Long = System.currentTimeMillis() / 1000L,
            ttlSecs: Long = 600L,
            isTxIncorporated: ((String) -> Boolean)? = null,
            isTxFailed: ((String) -> Boolean)? = null,
            isTxConfirmed: ((String) -> Boolean)? = null
        ): PendingOutboundSend {
            val incorporated = isTxIncorporated ?: isTxConfirmed
            if (pending.amountSats == 0L && !pending.isSendAll) {
                return pending
            }

            // 1. Per-txid resolution: remove entries whose txid has a terminal or incorporated status.
            var unresolvedEntries = pending.entries
            if (unresolvedEntries.isNotEmpty()) {
                unresolvedEntries = unresolvedEntries.filter { entry ->
                    if (entry.txid.isBlank()) return@filter true
                    // Failed transactions: release the deduction (funds were never spent).
                    if (isTxFailed != null && isTxFailed(entry.txid)) return@filter false
                    // Incorporated transactions: wallet already reflects the spend.
                    if (incorporated != null && incorporated(entry.txid)) return@filter false
                    true
                }
                // If all entries resolved, clear the entire record.
                if (unresolvedEntries.isEmpty()) {
                    return PendingOutboundSend()
                }
                // If some entries resolved, re-check raw balance drop against the reduced aggregate.
                if (unresolvedEntries.size < pending.entries.size) {
                    val resolved = PendingOutboundSend(
                        isSendAll = pending.isSendAll,
                        baselineOnchainSats = pending.baselineOnchainSats,
                        timestampSecs = pending.timestampSecs,
                        entries = unresolvedEntries
                    )
                    return resolveByBalanceDrop(rawOnchain, resolved)
                }
            }

            // 2. No per-txid resolution occurred; check raw balance drop.
            return resolveByBalanceDrop(rawOnchain, pending)
        }

        /** Checks whether the raw on-chain balance has dropped enough to account for the
         * remaining pending deduction. Pure helper for resolvePendingOutboundSend. */
        private fun resolveByBalanceDrop(
            rawOnchain: Long,
            pending: PendingOutboundSend
        ): PendingOutboundSend {
            if (pending.isSendAll) {
                if (rawOnchain == 0L) return PendingOutboundSend()
                return pending
            }
            val amount = pending.amountSats
            if (amount > 0L) {
                val expectedRemaining = (pending.baselineOnchainSats - amount).coerceAtLeast(0L)
                if (rawOnchain <= expectedRemaining) return PendingOutboundSend()
                // Fail closed: retain deduction until authoritative reconciliation.
                return pending
            }
            return pending
        }

        /**
         * Pure helper to evaluate if a background wallet sync completion owns the active send generation
         * and succeeded, preventing older out-of-order syncs from clearing newer pending broadcasts.
         */
        fun shouldClearPendingOnSyncCompletion(
            expectedGeneration: Long,
            currentGeneration: Long,
            syncSuccess: Boolean
        ): Boolean {
            return syncSuccess && expectedGeneration == currentGeneration
        }

        /**
         * Records an immediate outbound send broadcast and returns the updated pending state.
         * Pure helper ensuring architectural parity with iOS BalanceCalculator.recordBroadcast.
         */
        fun recordBroadcast(
            currentPending: PendingOutboundSend,
            amountSats: Long,
            isSendAll: Boolean,
            currentOnchain: Long,
            timestampSecs: Long = System.currentTimeMillis() / 1000L,
            txid: String? = null
        ): PendingOutboundSend {
            val baseline = if (currentPending.baselineOnchainSats == 0L) {
                currentOnchain
            } else {
                currentPending.baselineOnchainSats
            }
            val sendAmount = if (isSendAll) currentOnchain else amountSats
            val updatedEntries = currentPending.entries.toMutableList()
            if (!txid.isNullOrBlank()) {
                if (updatedEntries.none { it.txid == txid }) {
                    updatedEntries.add(TxEntry(txid, sendAmount))
                }
            } else {
                // No txid available yet; append an anonymous entry.
                updatedEntries.add(TxEntry("", sendAmount))
            }

            return PendingOutboundSend(
                isSendAll = isSendAll || currentPending.isSendAll,
                baselineOnchainSats = baseline,
                timestampSecs = timestampSecs,
                entries = updatedEntries
            )
        }

        /**
         * Deserializes cached pending outbound send state from SharedPreferences.
         * Supports per-txid colon-delimited format as well as legacy flat comma-delimited txid lists.
         */
        fun loadCachedPendingOutboundSend(prefs: SharedPreferences): PendingOutboundSend {
            val pendingAmount = prefs.getLong(BalanceCacheKey.PENDING_AMOUNT, 0L)
            val pendingIsSendAll = prefs.getBoolean(BalanceCacheKey.PENDING_IS_SEND_ALL, false)
            val pendingBaseline = prefs.getLong(BalanceCacheKey.PENDING_BASELINE, 0L)
            val pendingTimestamp = prefs.getLong(BalanceCacheKey.PENDING_TIMESTAMP, 0L)
            val pendingTxidsStr = prefs.getString(BalanceCacheKey.PENDING_TXIDS, "") ?: ""

            if (pendingAmount == 0L && !pendingIsSendAll) {
                return PendingOutboundSend(
                    isSendAll = false,
                    baselineOnchainSats = 0L,
                    timestampSecs = 0L,
                    entries = emptyList()
                )
            }

            val parts = if (pendingTxidsStr.isNotBlank()) {
                pendingTxidsStr.split(",").filter { it.isNotBlank() }
            } else {
                emptyList()
            }
            val hasEntryFormat = parts.any { it.contains(":") }

            val entries: List<TxEntry> = if (hasEntryFormat) {
                parts.mapNotNull { part ->
                    val components = part.split(":", limit = 2)
                    if (components.size == 2) {
                        val amt = components[1].toLongOrNull() ?: return@mapNotNull null
                        TxEntry(components[0], amt)
                    } else null
                }
            } else {
                // Legacy path: distribute stored aggregate across txids.
                if (parts.isNotEmpty() && pendingAmount > 0L) {
                    val perTx = pendingAmount / parts.size
                    val remainder = pendingAmount % parts.size
                    parts.mapIndexed { i, tid ->
                        TxEntry(tid, perTx + if (i.toLong() < remainder) 1L else 0L)
                    }
                } else if (pendingAmount > 0L) {
                    listOf(TxEntry("", pendingAmount))
                } else {
                    emptyList()
                }
            }

            return PendingOutboundSend(
                isSendAll = pendingIsSendAll,
                baselineOnchainSats = pendingBaseline,
                timestampSecs = if (pendingTimestamp > 0L) pendingTimestamp else (System.currentTimeMillis() / 1000L),
                entries = entries
            )
        }

        /**
         * Serializes pending outbound send state to SharedPreferences.
         */
        fun persistPendingOutboundSend(editor: SharedPreferences.Editor, pending: PendingOutboundSend) {
            editor
                .putLong(BalanceCacheKey.PENDING_AMOUNT, pending.amountSats)
                .putBoolean(BalanceCacheKey.PENDING_IS_SEND_ALL, pending.isSendAll)
                .putLong(BalanceCacheKey.PENDING_BASELINE, pending.baselineOnchainSats)
                .putLong(BalanceCacheKey.PENDING_TIMESTAMP, pending.timestampSecs)
                .putString(
                    BalanceCacheKey.PENDING_TXIDS,
                    pending.entries.joinToString(",") { "${it.txid}:${it.amountSats}" }
                )
        }
    }

    val nodeService = NodeService(context)
    val priceService = PriceService(context)
    val priceChartService: PriceChartFetcher = PriceChartService.shared
    private val isBackfillingHourly = java.util.concurrent.atomic.AtomicBoolean(false)
    private val isBackfillingDaily = java.util.concurrent.atomic.AtomicBoolean(false)
    var databaseService: DatabaseService? = null
        private set
    var tradeService: TradeService? = null
        private set
    // Bounds how long a stuck signed trade-sync message can keep retrying before we give up on
    // it, since NodeService's event queue is strictly sequential and won't process the next LDK
    // event (e.g. Event.ChannelClosed) until this one is acknowledged. Backed by SharedPreferences
    // (not just in-memory) because LDK persists an un-acked event and redelivers it after the app
    // process restarts — which Android can do well before 5 continuous minutes of foreground time
    // ever accumulate, so an in-memory-only clock would reset every restart and never give up.
    private val syncRetryTracker = SyncRetryTracker(
        loadFirstAttempt = { key ->
            context.getSharedPreferences("sync_retry_tracker", Context.MODE_PRIVATE)
                .getLong("first_attempt_$key", -1L)
                .takeIf { it >= 0 }
        },
        saveFirstAttempt = { key, ts ->
            // commit() (synchronous, blocks until written) instead of apply() (async): apply()'s
            // write can still be pending when Android SIGKILLs the process (background limits,
            // low memory), which has no graceful-shutdown hook to flush it — losing the very
            // timestamp this mechanism exists to survive process death for.
            context.getSharedPreferences("sync_retry_tracker", Context.MODE_PRIVATE)
                .edit().putLong("first_attempt_$key", ts).commit()
        },
        clearFirstAttempt = { key ->
            context.getSharedPreferences("sync_retry_tracker", Context.MODE_PRIVATE)
                .edit().remove("first_attempt_$key").commit()
        }
    )
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

    private val _paymentOutcomes = MutableStateFlow<Map<String, PaymentOutcome>>(emptyMap())
    val paymentOutcomes: StateFlow<Map<String, PaymentOutcome>> = _paymentOutcomes

    /** Background consumers may have acknowledged the event; LDK retains terminal status. */
    fun refreshPaymentOutcome(paymentId: String, attemptStartedAtNanos: Long) {
        viewModelScope.launch(Dispatchers.IO) {
            if (_paymentOutcomes.value[paymentId]?.belongsToAttempt(attemptStartedAtNanos) == true) return@launch
            val status = try { nodeService.node?.payment(paymentId)?.status } catch (_: Exception) { null }
            val outcome = when (status) {
                PaymentStatus.SUCCEEDED -> PaymentOutcome(true, "Payment confirmed")
                PaymentStatus.FAILED -> PaymentOutcome(false, "Payment failed: ${WalletErrorMessages.paymentFailure(null)}")
                else -> return@launch
            }
            // Never replace a more specific event reason that arrived during the lookup.
            _paymentOutcomes.update { outcomes ->
                if (outcomes[paymentId]?.belongsToAttempt(attemptStartedAtNanos) == true) outcomes
                else outcomes + (paymentId to outcome)
            }
        }
    }

    fun recordOutgoingLightningPayment(paymentId: String, paymentType: String, amountMsat: Long, price: Double) {
        val db = databaseService ?: throw IllegalStateException("Payment sent; history is unavailable. Check its status before retrying.")
        db.recordPendingLightningPayment(paymentId, paymentType, amountMsat, price)
        // A terminal event may beat the history insert. Read the node after writing pending;
        // any later event will update the now-existing row through the normal event handler.
        val payment = nodeService.node?.payment(paymentId)
        when (payment?.status) {
            PaymentStatus.SUCCEEDED -> db.updatePaymentStatus(paymentId, "completed", payment.feePaidMsat?.toLong() ?: 0)
            PaymentStatus.FAILED -> db.updatePaymentStatus(paymentId, "failed")
            else -> {}
        }
    }

    /** Repair outbound Lightning rows for events acknowledged while the app was backgrounded. */
    private fun reconcilePendingLightningPayments() {
        val db = databaseService ?: return
        val repaired = LightningPaymentRecovery.reconcilePending(db) { paymentId ->
            try {
                nodeService.node?.payment(paymentId)?.let { payment ->
                    when (payment.status) {
                        PaymentStatus.SUCCEEDED -> LightningPaymentResolution(true, payment.feePaidMsat?.toLong() ?: 0L)
                        PaymentStatus.FAILED -> LightningPaymentResolution(false)
                        else -> null
                    }
                }
            } catch (_: Exception) {
                null
            }
        }
        if (repaired > 0) {
            AuditService.log("PENDING_LIGHTNING_RECONCILED", mapOf("count" to repaired))
        }
    }

    private val _lightningBalanceSats: MutableStateFlow<Long>
    val lightningBalanceSats: StateFlow<Long> get() = _lightningBalanceSats

    private val _onchainBalanceSats: MutableStateFlow<Long>
    val onchainBalanceSats: StateFlow<Long> get() = _onchainBalanceSats

    private val _totalBalanceSats: MutableStateFlow<Long>
    val totalBalanceSats: StateFlow<Long> get() = _totalBalanceSats
    private val _hasReadyChannel = MutableStateFlow(false)
    val hasReadyChannel: StateFlow<Boolean> get() = _hasReadyChannel
    private val _pendingSweepBalanceSats = MutableStateFlow(0L)
    val pendingSweepBalanceSats: StateFlow<Long> get() = _pendingSweepBalanceSats

    private val _onchainReceiveAddress = MutableStateFlow<String?>(null)
    val onchainReceiveAddress: StateFlow<String?> get() = _onchainReceiveAddress

    private val _lastReceiveTxid = MutableStateFlow<String?>(null)
    val lastReceiveTxid: StateFlow<String?> get() = _lastReceiveTxid
    private var lastReceiveTxidAddress: String? = null

    private val _lastCloseTxid = MutableStateFlow<String?>(null)
    val lastCloseTxid: StateFlow<String?> get() = _lastCloseTxid

    fun setLastCloseTxid(txid: String?) {
        _lastCloseTxid.value = txid
        val editor = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE).edit()
        if (txid != null) {
            editor.putString(BalanceCacheKey.LAST_CLOSE_TXID, txid)
            editor.putLong(BalanceCacheKey.LAST_CLOSE_TXID_AT, System.currentTimeMillis())
        } else {
            editor.remove(BalanceCacheKey.LAST_CLOSE_TXID)
            editor.remove(BalanceCacheKey.LAST_CLOSE_TXID_AT)
        }
        editor.apply()
    }

    private fun setLastReceiveTxid(txid: String?, address: String?) {
        _lastReceiveTxid.value = txid
        lastReceiveTxidAddress = address

        val editor = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE).edit()
        if (txid.isNullOrBlank()) {
            editor.remove(BalanceCacheKey.LAST_RECEIVE_TXID)
            editor.remove(BalanceCacheKey.LAST_RECEIVE_TXID_ADDRESS)
        } else {
            editor.putString(BalanceCacheKey.LAST_RECEIVE_TXID, txid)
            if (!address.isNullOrBlank()) {
                editor.putString(BalanceCacheKey.LAST_RECEIVE_TXID_ADDRESS, address)
            } else {
                editor.remove(BalanceCacheKey.LAST_RECEIVE_TXID_ADDRESS)
            }
        }
        editor.apply()
    }

    fun resetInMemoryWalletState() {
        synchronized(pendingLock) {
            sendGeneration++
            pendingOutboundSend = PendingOutboundSend()
        }
        BalanceCacheKey.clearAll(context)
    }

    private val _spendableOnchainSats = MutableStateFlow(
        context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE).getLong(BalanceCacheKey.SPENDABLE, 0L)
    )
    val spendableOnchainSats: StateFlow<Long> = _spendableOnchainSats

    private val pendingLock = Any()

    /** Serializes "commit a stable-books mutation, then publish the row to _stableChannel"
     *  across the four payment paths that change expected_usd/stable_sats: runStabilityCheck()
     *  (decision re-validation and post-send debit), reconcilePendingOutgoingStabilityPayment(),
     *  handlePaymentReceived() and handlePaymentSuccessful()'s ordinary-send reconcile.
     *  Without it, path A can commit+publish between path B's commit and B's publish, and B's
     *  publish (built from B's own transaction result or an earlier snapshot) then overwrites
     *  A's newer books in memory — which is what the stability check reads (#299 review).
     *
     *  NOT yet covered (pre-existing, tracked as a follow-up to #299): the trade-sync apply
     *  paths under processSignedSyncMessage() and completeConfirmedSplice()'s full save. Both
     *  write these columns from in-memory state without taking this lock. */
    private val booksLock = Any()
    private var sendGeneration: Long = 0L

    @Volatile
    var pendingOutboundSend: PendingOutboundSend = PendingOutboundSend()
        private set

    private val _nativeSats: MutableStateFlow<Long>
    val nativeSats: StateFlow<Long> get() = _nativeSats

    init {
        val prefs = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE)
        val cachedLightning = prefs.getLong(BalanceCacheKey.LIGHTNING, 0L)
        val cachedOnchain = prefs.getLong(BalanceCacheKey.ONCHAIN, 0L)
        pendingOutboundSend = loadCachedPendingOutboundSend(prefs)
        _lightningBalanceSats = MutableStateFlow(cachedLightning)
        _onchainBalanceSats = MutableStateFlow(cachedOnchain)
        _totalBalanceSats = MutableStateFlow(cachedLightning + cachedOnchain)
        _nativeSats = MutableStateFlow(prefs.getLong(BalanceCacheKey.NATIVE, 0L))
        _onchainReceiveAddress.value = prefs.getString(BalanceCacheKey.RECEIVE_ADDRESS, null)?.let { QRCodeUtils.normalizeAddress(it) }
        _lastReceiveTxid.value = prefs.getString(BalanceCacheKey.LAST_RECEIVE_TXID, null)
        lastReceiveTxidAddress = prefs.getString(BalanceCacheKey.LAST_RECEIVE_TXID_ADDRESS, null)

        val closeAt = prefs.getLong(BalanceCacheKey.LAST_CLOSE_TXID_AT, 0L)
        if (System.currentTimeMillis() - closeAt < 7 * 86400 * 1000L) {
            _lastCloseTxid.value = prefs.getString(BalanceCacheKey.LAST_CLOSE_TXID, null)
        } else {
            prefs.edit()
                .remove(BalanceCacheKey.LAST_CLOSE_TXID)
                .remove(BalanceCacheKey.LAST_CLOSE_TXID_AT)
                .apply()
        }

        // Restore cached channel state so UI shows correct slider position immediately
        val cachedChannelId = prefs.getString(BalanceCacheKey.CACHED_CHANNEL_ID, null)
        val cachedUserChannelId = prefs.getString(BalanceCacheKey.CACHED_USER_CHANNEL_ID, null)
        val cachedExpectedUsd = prefs.getFloat(BalanceCacheKey.CACHED_EXPECTED_USD, 0f)
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

    /** Signed results and definitive fee failures, keyed by the trade's fee payment id. */
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
            // update {} — this runs on an IO thread while the handler path writes from
            // the event loop, and a read-modify-write on .value could drop an entry.
            _tradeOutcomes.update { outcomes ->
                outcomes + (paymentId to terminal)
            }
            _pendingTradePayments.update { it - paymentId }
        }
    }

    private fun refreshAllTradeOutcomes(paymentIds: Collection<String>) {
        paymentIds.forEach { refreshTradeOutcome(it) }
    }
    var pendingSplice: PendingSplice? = null
    private val _isOpeningChannel = MutableStateFlow(false)
    val isOpeningChannelFlow: StateFlow<Boolean> = _isOpeningChannel
    var isOpeningChannel: Boolean
        get() = _isOpeningChannel.value
        set(value) {
            _isOpeningChannel.value = value
        }
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
    // Identifies the current splice operation, independent of pendingSplice/spliceTxid's own
    // lifecycle (both can be null across a process restart). Bumped once per operation at the
    // earliest point it exists — row creation (beginSpliceOut/sweepToChannel) or resumption after
    // restart — NOT on every SpliceNegotiated, so a replayed/duplicate negotiation event for the
    // same operation is never mistaken for a newer one. A handler captures this at entry; if it
    // no longer matches when an async check resolves, a genuinely newer operation has since
    // started and none of this handler's in-memory cleanup may run against it.
    private val spliceGeneration = AtomicLong(0L)
    var spliceTxid: String? = null
    var fundingTxid: String? = null
        set(value) {
            field = value
            context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                .putString("funding_txid", value).apply()
        }
    // Mirrors fundingTxid: the real output index of the channel's funding transaction, needed
    // so CloseTxidResolver polls the correct /tx/{txid}/outspend/{vout} endpoint instead of
    // assuming vout 0 (a funding output isn't always at index 0 — see #264).
    var fundingVout: Int? = null
        set(value) {
            field = value
            context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
                .putInt("funding_vout", value ?: -1).apply()
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
    private val _chartUpdateTrigger = MutableStateFlow(0L)
    val chartUpdateTrigger: StateFlow<Long> = _chartUpdateTrigger

    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(4, TimeUnit.SECONDS)
        .readTimeout(4, TimeUnit.SECONDS)
        .callTimeout(6, TimeUnit.SECONDS)
        .build()
    private val spliceBroadcastChecker = SpliceBroadcastChecker(httpClient)

    fun start() {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val db = DatabaseService(context)
                databaseService = db
                launch {
                    databaseService?.seedHistoricalPrices()
                    _chartUpdateTrigger.value = System.currentTimeMillis()
                }
                launch {
                    backfillHourlyPrices()
                    backfillDailyPrices()
                }
                tradeService = TradeService(nodeService, db)
                db.markExpiredTradesUncertain()
                _pendingTradePayments.value = db.unresolvedTradePayments()
                refreshAllTradeOutcomes(_tradeOutcomes.value.keys + _pendingTradePayments.value.keys)
                surfaceUnseenTradeFailure()

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
                    reconcilePendingLightningPayments()
                    // Restore the known funding txid before the first live balance refresh so
                    // an ordinary cold start is not mistaken for a funding transition.
                    val balanceCachePrefs = context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE)
                    fundingTxid = balanceCachePrefs.getString("funding_txid", null)
                    fundingVout = balanceCachePrefs.getInt("funding_vout", -1).takeIf { it >= 0 }
                    refreshBalances()
                    pollPaymentConfirmations(force = true)
                    // Off the critical startup path: it makes blocking LDK/DB calls, and the
                    // first frames must not wait on a repair that almost never has work to do.
                    launch {
                        updateStableBalances()
                        repairBooksAboveLiveBalance()
                    }
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
                                // Resume background resolver if it hasn't found the TX yet.
                                // An unknown vout must NOT default to 0 — that's a real, possibly
                                // different output of the same funding tx, and CloseTxidResolver
                                // would accept whatever spent it as the close txid (see #264).
                                // Leave the row unresolved instead of guessing; it will resolve
                                // once fundingVout is known (e.g. after refreshBalances() backfills
                                // it on the next tick, if the channel is still visible to LDK).
                                val closeFundingTxid = fundingTxid
                                val closeVout = fundingVout
                                if (closeFundingTxid != null && closeVout != null && databaseService != null) {
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
                                            vout = closeVout,
                                            databaseService = databaseService!!
                                        )
                                    }
                                } else if (closeFundingTxid != null && closeVout == null) {
                                    AuditService.log("CLOSE_TXID_RESOLVE_SKIPPED_UNKNOWN_VOUT", mapOf("payment_id" to pendingCloseId))
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
                    reconcilePendingLightningPayments()
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
                reconcilePendingLightningPayments()
                _chartUpdateTrigger.value = System.currentTimeMillis()
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
                reconcilePendingLightningPayments()
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
                reconcilePendingLightningPayments()
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
                fundingVout = event.fundingTxo.vout.toInt()
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
                val recorded = pid?.let {
                    val db = databaseService ?: throw IllegalStateException("Trade database unavailable")
                    PaymentFailureRecorder.record(db, it, event.reason?.name) {
                        nodeService.node?.payment(it)?.takeIf { payment ->
                            payment.kind is PaymentKind.Spontaneous && payment.direction == PaymentDirection.OUTBOUND
                        }?.amountMsat?.toLong()
                    }
                }
                when {
                    recorded?.isTrade == true -> recorded.tradeOutcome?.let { outcome ->
                        _pendingTradePayments.update { it - pid!! }
                        _tradeOutcomes.update { it + (pid!! to outcome) }
                        if (outcome.sendFailed) _statusMessage.value = outcome.message
                    }
                    recorded?.isStability == true -> {
                        _statusMessage.value = "Stability payment failed. The wallet will retry when ready."
                    }
                    else -> {
                        val message = "Payment failed: ${WalletErrorMessages.paymentFailure(event.reason)}"
                        _statusMessage.value = message
                        if (pid != null) _paymentOutcomes.update { it + (pid to PaymentOutcome(false, message)) }
                    }
                }
                AuditService.log("PAYMENT_FAILED", mapOf(
                    "payment_id" to (pid ?: ""),
                    "payment_hash" to (event.paymentHash ?: ""),
                    "reason" to (event.reason?.name ?: "unknown"),
                    "is_trade" to (recorded?.isTrade == true),
                    "is_stability" to (recorded?.isStability == true)
                ))
            }
            is Event.SpliceNegotiated -> {
                handleSplicePending(event.channelId, event.userChannelId, "${event.newFundingTxo.txid}:${event.newFundingTxo.vout}")
            }
            is Event.SpliceNegotiationFailed -> {
                // Snapshot to local vals: spliceTxid/pendingSplice/spliceGeneration can all be
                // mutated concurrently by the IO-thread confirmation monitor or by a brand-new
                // splice starting, so re-reading them after the async check below would be a
                // TOCTOU race that could apply this branch's rollback to a different, newer
                // splice — including the pre-negotiation (capturedTxid == null) branch below,
                // which can otherwise fire for a stale/duplicate replay after a newer operation
                // has already taken pendingSplice's place.
                val capturedTxid = spliceTxid
                val capturedGeneration = spliceGeneration.get()
                val capturedPaymentRowId = pendingSplice?.paymentRowId
                if (capturedTxid != null) {
                    // A signed splice tx exists. It may already be broadcast/confirmed (even by
                    // the counterparty), in which case this failed event is a stale/duplicate
                    // replay — rolling back would mark a real splice "failed" and desync Stable
                    // USD from the node's actual balance. But the tx could also have been
                    // genuinely abandoned before broadcast (or this could be a real failure on a
                    // later attempt), so verify against esplora rather than assuming: if the tx
                    // was never broadcast, this is a real failure, and it must be handled or the
                    // confirmation monitor + "Move" lock (isSweeping) hang forever.
                    when (doesTxExist(capturedTxid)) {
                        TxBroadcastStatus.EXISTS -> {
                            AuditService.log("SPLICE_FAILED_IGNORED_STALE", mapOf(
                                "channel_id" to event.channelId,
                                "splice_txid" to capturedTxid
                            ))
                        }
                        TxBroadcastStatus.INCONCLUSIVE -> {
                            // Can't prove the tx doesn't exist (timeouts/429/5xx/no connectivity)
                            // — preserve the splice rather than risk a false failure.
                            AuditService.log("SPLICE_FAILED_CHECK_INCONCLUSIVE", mapOf(
                                "channel_id" to event.channelId,
                                "splice_txid" to capturedTxid
                            ))
                        }
                        TxBroadcastStatus.NOT_FOUND -> {
                            // The DB row genuinely failed regardless of what's current now — this
                            // uses the captured row id, never a live re-read, so it can only ever
                            // touch the row that belonged to this specific splice.
                            databaseService?.failPendingSplice(capturedPaymentRowId)
                            if (spliceGeneration.get() != capturedGeneration) {
                                // A newer splice has started while the check was in flight — none
                                // of its in-memory state belongs to this stale handler.
                                AuditService.log("SPLICE_FAILED_STALE_GENERATION", mapOf(
                                    "channel_id" to event.channelId,
                                    "splice_txid" to capturedTxid
                                ))
                            } else {
                                isSweeping = false
                                spliceConfirmationJob?.cancel()
                                spliceConfirmationJob = null
                                monitoredSpliceTxid = null
                                pendingSplice = null
                                if (spliceTxid == capturedTxid) spliceTxid = null
                                AuditService.log("SPLICE_FAILED", mapOf(
                                    "channel_id" to event.channelId,
                                    "splice_txid" to capturedTxid,
                                    "reason" to "txid_never_broadcast"
                                ))
                            }
                        }
                    }
                } else {
                    // Pre-negotiation failure. The row still failed regardless of what's current
                    // now. The generation guard here is defensive rather than closing a real race:
                    // capture and compare happen back-to-back on the same event-loop thread with
                    // no async work in between, so in practice it can only ever match — but it
                    // keeps this branch structurally consistent with the one above and costs
                    // nothing if some future change adds a suspend point here.
                    databaseService?.failPendingSplice(capturedPaymentRowId)
                    if (spliceGeneration.get() == capturedGeneration) {
                        isSweeping = false
                        spliceConfirmationJob?.cancel()
                        spliceConfirmationJob = null
                        monitoredSpliceTxid = null
                        pendingSplice = null
                        AuditService.log("SPLICE_FAILED", mapOf("channel_id" to event.channelId))
                    } else {
                        AuditService.log("SPLICE_FAILED_STALE_GENERATION", mapOf("channel_id" to event.channelId))
                    }
                }
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
            // The sync message itself is already resolved (applied/invalid/duplicate/given-up).
            // A failure refreshing UI-facing balances afterward must not cause the whole event
            // to be redelivered — NodeService can't tell "sync failed" from "balance refresh
            // failed" once it retries the raw event, so that would silently restart a fresh
            // 5-minute syncRetryTracker window for a message that already gave up.
            try {
                refreshBalances()
                updateStableBalances()
            } catch (e: Exception) {
                Log.e("AppState", "Post-sync balance refresh failed", e)
            }
            return
        }

        val price = priceService.currentPrice.value
        val signedRecord = customRecords.firstOrNull {
            it.typeNum == Constants.SIGNED_STABILITY_TLV_TYPE.toULong()
        }
        var isStabilityPayment = false
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
        var sc0 = _stableChannel.value
        // Always use paymentHash as fallback so dedup check runs even when paymentId is null.
        val effectiveId = paymentId ?: paymentHash
        if (signedRecord != null &&
            (sc0.userChannelId.isEmpty() || sc0.channelId.isEmpty())
        ) {
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
        // A valid signed STABILITY_PAYMENT_V1 record is the only stability classifier —
        // the legacy [0x01] marker is gone (#270); without one this is ordinary Lightning.
        var settlementId: String? = null
        if (signedRecord != null) {
            if (sc0.channelId.isEmpty() || sc0.userChannelId.isEmpty()) {
                // Discovery above could not recover local state. This is a retryable local
                // condition, not a bad envelope: demoting it to a Lightning receipt would dedupe
                // the payment id and make the backing credit unrecoverable. Nack instead.
                AuditService.log("STABILITY_PAYMENT_STATE_UNAVAILABLE", mapOf(
                    "payment_id" to (paymentId ?: ""),
                    "payment_hash" to paymentHash,
                    "amount_msat" to amountMsat
                ))
                throw Exception(
                    "Channel state unavailable for signed settlement — not acknowledging, will retry"
                )
            }
            when (val validation = StabilityPaymentProtocol.validateInbound(
                signedRecord.value,
                sc0.counterparty,
                sc0.channelId,
                amountMsat
            ) { msg, sig, pk -> nodeService.verifySignature(msg, sig, pk) }) {
                is SignedSettlementValidation.Valid -> {
                    isStabilityPayment = true
                    settlementId = validation.payment.settlementId
                }
                is SignedSettlementValidation.Invalid -> {
                    // An invalid signed record must not credit backing — record the keysend as
                    // an ordinary Lightning receipt instead (mirrors desktop user.rs).
                    AuditService.log("STABILITY_PAYMENT_INVALID", mapOf(
                        "payment_id" to (paymentId ?: ""),
                        "payment_hash" to paymentHash,
                        "amount_msat" to amountMsat,
                        "reason" to validation.reason
                    ))
                    isStabilityPayment = false
                }
            }
        }
        val paymentType = if (isStabilityPayment) "stability" else "lightning"
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
                backingDeltaSats = backingDelta,
                settlementId = settlementId
            ) ?: throw Exception("DB service unavailable")
        }
        val persistence = synchronized(booksLock) {
            val p = try {
                record()
            } catch (e: MissingChannelRowException) {
                // The channels row vanished (e.g. DB recreated) — rebuild it from in-memory state
                // via the full save, then retry once. If it still fails, rethrow to nack.
                Log.w("AppState", "Channel row missing during payment persist — recreating and retrying: ${e.message}")
                AuditService.log("CHANNEL_ROW_RECREATED", mapOf("user_channel_id" to (userChannelId ?: "")))
                saveChannelToDB()
                record()
            }
            if (isStabilityPayment) {
                p.backingSats ?: throw Exception("DB did not return backing after stability payment")
                // Publish the credited backing from the row, under booksLock, for the same reason
                // as the outgoing paths: an absolute taken from this transaction can overwrite a
                // newer value another path committed and published in the meantime.
                publishBooksFromDB()
            }
            // The balance refresh, native recompute and save below are a read-modify-write of
            // the books too — the save writes expected_usd from memory — so they stay inside
            // the lock. Released early, an ordinary-send reconcile could commit and publish in
            // between and this save would write the pre-reconcile target back (#299 review).
            refreshBalances()
            updateStableBalances()
            _stableChannel.update { StabilityService.reconcileIncoming(it) }
            saveChannelToDB(preserveBacking = isStabilityPayment)
            p
        }
        if (settlementId != null && !persistence.isNewPayment) {
            AuditService.log("STABILITY_PAYMENT_REPLAY_IGNORED", mapOf(
                "settlement_id" to settlementId,
                "payment_hash" to paymentHash
            ))
        }
        if (persistence.isNewPayment) {
            val usdVal = (amountMsat.toDouble() / 1000.0 / Constants.SATS_IN_BTC) * price
            _statusMessage.value = "Payment received: ${usdVal.usdFormatted()}"
            triggerPaymentFlash()
        }
    }

    // Retries the current LDK event (by throwing RetryableSyncException, which NodeService
    // catches and re-delivers the same un-acked event) up to syncRetryTracker's time bound.
    // Past that bound we give up and ack the event instead, so a message that can never commit
    // (e.g. a stale/unreachable channel row) doesn't block every subsequent LDK event forever —
    // including Event.ChannelClosed, which is required to resolve a channel-close receive's txid.
    // A post-apply channel reload can fail because the channel closed. If it's gone from both
    // the local DB and the node's own live channel list, it will never come back — this sync
    // message can never be applied, so drop it permanently instead of waiting out the retry
    // bound. Only fall back to the timed retry when the reload might just be a transient race
    // (e.g. the local row hasn't caught up with a channel that's still actually open).
    private fun deferOrDropForMissingChannel(paymentHash: String, reason: String): Boolean {
        nodeService.refreshChannels()
        val stillLive = nodeService.channels.any { it.userChannelId == _stableChannel.value.userChannelId }
        if (!stillLive) {
            AuditService.log("TRADE_RESULT_CHANNEL_GONE", mapOf("payment_hash" to paymentHash, "reason" to reason))
            syncRetryTracker.clear(paymentHash)
            // Note: calling node.removePayment() here was tried and confirmed ineffective —
            // ldk-node's own replay of an un-acked PaymentClaimable event on restart is driven
            // by its internal channel-manager/HTLC-claim bookkeeping, not by the payment store
            // removePayment() clears. The event can still resurface once per restart even after
            // this drop; each occurrence is now instant (no 5-minute wait) so the residual
            // impact is negligible. A durable fix for the resurfacing itself would need to land
            // in ldk-node, not here.
            return true
        }
        return deferSyncOrGiveUp(paymentHash, reason)
    }

    private fun deferSyncOrGiveUp(paymentHash: String, reason: String): Boolean {
        if (syncRetryTracker.recordAttemptAndShouldGiveUp(paymentHash)) {
            AuditService.log("TRADE_RESULT_GIVEN_UP", mapOf("payment_hash" to paymentHash, "reason" to reason))
            return true
        }
        AuditService.log("TRADE_RESULT_DEFERRED", mapOf("payment_hash" to paymentHash, "reason" to reason))
        throw RetryableSyncException(reason)
    }

    private fun handleSyncMessage(
        customRecords: List<CustomTlvRecord>,
        paymentHash: String,
        amountMsat: Long
    ): Boolean {
        val tlv = customRecords.find { it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() } ?: return false
        val data = tlv.value
        if (data.contentEquals(byteArrayOf(1))) return false
        if (amountMsat != TradeProtocol.RESULT_CONTROL_AMOUNT_MSAT) {
            AuditService.log("TRADE_RESULT_AMOUNT_INVALID", mapOf("payment_hash" to paymentHash, "amount_msat" to amountMsat))
            return true
        }
        val message = TradeProtocol.parseSignedControl(data, _stableChannel.value.counterparty) { msg, sig, pk ->
            nodeService.verifySignature(msg, sig, pk)
        } ?: run {
            AuditService.log("TRADE_RESULT_INVALID", mapOf("payment_hash" to paymentHash))
            return true
        }
        // Bound the ENTIRE remaining processing, not just the anticipated RETRY paths. An
        // unanticipated exception from the apply calls below (a bug, a transient SQL error,
        // etc.) must still go through the same bounded give-up accounting as an explicit RETRY —
        // otherwise it bypasses syncRetryTracker entirely, and NodeService's outer catch retries
        // the raw event with unbounded backoff forever, blocking every later LDK event forever.
        return try {
            processSignedSyncMessage(message, paymentHash, amountMsat)
        } catch (e: RetryableSyncException) {
            throw e
        } catch (e: Exception) {
            Log.e("AppState", "Unexpected error processing signed sync message", e)
            deferSyncOrGiveUp(paymentHash, "Unexpected error: ${e.message}")
        }
    }

    private fun processSignedSyncMessage(
        message: TradeControlMessage,
        paymentHash: String,
        amountMsat: Long
    ): Boolean {
        val db = databaseService
            ?: return deferSyncOrGiveUp(paymentHash, "Trade database unavailable")
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
                        return deferSyncOrGiveUp(paymentHash, "Cannot apply SYNC_V1 without a trusted BTC price")
                    }
                    db.applyUncorrelatedSyncIfNewer(message, price)
                }
            }
        }
        // Only clear the retry tracker once we're actually done retrying this payment_hash (i.e.
        // we won't immediately call deferSyncOrGiveUp again below). Clearing unconditionally here
        // for DUPLICATE/APPLIED wiped the persisted first-attempt right before the loadChannel
        // fallback below could re-defer, so a permanently stuck message never accumulated any
        // retry time at all — each retry looked like a brand-new first attempt.
        if (result.status == TradeControlApplyStatus.INVALID) {
            syncRetryTracker.clear(paymentHash)
        }
        when (result.status) {
            TradeControlApplyStatus.RETRY -> {
                try { db.markTradeResponseNotCommittable(message) } catch (_: Exception) {}
                return deferSyncOrGiveUp(paymentHash, "Signed trade result could not be committed")
            }
            TradeControlApplyStatus.INVALID -> {
                AuditService.log("TRADE_RESULT_INVALID", mapOf("payment_hash" to paymentHash))
                return true
            }
            TradeControlApplyStatus.DUPLICATE -> {
                result.paymentId?.let { paymentId ->
                    db.terminalTradeOutcome(paymentId)?.let { outcome ->
                        _pendingTradePayments.update { it - paymentId }
                        _tradeOutcomes.update { it + (paymentId to outcome) }
                    }
                }
                if (message is TradeControlMessage.Rejected) {
                    syncRetryTracker.clear(paymentHash)
                    return true
                }
                val channel = db.loadChannel(_stableChannel.value.userChannelId)
                    ?: return deferOrDropForMissingChannel(paymentHash, "Duplicate result channel could not be reloaded")
                syncRetryTracker.clear(paymentHash)
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
                    db.terminalTradeOutcome(paymentId)?.let { outcome ->
                        _pendingTradePayments.update { it - paymentId }
                        _tradeOutcomes.update { it + (paymentId to outcome) }
                    }
                }
                if (message is TradeControlMessage.Rejected) {
                    syncRetryTracker.clear(paymentHash)
                    _statusMessage.value = TradeProtocol.rejectionMessage(message.reasonCode)
                    // Shown now, so the next launch must not repeat it.
                    markTradeFailureSeen(message.correlation.tradePaymentId)
                    AuditService.log("TRADE_REJECTED_BY_LSP", mapOf("payment_id" to message.correlation.tradePaymentId,
                        "reason_code" to message.reasonCode))
                    return true
                }
                val channel = db.loadChannel(_stableChannel.value.userChannelId)
                    ?: return deferOrDropForMissingChannel(paymentHash, "Applied result channel could not be reloaded")
                syncRetryTracker.clear(paymentHash)
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
                if (result.paymentId != null) {
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
            refreshTradeOutcome(paymentId)
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

        // Ordinary (non-trade, non-stability) outgoing payment. Mirrors iOS's
        // handlePaymentSuccessful: reconcile expectedUSD and backingSats together when this
        // send dipped into the stable backing — otherwise the on-screen Stable USD never
        // reflects the send and the balances stop adding up to the total. The original bug
        // (#296) was persisting only the expectedUSD half of that result via
        // saveChannelToDB(preserveBacking = true), permanently desyncing the two fields.
        //
        // The reconcile math itself has to run inside the DB transaction that persists it, not
        // be precomputed against an in-memory snapshot: the stability timer (runStabilityCheck(),
        // a separate in-process coroutine) can commit its own backing debit straight to this row
        // via recordPaymentAndMaybeUpdateBacking() at any time. reconcileOutgoing()'s result is a
        // function of the backing value it's given, so computing it against a snapshot taken
        // before that debit — then applying the result as a delta on top of the DB's
        // already-debited row — double-counts the difference. reconcileOutgoingBacking() instead
        // re-reads expected_usd/stable_sats fresh and does the whole computation inside one
        // BEGIN IMMEDIATE transaction, composing correctly with whatever the timer already wrote.
        refreshBalances()
        updateStableBalances()
        val price = priceService.currentPrice.value
        val channelId = _stableChannel.value.channelId
        val userChannelId = _stableChannel.value.userChannelId
        val note = _stableChannel.value.note
        val latestPrice = _stableChannel.value.latestPrice
        // stableReceiverBTC is refreshed from live channel state just above, not from the
        // racy in-memory backingSats copy — safe to use directly as the reconcile input.
        val receiverSats = _stableChannel.value.stableReceiverBTC.sats
        // The transaction that mutates the books and the in-memory publish of its result run
        // under booksLock, and the publish re-reads the row rather than trusting the
        // transaction's return value. Every other path that mutates expected_usd/stable_sats
        // takes the same lock, so none of them can commit-and-publish between this commit and
        // this publish — the interleaving that let an older absolute backing overwrite a newer
        // one in memory and trigger a phantom stability payment (#299 review).
        val reconcileResult = synchronized(booksLock) {
            val result = if (userChannelId.isEmpty()) null else try {
                databaseService?.reconcileOutgoingBacking(
                    channelId = channelId,
                    userChannelId = userChannelId,
                    note = note,
                    receiverSats = receiverSats,
                    latestPrice = latestPrice,
                    price = price
                )
            } catch (e: MissingChannelRowException) {
                // Structural: there is no row to reconcile against and a retry can't create one.
                // Treat as nothing-to-reconcile, but leave a trace in the audit log.
                AuditService.log("OUTGOING_RECONCILE_SKIPPED", mapOf(
                    "payment_id" to (paymentId ?: ""),
                    "reason" to "missing_channel_row",
                    "user_channel_id" to userChannelId
                ))
                null
            } catch (e: Exception) {
                // Anything else (SQLite I/O error, disk full, lock timeout) is transient. The
                // transaction rolled back, so the deduction has NOT been recorded — rethrow so
                // the event loop leaves this PaymentSuccessful un-acked and LDK redelivers it
                // with backoff. reconcileOutgoingBacking() is idempotent on retry because it
                // measures overflow against live channel state. Swallowing the error here acked
                // the payment with the books still wrong (#299 review, P2).
                AuditService.log("OUTGOING_RECONCILE_FAILED", mapOf(
                    "payment_id" to (paymentId ?: ""),
                    "error" to (e.message ?: e.javaClass.simpleName),
                    "will_retry" to true
                ))
                throw e
            }
            if (result != null) {
                // receiverSats above is live post-send channel state, so native is safe to
                // recompute against it here.
                publishBooksFromDB(
                    lastStabilityPayment = System.currentTimeMillis() / 1000,
                    recomputeNative = true
                )
                // reconcileOutgoingBacking() bypasses saveChannelToDB(), the usual writer of the
                // SharedPreferences launch cache — refresh it so the next cold start doesn't
                // briefly show the pre-send Stable USD.
                cacheBalanceForLaunch()
            }
            result
        }
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
        if (reconcileResult != null) {
            AuditService.log("OUTGOING_STABLE_DEDUCTED", mapOf(
                "payment_id" to (paymentId ?: ""),
                "usd_deducted" to reconcileResult.usdDeducted,
                "old_expected_usd" to reconcileResult.oldExpectedUSD,
                "new_expected_usd" to reconcileResult.newExpectedUSD,
                "btc_price" to price
            ))
        } else {
            // Nothing to reconcile (or the reconcile attempt failed) — only
            // expectedUSD-independent metadata (status, note, price) may have changed.
            // preserveBacking keeps this call from ever touching stable_sats, so it's always
            // safe regardless of any concurrent stability write.
            saveChannelToDB(preserveBacking = true)
        }
        val feeSuffix = feePaidMsat?.let { " (fee: ${(it / 1000).satsFormatted()} sats)" } ?: ""
        val successMsg = if (displayVal != null) "Payment sent: $displayVal$feeSuffix" else "Payment sent$feeSuffix"
        _statusMessage.value = successMsg
        if (paymentId != null) _paymentOutcomes.update { it + (paymentId to PaymentOutcome(true, successMsg)) }
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
                } else {
                    FCMService.flagPendingPayment(context)
                    saveChannelToDB(preserveBacking = true)
                    _statusMessage.value = "Payment confirmed; syncing stability payment"
                }
                return true
            }

            if (!reconciled) {
                if (!paymentId.isNullOrEmpty()) {
                    databaseService?.updatePaymentStatus(paymentId, "completed", feePaidMsat ?: 0)
                }
                saveChannelToDB(preserveBacking = true)
                _statusMessage.value = "Payment confirmed; syncing stability payment"
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
        return true
    }

    private fun handleSplicePending(channelId: String, userChannelId: String, newFundingTxo: String) {
        val txid = newFundingTxo.split(":").firstOrNull() ?: newFundingTxo
        // Deliberately not bumping spliceGeneration here: it's established once at operation
        // creation (beginSpliceOut/sweepToChannel/resumePendingSpliceConfirmation), before this
        // event can even fire. A replayed/duplicate SpliceNegotiated for the same operation must
        // not look like a new one, or a stale monitor holding the old generation would never see
        // its cleanup run on confirmation (isSweeping wedged until process restart).
        isSweeping = true
        spliceTxid = txid
        fundingTxid = txid
        fundingVout = newFundingTxo.split(":").getOrNull(1)?.toIntOrNull()
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
        spliceGeneration.incrementAndGet()
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
        // Captured once here, not re-read later: completeConfirmedSplice must finalize the row
        // that belonged to THIS operation, never whatever pendingSplice happens to hold by the
        // time confirmation is observed (which could by then belong to a newer operation).
        val monitorGeneration = spliceGeneration.get()
        val monitorPaymentRowId = pendingSplice?.paymentRowId
        spliceConfirmationJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) {
                if (isTxConfirmed(normalizedTxid)) {
                    completeConfirmedSplice(normalizedTxid, monitorGeneration, monitorPaymentRowId)
                    break
                }
                delay(30_000)
            }
        }
    }

    private fun resumePendingSpliceConfirmation() {
        if (databaseService?.hasPendingSplice() != true) return
        val txid = databaseService?.getPendingSpliceTxid() ?: spliceTxid
        // In-process resumption (foreground grace-period reconnect, or startup racing a replayed
        // SpliceNegotiated) of an operation this instance is already actively monitoring is not a
        // new operation — bumping here would advance the counter past the value the still-running
        // monitor captured, and since startSpliceConfirmationMonitor below early-returns without
        // re-arming (same txid, active job), nothing would ever hold the new generation. That
        // wedges isSweeping forever once the untouched monitor eventually confirms.
        val alreadyMonitoring = shouldSkipGenerationBumpOnResume(
            monitorActive = spliceConfirmationJob?.isActive == true,
            monitoredTxid = monitoredSpliceTxid,
            resumedTxid = txid
        )
        if (!alreadyMonitoring) {
            spliceGeneration.incrementAndGet()
        }
        isSweeping = true
        spliceTxid = txid
        txid?.takeIf { it.isNotBlank() }?.let { startSpliceConfirmationMonitor(it) }
    }

    /**
     * Whether esplora has ever heard of this txid (broadcast, mempool, or confirmed) — distinct
     * from isTxConfirmed(), which only reports confirmation depth. Used to tell a genuinely
     * abandoned/never-broadcast splice tx (404 everywhere) apart from a stale failure event for a
     * splice that did make it on-chain. If every endpoint errors out or times out (no connectivity,
     * 429/5xx, mixed results), we can't prove non-existence, so this returns INCONCLUSIVE rather
     * than a boolean — the caller treats that the same as "exists" (preserve the splice) since the
     * safer failure mode is treating a real failure as a stale replay (recoverable manually) rather
     * than mislabeling a possibly real splice as failed.
     */
    private fun doesTxExist(txid: String): TxBroadcastStatus {
        val urls = listOf(chainUrl, Constants.PRIMARY_CHAIN_URL, Constants.FALLBACK_CHAIN_URL).distinct()
        return spliceBroadcastChecker.checkStatus(txid, urls)
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

    private fun completeConfirmedSplice(txid: String, expectedGeneration: Long, capturedPaymentRowId: Long?) {
        // If SPLICE_TXID_UNMATCHED fired when this splice was negotiated (assignPendingSpliceTxid
        // found no unambiguous pending row), the DB row's txid is still NULL and completeSplice()
        // — which requires an exact txid match — can never find it, permanently desyncing Stable
        // USD from the confirmed on-chain balance. Retry the assignment now that the tx has
        // confirmed; assignPendingSpliceTxid is a no-op if a row already carries this txid.
        // Uses the row id captured when this monitor started, NOT the live pendingSplice — by the
        // time this tx confirms, pendingSplice may already belong to a newer operation, and
        // reading it here could bind this (older, unrelated) txid to that newer row.
        databaseService?.assignPendingSpliceTxid(txid, capturedPaymentRowId)
        // Books first, row second. completeSplice() only matches a row that is still 'pending',
        // so marking it complete before the deduction is durable turns a crash in between into a
        // permanently unaccounted withdrawal — nothing revisits a completed row (#311). With the
        // order reversed, a crash leaves the row pending, the resume path runs this again, and
        // the reconcile is idempotent (it only ever removes backing above the live balance), so
        // the deduction lands exactly once either way.
        val matchesPendingRow = try {
            databaseService?.hasPendingSpliceFor(txid) == true
        } catch (e: Exception) {
            Log.w("AppState", "Could not check the pending splice row: ${e.message}")
            false
        }
        var completed = false
        if (matchesPendingRow) {
            synchronized(booksLock) {
                refreshBalances()
                updateStableBalances()
                val price = priceService.currentAccountingPrice()
                if (price > 0.0) {
                    val result = StabilityService.reconcileOutgoing(_stableChannel.value, price)
                    val reconciled = result.first
                    if (result.second != null) {
                        reconciled.lastStabilityPayment = System.currentTimeMillis() / 1000
                    }
                    _stableChannel.value = reconciled
                    saveChannelToDB()
                } else {
                    // No trusted price to value the spend: leave the row pending so the resume
                    // path retries, rather than completing it with the books untouched.
                    AuditService.log("SPLICE_RECONCILE_DEFERRED", mapOf(
                        "txid" to txid, "reason" to "untrusted_price"
                    ))
                }
            }
            if (priceService.currentAccountingPrice() > 0.0) {
                completed = databaseService?.completeSplice(txid) == true
                if (completed) {
                    // History only reloads when this epoch moves, and the confirmation poller no
                    // longer touches splice rows at all, so without this bump an open History
                    // screen keeps showing "0/1 confirmed" until it is reopened (#304).
                    _confirmationUpdateEpoch.value = _confirmationUpdateEpoch.value + 1
                }
            }
        }

        // Only clear the shared in-memory splice state if a newer splice hasn't since replaced
        // it — otherwise this stale monitor tears down the newer operation's state instead.
        if (spliceGeneration.get() == expectedGeneration) {
            isSweeping = false
            pendingSplice = null
            sweepOnchainStart = 0
            if (spliceTxid == txid) spliceTxid = null
            monitoredSpliceTxid = null
            spliceConfirmationJob = null
            _statusMessage.value = "Move confirmed"
            // Unlike "Move pending confirmation" (which the user can dismiss by tapping, or
            // which naturally gets replaced by a later status), "Move confirmed" is terminal —
            // nothing else ever overwrites or clears it, so without this it would sit in the
            // status capsule forever. Auto-clear it a few seconds later, but only if some other
            // event hasn't already replaced it with a newer message in the meantime.
            viewModelScope.launch {
                delay(4_000)
                if (_statusMessage.value == "Move confirmed") {
                    _statusMessage.value = ""
                }
            }
        } else {
            AuditService.log("SPLICE_CONFIRM_STALE_GENERATION", mapOf("txid" to txid))
        }

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
            val closeFundingVout = fundingVout
                ?: context.getSharedPreferences("balance_cache", android.content.Context.MODE_PRIVATE)
                    .getInt("funding_vout", -1).takeIf { it >= 0 }
            if (closeFundingTxid != null && closeFundingVout != null && databaseService != null) {
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
                        vout = closeFundingVout,
                        databaseService = databaseService!!
                    )
                }
            } else if (closeFundingTxid != null && closeFundingVout == null) {
                // Unknown vout must not default to 0 — that could be a different, unrelated
                // output of the same funding tx, and CloseTxidResolver would accept whatever
                // spent it as the close txid (this was #264's exact bug). Leave the row
                // unresolved rather than risk attaching the wrong transaction.
                AuditService.log("CLOSE_TXID_RESOLVE_SKIPPED_UNKNOWN_VOUT", mapOf("payment_id" to paymentId))
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
            _statusMessage.value = "Trade result delayed; waiting for the provider's decision. Do not place the order again."
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
        val targetAddress = QRCodeUtils.normalizeAddress(address)
        if (normalizedTxid.isEmpty() || targetAddress.isBlank()) return null

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
                        val voutAddress = QRCodeUtils.normalizeAddress(vout.optString("scriptpubkey_address", ""))
                        if (voutAddress == targetAddress) {
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
                try {
                    nodeService.syncWallets()
                } catch (_: Exception) {}
                refreshBalances()
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
        // Retry a repair that startup deferred (no trusted price yet, or an operation still in
        // flight). The guard is a free in-memory comparison, so this costs nothing on the tick
        // where the books are already consistent — which is every tick but the broken ones.
        if (_stableChannel.value.backingSats > _stableChannel.value.stableReceiverBTC.sats) {
            repairBooksAboveLiveBalance()
        }
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

            // Stable allocations are sat-denominated — floor to whole sats so the signed
            // amount matches the keysend exactly (mirrors src/stable.rs).
            val amountMsat = (USD(abs(result.dollarsFromPar)).toMsats(price) / 1000L) * 1000L
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

            // Re-validate under booksLock now that the send is claimed. The decision above was
            // made on the tick-top `sc`; an ordinary-send reconcile or an incoming settlement
            // may have committed and published since, leaving the books already on par. Re-read
            // the row and re-decide at the same price; if the answer or the amount changed,
            // release the claim and let the next tick decide afresh. This narrows the
            // stale-decision window to the sign+send below — it cannot be closed without
            // holding the lock across a network call, which would block the LDK event handler
            // (#299 review).
            val revalidated = synchronized(booksLock) {
                publishBooksFromDB()
                _stableChannel.value
            }
            val recheck = StabilityService.checkStabilityAction(revalidated, price)
            val recheckedAmountMsat = if (recheck.action == StabilityService.StabilityAction.PAY) {
                (USD(abs(recheck.dollarsFromPar)).toMsats(price) / 1000L) * 1000L
            } else 0L
            if (recheckedAmountMsat != amountMsat) {
                try { databaseService?.clearPendingSend() } catch (_: Exception) {}
                AuditService.log("STABILITY_SKIP", mapOf(
                    "reason" to "books_changed_after_decision",
                    "claimed_amount_msat" to amountMsat,
                    "rechecked_amount_msat" to recheckedAmountMsat
                ))
                return
            }

            val paymentId = try {
                // Attach only the signed STABILITY_PAYMENT_V1 envelope — the legacy
                // STABLE_CHANNEL_TLV [0x01] marker is gone (#270). If the envelope can't
                // be built, release the claim and skip the payment entirely.
                val signedEnvelope = StabilityPaymentProtocol.buildSignedEnvelope(
                    channelId = revalidated.channelId,
                    amountMsat = amountMsat,
                    expectedUsd = revalidated.expectedUSD.amount,
                    sign = { payload -> nodeService.signMessage(payload) }
                )
                if (signedEnvelope == null) {
                    try { databaseService?.clearPendingSend() } catch (_: Exception) {}
                    AuditService.log("STABILITY_SKIP", mapOf("reason" to "envelope_build_failed"))
                    return
                }
                val records = listOf(CustomTlvRecord(
                    Constants.SIGNED_STABILITY_TLV_TYPE.toULong(),
                    signedEnvelope.toByteArray(Charsets.UTF_8)
                ))
                nodeService.sendStabilityPayment(amountMsat, sc.counterparty, records)
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
                synchronized(booksLock) {
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
                    persistence.backingSats
                        ?: throw IllegalStateException("DB did not return backing after outgoing stability payment")
                    // Publish from the row — not from persistence.backingSats and not from the
                    // tick-top `sc` snapshot. An ordinary send can reconcile (commit + publish)
                    // at any point; republishing `sc` clobbered its expectedUSD, and publishing
                    // the transaction-returned absolute backing clobbered its newer backing,
                    // leaving in-memory books off-par and the next tick paying for nothing
                    // (#299 review). Under booksLock the read-and-publish can't interleave with
                    // another path's commit-and-publish. The debit is already durable and
                    // lastStabilityPayment isn't a column, so no save is needed here.
                    publishBooksFromDB(lastStabilityPayment = now)
                }
                databaseService?.clearPendingSend()
                AuditService.log("STABILITY_PAYMENT_SENT", mapOf("amount_msat" to amountMsat))
            } catch (e: Exception) {
                // The send already succeeded. Keep the durable marker and block all later sends
                // until the payment row and backing delta can be committed together.
                _stableChannel.update { it.copy(lastStabilityPayment = now) }
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
            synchronized(booksLock) {
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
                persistence.backingSats
                    ?: throw IllegalStateException("DB did not return backing during outgoing reconciliation")
                // Same rule as runStabilityCheck(): publish from the row under booksLock. The
                // old snapshot republish + preserveBacking save here wrote a stale expected_usd
                // over a concurrent ordinary-send reconcile (#299 review).
                publishBooksFromDB()
            }
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
                // Only an address match is authoritative enough to persist a txid — it's proof
                // this specific tx pays our own tracked receive address. Matching by amount and
                // timestamp proximity against LDK's payment list is not proof of identity (an
                // unrelated same-amount payment can be the only visible candidate), so — mirroring
                // iOS's DepositRecorder, which either resolves via a direct address lookup or
                // leaves the row permanently txid-less — we never guess here. This branch only
                // runs when there's no pending channel close (that case, the reported #264 bug,
                // has its own authoritative fix via fundingVout-based CloseTxidResolver above);
                // an ordinary receive with no tracked address is a rare edge case (e.g. an
                // LSP-initiated on-chain funding outside the app's own receive flow). If it
                // happens, the row is left without a txid/confirmation link — there is currently
                // no way to retroactively recover it, including by generating a new address.
                val resolvedTxid = _lastReceiveTxid.value?.takeIf {
                    !it.isNullOrBlank() &&
                        !receiveAddress.isNullOrBlank() &&
                        lastReceiveTxidAddress == receiveAddress
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
        spliceGeneration.incrementAndGet()
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
        val normalized = address?.let { QRCodeUtils.normalizeAddress(it) }
        val oldAddress = _onchainReceiveAddress.value
        _onchainReceiveAddress.value = normalized
        val editor = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE).edit()
        if (normalized == null) {
            editor.remove(BalanceCacheKey.RECEIVE_ADDRESS)
        } else {
            editor.putString(BalanceCacheKey.RECEIVE_ADDRESS, normalized)
        }
        editor.apply()

        if (!oldAddress.isNullOrBlank() && oldAddress != normalized) {
            mempoolWebSocketService.untrackAddress(oldAddress)
        }

        if (!normalized.isNullOrBlank() && oldAddress != normalized) {
            // New receive request: drop stale txid from previous address/session.
            setLastReceiveTxid(null, null)
        }

        if (normalized == null) {
            return
        }

        mempoolWebSocketService.trackAddress(normalized)
        
        // Start polling for this address to be hit
        viewModelScope.launch {
            val esploraUrl = com.stablechannels.app.util.Constants.PRIMARY_CHAIN_URL
            val txid = com.stablechannels.app.services.OnchainTxidResolver.resolve(normalized, esploraUrl)
            if (txid != null) {
                setLastReceiveTxid(txid, normalized)
                databaseService?.reconcileResolvedReceiveTxid(txid, normalized)
            }
        }
    }

    fun prepareChannelCloseTracking(userChannelId: String) {
        setLastCloseTxid(null)
        val liveChannel = nodeService.channels
            .firstOrNull { it.userChannelId == userChannelId || it.isChannelReady }
        val liveTxid = liveChannel?.fundingTxo?.txid

        if (!liveTxid.isNullOrBlank()) {
            fundingTxid = liveTxid
            fundingVout = liveChannel.fundingTxo?.vout?.toInt()
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
        val rawOnchain = balances.totalOnchainBalanceSats.toLong()
        val rawSpendable = balances.spendableOnchainBalanceSats.toLong()
        val hasReady = nodeService.channels.any { it.isChannelReady }

        // Resolve pending outbound deduction against raw wallet observation
        val effectivePending = synchronized(pendingLock) {
            // Wallet-incorporation predicate: once LDK tracks the txid (pending or succeeded),
            // the wallet's raw balance already reflects the spend. Any positive balance delta
            // is a genuine incoming deposit, not a masked deduction. This fixes the relaunch+deposit
            // scenario where the old "succeeded-only" check left funds stuck until 6 confirmations.
            // Invariant note: ldk-node creates Onchain payment rows strictly from wallet events
            // (TxUnconfirmed/TxConfirmed) diffing the wallet's tx graph, ensuring raw balances
            // already incorporate the spend when PENDING is reached.
            val paymentStatusMap by lazy {
                val map = mutableMapOf<String, PaymentStatus>()
                nodeService.node?.listPayments()?.forEach { p ->
                    val kind = p.kind
                    if (kind is PaymentKind.Onchain) {
                        map[kind.txid] = p.status
                    }
                }
                map
            }
            val incorporatedPredicate: (String) -> Boolean = { tid ->
                val status = paymentStatusMap[tid]
                status == PaymentStatus.SUCCEEDED || status == PaymentStatus.PENDING
            }
            val failedPredicate: (String) -> Boolean = { tid ->
                paymentStatusMap[tid] == PaymentStatus.FAILED
            }
            pendingOutboundSend = resolvePendingOutboundSend(
                rawOnchain = rawOnchain,
                pending = pendingOutboundSend,
                isTxIncorporated = incorporatedPredicate,
                isTxFailed = failedPredicate
            )
            pendingOutboundSend
        }
        val (onchain, spendable) = calculateEffectiveBalances(rawOnchain, rawSpendable, effectivePending)

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
                // Backfill independently of the txid check above: an existing wallet upgrading
                // to this fix already has fundingTxid cached but never had fundingVout, so its
                // txid never "changes" and the vout would otherwise stay null forever, forcing
                // CloseTxidResolver's callers to fall back to an unproven vout=0 guess (#264).
                val liveVout = txo.vout.toInt()
                if (fundingVout != liveVout) {
                    fundingVout = liveVout
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
        // Pending sweep: count PendingBroadcast and BroadcastAwaitingConfirmation
        // These funds are NOT yet in total_onchain_balance_sats
        var sweepSats = 0L
        for (pending in balances.pendingBalancesFromChannelClosures) {
            when (pending) {
                is PendingSweepBalance.PendingBroadcast -> sweepSats += pending.amountSatoshis.toLong()
                is PendingSweepBalance.BroadcastAwaitingConfirmation -> sweepSats += pending.amountSatoshis.toLong()
                else -> {}
            }
        }
        _pendingSweepBalanceSats.value = sweepSats

        _lightningBalanceSats.value = lightning
        _onchainBalanceSats.value = onchain
        _hasReadyChannel.value = hasReady
        _spendableOnchainSats.value = spendable


        // Clear closing flag once lightning balance fully resolves, or if a new channel is opened
        // Don't clear pendingClosePaymentId here — let detectOnchainDeposit()
        // handle it when the on-chain funds arrive
        if (isChannelClosing && lightning == 0L) {
            isChannelClosing = false
        }

        val hasAnyChannel = nodeService.channels.isNotEmpty()
        _totalBalanceSats.value = calculateTotalBalance(
            lightning = lightning,
            onchain = onchain,
            hasReady = hasReady,
            isChannelClosing = isChannelClosing,
            isSweeping = isSweeping,
            pendingSweep = sweepSats,
            isOpeningChannel = isOpeningChannel,
            hasAnyChannel = hasAnyChannel
        )

        // Calculate native sats (lightning minus stable portion) for slider position
        // On-chain funds excluded — they're not in the channel yet
        val sc = _stableChannel.value
        val btcPrice = priceService.currentPrice.value
        val stableSats = if (btcPrice > 0) (sc.expectedUSD.amount / btcPrice * Constants.SATS_IN_BTC).toLong() else 0L
        val native = (lightning - stableSats).coerceAtLeast(0L)
        _nativeSats.value = native

        // Cache for instant display on next launch
        val editor = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE).edit()
            .putLong(BalanceCacheKey.LIGHTNING, lightning)
            .putLong(BalanceCacheKey.ONCHAIN, onchain)
            .putLong(BalanceCacheKey.SPENDABLE, spendable)
            .putLong(BalanceCacheKey.NATIVE, native)
        persistPendingOutboundSend(editor, pendingOutboundSend)
        editor.apply()
    }

    fun onchainSendBroadcasted(amountSats: Long, isSendAll: Boolean, txid: String? = null) {
        val currentOnchain = _onchainBalanceSats.value
        val currentSpendable = _spendableOnchainSats.value

        val newOnchain = if (isSendAll) 0L else (currentOnchain - amountSats).coerceAtLeast(0L)
        val newSpendable = if (isSendAll) 0L else (currentSpendable - amountSats).coerceAtLeast(0L)

        val gen = synchronized(pendingLock) {
            pendingOutboundSend = recordBroadcast(
                currentPending = pendingOutboundSend,
                amountSats = amountSats,
                isSendAll = isSendAll,
                currentOnchain = currentOnchain,
                txid = txid
            )
            ++sendGeneration
        }

        _onchainBalanceSats.value = newOnchain
        _spendableOnchainSats.value = newSpendable

        val lightning = _lightningBalanceSats.value
        val hasReady = _hasReadyChannel.value
        val hasAnyChannel = nodeService.channels.isNotEmpty()
        _totalBalanceSats.value = calculateTotalBalance(
            lightning = lightning,
            onchain = newOnchain,
            hasReady = hasReady,
            isChannelClosing = isChannelClosing,
            isSweeping = isSweeping,
            pendingSweep = _pendingSweepBalanceSats.value,
            isOpeningChannel = isOpeningChannel,
            hasAnyChannel = hasAnyChannel
        )

        val editor = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE).edit()
            .putLong(BalanceCacheKey.ONCHAIN, newOnchain)
            .putLong(BalanceCacheKey.SPENDABLE, newSpendable)
        persistPendingOutboundSend(editor, pendingOutboundSend)
        if (!hasReady && !hasAnyChannel) {
            editor.putLong(BalanceCacheKey.LIGHTNING, 0L)
        }
        editor.apply()

        viewModelScope.launch(Dispatchers.IO) {
            var syncSuccess = false
            for (attempt in 0..2) {
                try {
                    nodeService.syncWallets()
                    syncSuccess = true
                    break
                } catch (_: Exception) {
                    delay(500L * (attempt + 1))
                }
            }
            withContext(Dispatchers.Main) {
                if (shouldClearPendingOnSyncCompletion(gen, sendGeneration, syncSuccess)) {
                    synchronized(pendingLock) {
                        if (gen == sendGeneration) {
                            pendingOutboundSend = PendingOutboundSend()
                        }
                    }
                }
                refreshBalances()
            }
        }
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
        cacheBalanceForLaunch()
    }

    /** Cache in SharedPreferences so the UI has correct state on next launch, before the
     *  database is open. Must be called any time _stableChannel's expectedUSD changes and is
     *  considered durable — including paths that update the DB directly (e.g.
     *  reconcileOutgoingBacking()) without going through saveChannelToDB(). */
    private fun cacheBalanceForLaunch() {
        val sc = _stableChannel.value
        context.getSharedPreferences("balance_cache", Context.MODE_PRIVATE).edit()
            .putString("cached_channel_id", sc.channelId)
            .putString("cached_user_channel_id", sc.userChannelId)
            .putFloat("cached_expected_usd", sc.expectedUSD.amount.toFloat())
            .apply()
    }

    /** Called when the UI returns to the foreground. Reloads channel state from the DB so
     *  backing increments committed by StabilityProcessingService while this process was
     *  cached are picked up before any save can clobber them. Cheap and safe to call repeatedly. */
    /**
     * Tell the user about an order that was refused while they were away.
     *
     * A rejection delivered while the app is backgrounded is verified and committed by the
     * event handler, but the only place it is ever shown is the trade sheet's result step and a
     * status message set in that same moment — both live in process memory. The relaunch that
     * follows is a cold start, so both are gone and the refusal is silent: the balance simply
     * never moved. Resurface the most recent failure once, in the status capsule, so a rejection
     * is never lost just because the app was not in the foreground when it arrived.
     *
     * Once per outcome (a seen-marker keyed on its payment id) and only while the capsule is
     * free, so it can never displace a live message or reappear on every launch.
     */
    private fun surfaceUnseenTradeFailure() {
        val db = databaseService ?: return
        val failure = try {
            db.mostRecentTradeFailure(Constants.TRADE_FAILURE_RESURFACE_WINDOW_SECS)
        } catch (e: Exception) {
            Log.w("AppState", "Could not read the last trade failure: ${e.message}")
            null
        } ?: return
        val prefs = context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE)
        val lastShown = prefs.getString(BalanceCacheKey.LAST_SHOWN_TRADE_FAILURE, null)
        // Mark it seen only once it is actually on screen. start() is re-invocable (ErrorView's
        // retry button), and by then the capsule may hold a live message — recording the failure
        // as shown there would swallow it for good, since the marker is keyed on the payment id.
        if (!TradeFailureNotice.shouldShow(failure.paymentId, lastShown, _statusMessage.value.isNotEmpty())) return
        _statusMessage.value = failure.outcome.message
        markTradeFailureSeen(failure.paymentId)
        AuditService.log("TRADE_FAILURE_RESURFACED", mapOf(
            "payment_id" to failure.paymentId,
            "resolved_at" to failure.resolvedAt
        ))
    }

    private fun markTradeFailureSeen(paymentId: String) {
        context.getSharedPreferences(BalanceCacheKey.PREFS_NAME, Context.MODE_PRIVATE)
            .edit()
            .putString(BalanceCacheKey.LAST_SHOWN_TRADE_FAILURE, paymentId)
            .apply()
    }

    fun onForegroundResume() {
        loadChannelFromDB()
    }

    /**
     * Heal books that claim more backing than the channel holds.
     *
     * backing > the live receiver balance cannot happen in normal operation: the backing is a
     * slice of that balance. It means a withdrawal moved sats out without its stable-books
     * deduction — the #311 splice race, which stranded wallets that cannot recover any other way
     * (the payment row is already 'completed', so no confirmation or resume path revisits it, and
     * on Android the LSP's corrective sync never applies). Deduct the excess once, at the current
     * accounting price, and pin backing to the live balance.
     *
     * Cold start only, and only with nothing in flight: an in-flight HTLC lowers the receiver
     * balance for as long as it is pending and would read as an overflow.
     */
    private fun repairBooksAboveLiveBalance() {
        val db = databaseService ?: return
        val sc = _stableChannel.value
        if (sc.userChannelId.isEmpty()) return
        val receiverSats = sc.stableReceiverBTC.sats
        // A zero balance is a legitimate repair case (a full splice-out closes the position), but
        // it is only meaningful against a live channel — without one the figure is not
        // authoritative and there is nothing to reconcile against.
        if (receiverSats < 0L || !_hasReadyChannel.value) return
        if (isChannelClosing || isSweeping || pendingSplice != null) return
        if (try { db.hasPendingSplice() } catch (_: Exception) { true }) return
        // A stability send whose backing debit has not been recorded yet looks exactly like an
        // overflow. clampBackingToLiveReceiver() re-checks this inside its transaction; this is
        // the cheap early out.
        if (try { db.loadPendingSend() != null } catch (_: Exception) { true }) return
        val inFlight = try {
            nodeService.node?.listPayments()?.any { it.status == PaymentStatus.PENDING } ?: true
        } catch (e: Exception) {
            true
        }
        if (inFlight) return
        val price = priceService.currentAccountingPrice()
        if (price <= 0.0) return
        val result = try {
            synchronized(booksLock) {
                val clamped = db.clampBackingToLiveReceiver(sc.userChannelId, receiverSats, price)
                if (clamped != null) {
                    publishBooksFromDB(recomputeNative = true)
                    cacheBalanceForLaunch()
                }
                clamped
            }
        } catch (e: Exception) {
            Log.w("AppState", "Books repair failed: ${e.message}")
            AuditService.log("BOOKS_REPAIR_FAILED", mapOf("error" to (e.message ?: "")))
            return
        } ?: return
        AuditService.log("BOOKS_REPAIRED_ABOVE_LIVE_BALANCE", mapOf(
            "user_channel_id" to sc.userChannelId,
            "overflow_sats" to result.overflowSats,
            "usd_deducted" to result.usdDeducted,
            "old_expected_usd" to result.oldExpectedUSD,
            "new_expected_usd" to result.newExpectedUSD,
            "backing_sats" to result.newBackingSats,
            "btc_price" to price
        ))
    }

    /** Republish expectedUSD/backingSats from the channel row — the single source of truth for
     *  the stable books — never from an earlier in-memory snapshot or a transaction's return
     *  value. Must be called inside synchronized(booksLock), immediately after the transaction
     *  that changed the row, so no other path can commit-and-publish in between.
     *  [recomputeNative] is only safe when the in-memory receiver balance is already live. */
    private fun publishBooksFromDB(lastStabilityPayment: Long? = null, recomputeNative: Boolean = false) {
        val ucid = _stableChannel.value.userChannelId
        if (ucid.isEmpty()) return
        val record = databaseService?.loadChannel(ucid) ?: return
        _stableChannel.update {
            it.copy(
                expectedUSD = USD(record.expectedUSD),
                backingSats = record.backingSats,
                lastStabilityPayment = lastStabilityPayment ?: it.lastStabilityPayment
            ).also { c -> if (recomputeNative) StabilityService.recomputeNative(c) }
        }
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
        if (!isBackfillingHourly.compareAndSet(false, true)) return
        try {
            val thirtyDaysAgo = System.currentTimeMillis() / 1000 - 30 * 24 * 3600
            val oldest = db.getOldestPriceHistoryTimestamp()
            val since = if (oldest != null && oldest < thirtyDaysAgo) {
                db.getLatestPriceHistoryTimestamp() ?: thirtyDaysAgo
            } else {
                thirtyDaysAgo
            }
            for (attempt in 1..3) {
                val candles = priceChartService.fetchKrakenHourlyOHLC(since)
                if (candles == null) {
                    if (attempt < 3) kotlinx.coroutines.delay(attempt * 1000L)
                    continue
                }
                if (candles.isNotEmpty()) {
                    val count = db.backfillHourlyPrices(candles)
                    if (count > 0) {
                        AuditService.log("CHART_BACKFILL", mapOf("points" to count))
                        cachedChartHourly = db.getPriceHistory(24 * 30)
                        _chartUpdateTrigger.value = System.currentTimeMillis()
                    }
                }
                break
            }
        } finally {
            isBackfillingHourly.set(false)
        }
    }

    private suspend fun backfillDailyPrices() {
        val db = databaseService ?: return
        if (!isBackfillingDaily.compareAndSet(false, true)) return
        try {
            val sevenTwentyDaysAgo = System.currentTimeMillis() / 1000 - 720 * 24 * 3600
            val latest = db.getLatestDailyPriceDate()
            val fmt = java.text.SimpleDateFormat("yyyy-MM-dd", java.util.Locale.US).apply {
                timeZone = java.util.TimeZone.getTimeZone("UTC")
            }
            val since = if (latest != null) {
                val date = try { fmt.parse(latest) } catch (_: Exception) { null }
                if (date != null) maxOf(date.time / 1000 - 86400, sevenTwentyDaysAgo) else sevenTwentyDaysAgo
            } else {
                sevenTwentyDaysAgo
            }
            for (attempt in 1..3) {
                val candles = priceChartService.fetchKrakenDailyOHLC(since)
                if (candles == null) {
                    if (attempt < 3) kotlinx.coroutines.delay(attempt * 1000L)
                    continue
                }
                if (candles.isNotEmpty()) {
                    val count = db.backfillDailyPrices(candles)
                    if (count > 0) {
                        AuditService.log("CHART_DAILY_BACKFILL", mapOf("points" to count))
                    }
                    val dailyPrices = db.getDailyPrices(99999)
                    val daily = dailyPrices.mapNotNull { d ->
                        val date = try { fmt.parse(d.date) } catch (_: Exception) { null } ?: return@mapNotNull null
                        val ts = date.time / 1000
                        com.stablechannels.app.models.PriceRecord(id = ts, price = d.close, source = "daily", timestamp = ts)
                    }.sortedBy { it.timestamp }
                    cachedChartDaily = daily
                    _chartUpdateTrigger.value = System.currentTimeMillis()
                }
                break
            }
        } finally {
            isBackfillingDaily.set(false)
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
