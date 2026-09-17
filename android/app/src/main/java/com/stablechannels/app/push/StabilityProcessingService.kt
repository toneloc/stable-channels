package com.stablechannels.app.push

import android.app.Service
import android.content.Intent
import android.database.sqlite.SQLiteDatabase
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.stablechannels.app.R
import com.stablechannels.app.StableChannelsApp
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.LdkNodeOwner
import com.stablechannels.app.services.LightningPaymentRecovery
import com.stablechannels.app.services.OutgoingStabilityPaymentRecovery
import com.stablechannels.app.services.PaymentFailureRecorder
import com.stablechannels.app.services.SignedSettlementValidation
import com.stablechannels.app.services.SpliceEventRecorder
import com.stablechannels.app.services.StabilityKeysend
import com.stablechannels.app.services.StabilityPaymentProtocol
import com.stablechannels.app.services.TradeControlApplyStatus
import com.stablechannels.app.services.TradeControlMessage
import com.stablechannels.app.services.TradeProtocol
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.LspPreferencesManager
import com.stablechannels.app.util.NamedPrice
import com.stablechannels.app.util.PriceFeedConfig
import com.stablechannels.app.util.PriceOracle
import com.stablechannels.app.util.PriceOracleAnchorStore
import com.stablechannels.app.util.PriceOracleException
import com.stablechannels.app.util.StabilityFreshness
import java.io.File
import java.util.concurrent.TimeUnit
import kotlin.math.abs
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import org.lightningdevkit.ldknode.*

class StabilityProcessingService : Service() {

    private enum class InsertResult {
        INSERTED,
        DUPLICATE,
        MISSING_CHANNEL,
        FAILED,
    }

    /**
     * Thrown when a stability payment DB write fails permanently; the polling catch re-throws this
     * so it escapes handleLspToUser and reaches onStartCommand's flagPendingPayment path.
     */
    private class BackingUpdateFailed(msg: String) : Exception(msg)

    private class NodeOwnerBusy(msg: String) : Exception(msg)

    /** Persist failed fee sends before acknowledging the LDK event. */
    private fun persistPaymentFailure(node: Node, event: Event.PaymentFailed) {
        val paymentId = event.paymentId ?: return
        val db = DatabaseService(this)
        try {
            PaymentFailureRecorder.record(db, paymentId, event.reason?.name) {
                node
                    .payment(paymentId)
                    ?.takeIf { payment ->
                        payment.kind is PaymentKind.Spontaneous &&
                            payment.direction == PaymentDirection.OUTBOUND
                    }
                    ?.amountMsat
                    ?.toLong()
            }
            AuditService.log(
                "PAYMENT_FAILED",
                mapOf(
                    "payment_id" to paymentId,
                    "reason" to (event.reason?.name ?: "unknown"),
                    "source" to "background",
                ),
            )
        } catch (e: Exception) {
            throw BackingUpdateFailed("Cannot persist failed payment: ${e.message}")
        } finally {
            db.close()
        }
    }

    /** Persist successful outbound payments before acknowledging the LDK event. */
    private fun persistPaymentSuccess(node: Node, event: Event.PaymentSuccessful) {
        val db = DatabaseService(this)
        try {
            // The event proves a stability marker with this id, even if LDK lost its record.
            if (
                !event.paymentId.isNullOrEmpty() &&
                    db.loadPendingSend()?.paymentId == event.paymentId
            )
                OutgoingStabilityPaymentRecovery.reconcile(
                    db,
                    node,
                    channelsAuthoritative = true,
                    succeededPaymentId = event.paymentId,
                )
            LightningPaymentRecovery.recordSuccess(db, event.paymentId, event.feePaidMsat?.toLong())
            AuditService.log(
                "PAYMENT_SUCCESSFUL",
                mapOf(
                    "payment_id" to event.paymentId,
                    "fee_msat" to (event.feePaidMsat?.toLong() ?: 0L),
                    "source" to "background",
                ),
            )
        } finally {
            db.close()
        }
    }

    /** Both background loops commit splice details before removing the event from LDK. */
    internal fun persistSpliceEvent(
        event: Event,
        fundingForReady: (Event.ChannelReady) -> OutPoint?,
        acknowledge: () -> Unit,
    ) {
        try {
            DatabaseService(this).use { db ->
                check(SpliceEventRecorder.record(db, event, fundingForReady))
            }
        } catch (e: Exception) {
            throw BackingUpdateFailed("Cannot persist splice event: ${e.message}")
        }
        acknowledge()
    }

    /**
     * A splice failure needs the foreground's esplora-verified bookkeeping; only ack it when
     * nothing is pending.
     */
    internal fun deferSpliceFailure(acknowledge: () -> Unit) {
        val pending =
            try {
                DatabaseService(this).use { it.hasPendingSplice() }
            } catch (e: Exception) {
                throw BackingUpdateFailed("Cannot check for a pending splice: ${e.message}")
            }
        if (pending)
            throw BackingUpdateFailed(
                "Splice failure needs foreground bookkeeping — leaving for foreground"
            )
        acknowledge()
    }

    companion object {
        private const val TAG = "StabilityBgService"
        private const val POLL_TIMEOUT_SECS = 25
        private const val DB_RETRY_BACKOFF_MS = 500L
        private const val SYNC_FRESHNESS_POLL_MS = 500L

        @Volatile
        var isRunning = false
            private set
    }

    /**
     * Whether this run's gossip strip deleted node_metrics — resetting LDK's persisted
     * Lightning-sync timestamp, so the freshness gate can't inherit the app's last sync.
     */
    private var nodeMetricsReset = false

    private val httpClient =
        OkHttpClient.Builder()
            .connectTimeout(Constants.PRICE_FETCH_TIMEOUT_SECS, TimeUnit.SECONDS)
            .readTimeout(Constants.PRICE_FETCH_TIMEOUT_SECS, TimeUnit.SECONDS)
            .callTimeout(Constants.PRICE_FETCH_TIMEOUT_SECS, TimeUnit.SECONDS)
            .build()

    private data class InboundClassification(
        val isStability: Boolean,
        val settlementId: String?,
        /// Local channel state was unreadable, which says nothing about the peer's envelope.
        val stateUnavailable: Boolean = false,
    )

    /**
     * A payment is a stability settlement only with a valid signed STABILITY_PAYMENT_V1 record on
     * TLV 13377333 — the legacy [0x01] marker is gone (#270), so anything without a signed record
     * is ordinary Lightning. An invalid signed record must not credit backing either, so it also
     * falls through as Lightning (mirrors desktop user.rs). Unreadable local channel state is NOT
     * an invalid envelope: it is reported separately so the caller can leave the event unacked
     * instead of demoting a real settlement.
     */
    private fun classifyInboundPayment(
        node: Node,
        records: List<CustomTlvRecord>,
        amountMsat: Long,
    ): InboundClassification {
        val signedRecord =
            records.firstOrNull {
                it.typeNum == Constants.SIGNED_STABILITY_TLV_TYPE.toULong()
            } ?: return InboundClassification(false, null)
        val channelId = loadChannelStateFromDB()?.channelId.orEmpty()
        if (channelId.isEmpty()) {
            // Retryable local condition: recording now would dedupe the payment id and make
            // the backing credit unrecoverable once the channel row is readable again.
            Log.w(TAG, "Channel state unavailable for signed stability record — deferring")
            AuditService.log(
                "STABILITY_PAYMENT_STATE_UNAVAILABLE",
                mapOf("amount_msat" to amountMsat),
            )
            return InboundClassification(false, null, stateUnavailable = true)
        }
        return when (
            val validation =
                StabilityPaymentProtocol.validateInbound(
                    signedRecord.value,
                    LspPreferencesManager.getLspPubkey(this),
                    channelId,
                    amountMsat,
                ) { msg, sig, pk ->
                    node.verifySignature(msg.map { it.toUByte() }, sig, pk)
                }
        ) {
            is SignedSettlementValidation.Valid ->
                InboundClassification(true, validation.payment.settlementId)
            is SignedSettlementValidation.Invalid -> {
                Log.w(
                    TAG,
                    "Invalid signed stability record (${validation.reason}) — recording as lightning",
                )
                AuditService.log(
                    "STABILITY_PAYMENT_INVALID",
                    mapOf(
                        "amount_msat" to amountMsat,
                        "reason" to validation.reason,
                    ),
                )
                InboundClassification(false, null)
            }
        }
    }

    private fun hasStableControlMessage(records: List<CustomTlvRecord>): Boolean = records.any {
        it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() &&
            !it.value.contentEquals(byteArrayOf(1))
    }

    private fun handleStableControlMessage(
        node: Node,
        records: List<CustomTlvRecord>,
        amountMsat: Long,
    ): Boolean {
        val tlv =
            records.firstOrNull {
                it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() &&
                    !it.value.contentEquals(byteArrayOf(1))
            } ?: return false

        if (amountMsat != TradeProtocol.RESULT_CONTROL_AMOUNT_MSAT) return true
        val message =
            TradeProtocol.parseSignedControl(
                tlv.value,
                LspPreferencesManager.getLspPubkey(this),
            ) { msg, sig, pk ->
                node.verifySignature(msg.map { it.toUByte() }, sig, pk)
            } ?: return true
        val db = DatabaseService(this)
        return try {
            val result =
                when (message) {
                    is TradeControlMessage.Rejected -> db.applyTradeRejection(message)
                    is TradeControlMessage.Sync ->
                        if (message.correlation != null) {
                            db.applyCorrelatedTradeAcceptance(message)
                        } else {
                            val price = fetchMedianPrice()
                            if (price <= 0.0) {
                                throw BackingUpdateFailed(
                                    "Cannot apply SYNC_V1 without a BTC price"
                                )
                            }
                            db.applyUncorrelatedSyncIfNewer(message, price)
                        }
                }
            when (result.status) {
                TradeControlApplyStatus.APPLIED,
                TradeControlApplyStatus.DUPLICATE,
                TradeControlApplyStatus.INVALID -> true
                TradeControlApplyStatus.RETRY -> {
                    try {
                        db.markTradeResponseNotCommittable(message)
                    } catch (_: Exception) {}
                    throw BackingUpdateFailed("Signed trade result could not be committed")
                }
            }
        } finally {
            db.close()
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val direction = intent?.getStringExtra("direction") ?: "lsp_to_user"

        val notification =
            NotificationCompat.Builder(this, StableChannelsApp.STABILITY_CHANNEL_ID)
                .setContentTitle("Stability")
                .setContentText("Processing stability payment...")
                .setSmallIcon(R.mipmap.ic_launcher)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .build()
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            startForeground(
                StableChannelsApp.STABILITY_NOTIFICATION_ID,
                notification,
                android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
            )
        } else {
            startForeground(StableChannelsApp.STABILITY_NOTIFICATION_ID, notification)
        }

        isRunning = true

        Thread {
            try {
                processStability(direction)
                FCMService.clearPendingPayment(this)
            } catch (e: NodeOwnerBusy) {
                Log.d(TAG, "Deferring stability processing: ${e.message}")
                FCMService.flagPendingPayment(this)
            } catch (e: Exception) {
                Log.e(TAG, "Stability processing failed", e)
                FCMService.flagPendingPayment(this)
            } finally {
                isRunning = false
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
            .start()

        return START_NOT_STICKY
    }

    private fun processStability(direction: String) {
        Log.d(TAG, "Processing stability: direction=$direction")

        val dataDir = Constants.userDataDir(this)
        // AuditService is a process-wide singleton with no path until set, so audit calls from
        // this service would silently no-op without it.
        AuditService.setLogPath(File(dataDir, "audit_log.txt").absolutePath)
        val keySeedFile = File(dataDir, "keys_seed")
        val seedPhraseFile = File(dataDir, "seed_phrase")
        if (!keySeedFile.exists() && !seedPhraseFile.exists()) {
            Log.w(TAG, "No seed file (checked keys_seed and seed_phrase), skipping")
            return
        }

        if (!LdkNodeOwner.tryAcquire(LdkNodeOwner.STABILITY_SERVICE)) {
            throw NodeOwnerBusy(
                "LDK node data is already owned by ${LdkNodeOwner.currentOwner() ?: "another owner"}"
            )
        }

        var node: Node? = null
        try {
            // Strip gossip from SQLite to avoid OOM in the foreground service.
            // The service doesn't need gossip (it only routes to the LSP, a direct peer).
            nodeMetricsReset = stripGossipFromDB(dataDir)

            val lspPubkey = LspPreferencesManager.getLspPubkey(this)
            val lspAddress = LspPreferencesManager.getLspAddress(this)

            // Build lightweight LDK node (no RGS, no LSPS2)
            val anchorConfig =
                AnchorChannelsConfig(
                    trustedPeersNoReserve = listOf(lspPubkey),
                    perChannelReserveSats = 25_000UL,
                )

            val config =
                Config(
                    storageDirPath = dataDir.absolutePath,
                    network = Network.BITCOIN,
                    listeningAddresses = null,
                    announcementAddresses = null,
                    nodeAlias = null,
                    trustedPeers0conf = listOf(lspPubkey),
                    probingLiquidityLimitMultiplier = 3UL,
                    anchorChannelsConfig = anchorConfig,
                    routeParameters = null,
                    torConfig = null,
                    hrnConfig =
                        HumanReadableNamesConfig(
                            HrnResolverConfig.Dns(
                                dnsServerAddress = "8.8.8.8:53",
                                enableHrnResolutionService = false,
                            )
                        ),
                )

            val builder = Builder.fromConfig(config)
            builder.setChainSourceEsplora(Constants.PRIMARY_CHAIN_URL, null)

            // Derive node entropy (entropy is now passed to build()): prefer the seed_phrase
            // mnemonic if present, otherwise fall back to the existing keys_seed file.
            val seedWords = if (seedPhraseFile.exists()) seedPhraseFile.readText().trim() else ""
            val nodeEntropy =
                if (seedWords.isNotEmpty()) {
                    Log.d(TAG, "Using seed_phrase mnemonic")
                    NodeEntropy.fromBip39Mnemonic(seedWords, null)
                } else {
                    NodeEntropy.fromSeedPath(keySeedFile.absolutePath)
                }
            // No RGS gossip (saves ~5s startup + ~8MB RAM)
            // No LSPS2 (not needed for stability payments)

            val startedNode = builder.build(nodeEntropy)
            node = startedNode
            startedNode.start()

            // Connect to LSP
            try {
                startedNode.connect(lspPubkey, lspAddress, true)
            } catch (e: Exception) {
                Log.w(TAG, "LSP connect: ${e.message}")
            }

            val dbPath = File(dataDir, "stablechannels.db").absolutePath

            when (direction) {
                "lsp_to_user" -> handleLspToUser(startedNode, dbPath)
                "user_to_lsp" -> handleUserToLsp(startedNode, dbPath)
                "incoming_payment" -> handleIncomingPayment(startedNode, dbPath)
                else -> Log.w(TAG, "Unknown direction: $direction")
            }
        } finally {
            try {
                node?.stop()
            } finally {
                LdkNodeOwner.release(LdkNodeOwner.STABILITY_SERVICE)
            }
        }
    }

    private fun handleLspToUser(node: Node, dbPath: String) {
        // Price dropped — LSP sends us sats. Just poll for incoming payment.
        Log.d(TAG, "Polling for incoming payment...")
        val deadline = System.currentTimeMillis() + POLL_TIMEOUT_SECS * 1000L
        var hasUnpersistedEvent = false
        var price = 0.0

        while (System.currentTimeMillis() < deadline) {
            try {
                val event =
                    try {
                        runBlocking { withTimeout(1000L) { node.nextEventAsync() } }
                    } catch (e: TimeoutCancellationException) {
                        continue
                    }
                when (event) {
                    is Event.PaymentReceived -> {
                        Log.d(TAG, "Payment received: ${event.amountMsat} msat")
                        if (hasStableControlMessage(event.customRecords)) {
                            if (
                                handleStableControlMessage(
                                    node,
                                    event.customRecords,
                                    event.amountMsat.toLong(),
                                )
                            ) {
                                node.eventHandled()
                                hasUnpersistedEvent = false
                                continue
                            }
                            throw BackingUpdateFailed(
                                "Unrecognized stable-control message — leaving for foreground"
                            )
                        }
                        if (event.amountMsat.toLong() < 1000L) {
                            node.eventHandled()
                            Log.d(TAG, "Ignored sub-sat incoming event")
                            continue
                        }
                        val inbound =
                            classifyInboundPayment(
                                node,
                                event.customRecords,
                                event.amountMsat.toLong(),
                            )
                        if (inbound.stateUnavailable) {
                            throw BackingUpdateFailed(
                                "Channel state unavailable for signed settlement — not acknowledging, foreground will heal"
                            )
                        }
                        val isStabilityPayment = inbound.isStability
                        val paymentId = event.paymentId ?: event.paymentHash
                        if (price <= 0) price = fetchMedianPrice()
                        if (isStabilityPayment) {
                            val amountSats = event.amountMsat.toLong() / 1000
                            val result =
                                recordPaymentAtomicInDB(
                                    dbPath,
                                    paymentId,
                                    "stability",
                                    "received",
                                    event.amountMsat.toLong(),
                                    price,
                                    amountSats,
                                    userChannelId = activeUserChannelId(),
                                    settlementId = inbound.settlementId,
                                )
                            when (result) {
                                InsertResult.INSERTED,
                                InsertResult.DUPLICATE -> {
                                    node.eventHandled()
                                    if (result == InsertResult.INSERTED) {
                                        Log.d(TAG, "Updated backingSats += $amountSats (delta)")
                                    }
                                }
                                InsertResult.MISSING_CHANNEL ->
                                    throw BackingUpdateFailed(
                                        "No channel row for stability payment — not acknowledging, foreground will heal"
                                    )
                                InsertResult.FAILED ->
                                    throw BackingUpdateFailed(
                                        "DB write failed for stability payment — not acknowledging, LDK will retry"
                                    )
                            }
                            return
                        } else {
                            Log.d(
                                TAG,
                                "Non-stability payment received, recording as lightning and continuing to poll",
                            )
                            val result =
                                recordPaymentAtomicInDB(
                                    dbPath,
                                    paymentId,
                                    "lightning",
                                    "received",
                                    event.amountMsat.toLong(),
                                    price,
                                    null,
                                )
                            when (result) {
                                InsertResult.INSERTED,
                                InsertResult.DUPLICATE -> {
                                    node.eventHandled()
                                    hasUnpersistedEvent = false
                                }
                                InsertResult.MISSING_CHANNEL,
                                InsertResult.FAILED -> {
                                    hasUnpersistedEvent = true
                                    Log.e(
                                        TAG,
                                        "DB write failed for non-stability payment — backing off before retry",
                                    )
                                    Thread.sleep(DB_RETRY_BACKOFF_MS)
                                }
                            }
                        }
                    }
                    is Event.PaymentFailed -> {
                        persistPaymentFailure(node, event)
                        node.eventHandled()
                    }
                    is Event.PaymentSuccessful -> {
                        persistPaymentSuccess(node, event)
                        node.eventHandled()
                    }
                    is Event.SpliceNegotiated,
                    is Event.ChannelReady -> {
                        persistSpliceEvent(
                            event,
                            { ready ->
                                node
                                    .listChannels()
                                    .singleOrNull {
                                        it.userChannelId == ready.userChannelId &&
                                            it.channelId == ready.channelId
                                    }
                                    ?.fundingTxo
                            },
                        ) {
                            node.eventHandled()
                        }
                    }
                    is Event.SpliceNegotiationFailed -> deferSpliceFailure { node.eventHandled() }
                    else -> node.eventHandled()
                }
            } catch (e: Exception) {
                if (e is BackingUpdateFailed)
                    throw e // permanent; let onStartCommand flag for retry
                Thread.sleep(500)
            }
        }
        if (hasUnpersistedEvent) {
            throw BackingUpdateFailed(
                "DB write still failing for non-stability payment — leaving pending for foreground retry"
            )
        }
        Log.d(TAG, "Poll timeout — no payment received")
    }

    /**
     * Insert a payment and optionally update channel backing sats in one SQLite transaction. BEGIN
     * IMMEDIATE is used so the write lock is held before the dedup SELECT, preventing TOCTOU races
     * across processes.
     */
    private fun recordPaymentAtomicInDB(
        dbPath: String,
        paymentId: String?,
        paymentType: String,
        direction: String,
        amountMsat: Long,
        btcPrice: Double,
        backingDeltaSats: Long?,
        userChannelId: String? = null,
        settlementId: String? = null,
    ): InsertResult {
        return try {
            val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)
            ensureSettlementTable(db)
            // BEGIN IMMEDIATE acquires the write lock before the dedup SELECT.
            db.execSQL("BEGIN IMMEDIATE")
            try {
                if (!paymentId.isNullOrEmpty()) {
                    val cursor =
                        db.rawQuery(
                            "SELECT id FROM payments WHERE payment_id = ?",
                            arrayOf(paymentId),
                        )
                    val exists = cursor.use { it.moveToFirst() }
                    if (exists) {
                        Log.d(TAG, "recordPaymentAtomicInDB: already exists, skipping")
                        db.execSQL("ROLLBACK")
                        db.close()
                        return InsertResult.DUPLICATE
                    }
                }
                if (settlementId != null) {
                    // Replay guard: an already-applied settlement id never credits backing again.
                    val cursor =
                        db.rawQuery(
                            "SELECT settlement_id FROM stability_settlements WHERE settlement_id = ?",
                            arrayOf(settlementId),
                        )
                    val seen = cursor.use { it.moveToFirst() }
                    if (seen) {
                        Log.d(
                            TAG,
                            "recordPaymentAtomicInDB: settlement $settlementId already applied, skipping",
                        )
                        db.execSQL("ROLLBACK")
                        db.close()
                        return InsertResult.DUPLICATE
                    }
                }
                val amountUsd =
                    if (btcPrice > 0)
                        (amountMsat.toDouble() / 1000.0 / Constants.SATS_IN_BTC) * btcPrice
                    else 0.0
                db.execSQL(
                    "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status) VALUES (?, ?, ?, ?, ?, ?, 'completed')",
                    arrayOf<Any?>(
                        paymentId,
                        paymentType,
                        direction,
                        amountMsat,
                        amountUsd,
                        btcPrice,
                    ),
                )
                if (settlementId != null) {
                    db.execSQL(
                        "INSERT INTO stability_settlements (settlement_id) VALUES (?)",
                        arrayOf(settlementId),
                    )
                }
                if (backingDeltaSats != null) {
                    // Target the backing UPDATE by the explicit user_channel_id — never by recency
                    // —
                    // so a push-triggered payment can't credit/debit the wrong channel row.
                    if (userChannelId.isNullOrEmpty()) {
                        throw Exception(
                            "Backing delta requested without user_channel_id — rolling back"
                        )
                    }
                    val backingCursor =
                        db.rawQuery(
                            "SELECT stable_sats FROM channels WHERE user_channel_id = ?",
                            arrayOf(userChannelId),
                        )
                    val currentBacking = backingCursor.use {
                        if (it.moveToFirst()) it.getLong(0) else null
                    }
                    if (currentBacking == null) {
                        Log.e(
                            TAG,
                            "recordPaymentAtomicInDB: no channel row for user_channel_id=$userChannelId — rolling back",
                        )
                        db.execSQL("ROLLBACK")
                        db.close()
                        return InsertResult.MISSING_CHANNEL
                    }
                    // Clamp instead of refusing: this runs after the payment already settled, so
                    // the sats truly moved — a floor of 0 keeps the ledger recordable.
                    val newBacking = maxOf(0L, currentBacking + backingDeltaSats)
                    if (currentBacking + backingDeltaSats < 0) {
                        Log.w(
                            TAG,
                            "BACKING_CLAMPED: current=$currentBacking delta=$backingDeltaSats clamped_to=$newBacking user_channel_id=$userChannelId",
                        )
                    }
                    val updateStmt =
                        db.compileStatement(
                            "UPDATE channels SET stable_sats = ?, updated_at = strftime('%s','now') WHERE user_channel_id = ?"
                        )
                    updateStmt.bindLong(1, newBacking)
                    updateStmt.bindString(2, userChannelId)
                    val rowsAffected = updateStmt.executeUpdateDelete()
                    if (rowsAffected != 1) {
                        throw Exception(
                            "Backing UPDATE affected $rowsAffected rows, expected 1 — rolling back"
                        )
                    }
                }
                db.execSQL("COMMIT")
                db.close()
                Log.d(TAG, "recordPaymentAtomicInDB: saved $direction $amountMsat msat")
                InsertResult.INSERTED
            } catch (e: Exception) {
                try {
                    db.execSQL("ROLLBACK")
                } catch (_: Exception) {}
                db.close()
                throw e
            }
        } catch (e: Exception) {
            Log.e(TAG, "recordPaymentAtomicInDB failed", e)
            InsertResult.FAILED
        }
    }

    private fun handleIncomingPayment(node: Node, dbPath: String) {
        // Wake push — stay online for POLL_TIMEOUT_SECS to receive any pending payments.
        Log.d(TAG, "Polling for incoming payments (wake push)...")
        val deadline = System.currentTimeMillis() + POLL_TIMEOUT_SECS * 1000L
        var received = false
        var price = 0.0
        var hasUnpersistedEvent = false

        while (System.currentTimeMillis() < deadline) {
            try {
                val event =
                    try {
                        runBlocking { withTimeout(1000L) { node.nextEventAsync() } }
                    } catch (e: TimeoutCancellationException) {
                        continue
                    }
                when (event) {
                    is Event.PaymentReceived -> {
                        Log.d(TAG, "Payment received: ${event.amountMsat} msat")
                        if (hasStableControlMessage(event.customRecords)) {
                            if (
                                handleStableControlMessage(
                                    node,
                                    event.customRecords,
                                    event.amountMsat.toLong(),
                                )
                            ) {
                                node.eventHandled()
                                received = true
                                hasUnpersistedEvent = false
                                continue
                            }
                            throw BackingUpdateFailed(
                                "Unrecognized stable-control message — leaving for foreground"
                            )
                        }
                        if (event.amountMsat.toLong() < 1000L) {
                            node.eventHandled()
                            Log.d(TAG, "Ignored sub-sat incoming event")
                            continue
                        }
                        if (price <= 0) price = fetchMedianPrice()
                        val pid = event.paymentId ?: event.paymentHash
                        // Classify by TLV like handleLspToUser — a stability payment must credit
                        // backing, not be misfiled as a plain lightning receive.
                        val inbound =
                            classifyInboundPayment(
                                node,
                                event.customRecords,
                                event.amountMsat.toLong(),
                            )
                        if (inbound.stateUnavailable) {
                            throw BackingUpdateFailed(
                                "Channel state unavailable for signed settlement — not acknowledging, foreground will heal"
                            )
                        }
                        val isStabilityPayment = inbound.isStability
                        val result =
                            if (isStabilityPayment) {
                                val amountSats = event.amountMsat.toLong() / 1000
                                recordPaymentAtomicInDB(
                                    dbPath,
                                    pid,
                                    "stability",
                                    "received",
                                    event.amountMsat.toLong(),
                                    price,
                                    amountSats,
                                    userChannelId = activeUserChannelId(),
                                    settlementId = inbound.settlementId,
                                )
                            } else {
                                recordPaymentAtomicInDB(
                                    dbPath,
                                    pid,
                                    "lightning",
                                    "received",
                                    event.amountMsat.toLong(),
                                    price,
                                    null,
                                )
                            }
                        when (result) {
                            InsertResult.INSERTED,
                            InsertResult.DUPLICATE -> {
                                node.eventHandled()
                                received = true
                                hasUnpersistedEvent = false
                            }
                            InsertResult.MISSING_CHANNEL ->
                                throw BackingUpdateFailed(
                                    "No channel row for stability payment — not acknowledging, foreground will heal"
                                )
                            InsertResult.FAILED -> {
                                if (isStabilityPayment) {
                                    throw BackingUpdateFailed(
                                        "DB write failed for stability payment — not acknowledging, LDK will retry"
                                    )
                                }
                                hasUnpersistedEvent = true
                                Log.e(
                                    TAG,
                                    "DB write failed for incoming payment — backing off before retry",
                                )
                                Thread.sleep(DB_RETRY_BACKOFF_MS)
                            }
                        }
                        // Keep polling — there might be more payments
                    }
                    is Event.PaymentFailed -> {
                        persistPaymentFailure(node, event)
                        node.eventHandled()
                    }
                    is Event.PaymentSuccessful -> {
                        persistPaymentSuccess(node, event)
                        node.eventHandled()
                    }
                    is Event.SpliceNegotiated,
                    is Event.ChannelReady -> {
                        persistSpliceEvent(
                            event,
                            { ready ->
                                node
                                    .listChannels()
                                    .singleOrNull {
                                        it.userChannelId == ready.userChannelId &&
                                            it.channelId == ready.channelId
                                    }
                                    ?.fundingTxo
                            },
                        ) {
                            node.eventHandled()
                        }
                    }
                    is Event.SpliceNegotiationFailed -> deferSpliceFailure { node.eventHandled() }
                    else -> node.eventHandled()
                }
            } catch (e: Exception) {
                if (e is BackingUpdateFailed)
                    throw e // permanent; let onStartCommand flag for retry
                Thread.sleep(500)
            }
        }
        if (hasUnpersistedEvent) {
            throw BackingUpdateFailed(
                "DB write still failing for incoming payment — leaving pending for foreground retry"
            )
        }
        if (received) {
            Log.d(TAG, "Incoming payment(s) received during wake")
        } else {
            Log.d(TAG, "No incoming payments during wake poll")
        }
    }

    private fun handleUserToLsp(node: Node, dbPath: String) {
        if (!reconcilePendingOutgoingPayment(node, dbPath)) {
            throw BackingUpdateFailed(
                "Previous outgoing payment marker is unresolved — refusing to send again"
            )
        }
        val accountingDb = DatabaseService(this)
        try {
            if (
                accountingDb.hasPendingChannelSend() ||
                    node.listPayments().any {
                        it.direction == PaymentDirection.OUTBOUND &&
                            it.status == PaymentStatus.PENDING &&
                            it.kind !is PaymentKind.Onchain
                    }
            ) {
                throw BackingUpdateFailed(
                    "Outgoing payment accounting is pending — deferring stability settlement"
                )
            }
        } finally {
            accountingDb.close()
        }

        // Cooldown: skip if we sent a stability payment recently
        val prefs = FCMService.getPrefs(this)
        val lastSent = prefs.getLong("bg_last_stability_sent", 0)
        val now = System.currentTimeMillis() / 1000
        if (lastSent > 0 && (now - lastSent) < 120) {
            Log.d(TAG, "Cooldown: ${now - lastSent}s since last payment, skipping (120s required)")
            return
        }

        // Price rose — user owes LSP sats. Read channel state and send keysend.
        val channelState =
            loadChannelStateFromDB()
                ?: run {
                    Log.w(TAG, "No channel state in DB")
                    return
                }

        val price = fetchMedianPrice()
        if (price <= 0) {
            Log.w(TAG, "Could not fetch BTC price")
            return
        }

        val expectedUsd = channelState.expectedUsd

        if (expectedUsd < 0.01 && channelState.backingSats == 0L) {
            Log.d(TAG, "No stable position, skipping")
            return
        }

        // Use backingSats from DB directly — set at trade time, reset after payments
        val backingSats = channelState.backingSats

        Log.d(TAG, "Channel state: expectedUSD=$expectedUsd, backingSats=$backingSats")

        // Calculate stability check using backing_sats from DB
        val stableUsdValue =
            if (backingSats > 0) {
                (backingSats.toDouble() / Constants.SATS_IN_BTC) * price
            } else {
                0.0
            }

        val dollarsFromPar = stableUsdValue - expectedUsd
        val percentFromPar = abs(dollarsFromPar / maxOf(expectedUsd, 0.01)) * 100.0

        if (
            percentFromPar < Constants.STABILITY_THRESHOLD_PERCENT ||
                abs(dollarsFromPar) < Constants.STABILITY_THRESHOLD_USD
        ) {
            Log.d(TAG, "Within threshold (${percentFromPar}%), skipping")
            return
        }

        if (dollarsFromPar <= 0) {
            Log.d(TAG, "Price went down, not up — nothing to pay")
            return
        }

        // Stable allocations are sat-denominated — floor to whole sats so the signed amount
        // matches the keysend exactly (mirrors src/stable.rs).
        val amountMsat =
            Math.floor(dollarsFromPar / price * Constants.SATS_IN_BTC * 1000).toLong() / 1000L *
                1000L
        if (amountMsat <= 0) return

        Log.d(TAG, "Sending stability payment: $amountMsat msat ($$dollarsFromPar)")

        // Chain-freshness gate (see #243): never keysend on a stale chain tip — an outbound
        // HTLC built on an old best block understates its expiry, and LDK later force-closes
        // on it. If stale, wait for LDK's background sync within this service's existing
        // work budget. Checked BEFORE the claim so a deferral never leaves a
        // claimed-but-unsent marker that would block the foreground retry.
        val initialSyncAge = lightningSyncAgeSecs(node)
        logStabilityGateEvent("stability_background_attempted", initialSyncAge, waitedMs = 0)
        var waitedMs = 0L
        if (!lightningSyncIsFresh(node)) {
            val waitStart = System.currentTimeMillis()
            val waitDeadline = waitStart + POLL_TIMEOUT_SECS * 1000L
            while (System.currentTimeMillis() < waitDeadline && !lightningSyncIsFresh(node)) {
                Thread.sleep(SYNC_FRESHNESS_POLL_MS)
            }
            waitedMs = System.currentTimeMillis() - waitStart
            if (!lightningSyncIsFresh(node)) {
                logStabilityGateEvent(
                    "stability_background_deferred_stale_sync",
                    initialSyncAge,
                    waitedMs,
                )
                Log.w(
                    TAG,
                    "Lightning sync still stale after ${waitedMs}ms — deferring to foreground",
                )
                FCMService.flagPendingPayment(this)
                return
            }
            logStabilityGateEvent("stability_background_fresh_after_wait", initialSyncAge, waitedMs)
        } else {
            logStabilityGateEvent("stability_background_fresh_ready", initialSyncAge, waitedMs)
        }

        // Atomically claim the send before starting it. If another process (foreground timer)
        // already holds the marker, the claim is denied and we skip this tick — this is the
        // check-and-set that prevents a double send.
        if (!claimPendingSendInDB(amountMsat, price, channelState.userChannelId)) {
            Log.d(TAG, "Pending send already claimed by another sender — skipping this tick")
            return
        }

        // Re-check after the claim: the SQLite claim can take up to ~2s under cross-process
        // contention and could carry the timestamp past the 120s boundary. No send happened,
        // so clear the claim rather than blocking the foreground retry.
        if (!lightningSyncIsFresh(node)) {
            try {
                clearPendingSendInDB()
            } catch (_: Exception) {}
            logStabilityGateEvent(
                "stability_background_deferred_stale_sync",
                initialSyncAge,
                waitedMs,
            )
            Log.w(TAG, "Lightning sync went stale during claim — deferring to foreground")
            FCMService.flagPendingPayment(this)
            return
        }

        // Attach only the signed STABILITY_PAYMENT_V1 envelope bound to this exact
        // amount and channel — the legacy [0x01] marker is gone (#270). If the
        // envelope can't be built, release the claim and skip the payment entirely.
        val signedEnvelope =
            try {
                StabilityPaymentProtocol.buildSignedEnvelope(
                    channelId = channelState.channelId,
                    amountMsat = amountMsat,
                    expectedUsd = channelState.expectedUsd,
                    sign = { payload -> node.signMessage(payload.map { it.toUByte() }) },
                )
            } catch (e: Exception) {
                try {
                    clearPendingSendInDB()
                } catch (_: Exception) {}
                throw e
            }
        if (signedEnvelope == null) {
            try {
                clearPendingSendInDB()
            } catch (_: Exception) {}
            Log.w(TAG, "Could not build signed stability envelope — skipping payment")
            return
        }
        val records =
            listOf(
                CustomTlvRecord(
                    Constants.SIGNED_STABILITY_TLV_TYPE.toULong(),
                    signedEnvelope.toByteArray(Charsets.UTF_8),
                )
            )
        // The claim carries the payment id before LDK sends, so no outcome can orphan it.
        val outcome =
            DatabaseService(this).use { db ->
                StabilityKeysend.send(db) { preimage ->
                    node
                        .spontaneousPayment()
                        .sendWithPreimageAndCustomTlvs(
                            amountMsat.toULong(),
                            LspPreferencesManager.getLspPubkey(this),
                            records,
                            preimage,
                            null,
                        )
                }
            }
        when (outcome) {
            is StabilityKeysend.Outcome.NotSent -> {
                // Nothing left the node, so there is no successful payment to protect.
                try {
                    clearPendingSendInDB()
                } catch (_: Exception) {}
                Log.e(TAG, "Stability keysend failed", outcome.error)
                throw outcome.error
            }
            is StabilityKeysend.Outcome.OutcomeUnknown ->
                throw BackingUpdateFailed(
                    "Stability payment may have been sent; its claim stays until the outcome is known"
                )
            is StabilityKeysend.Outcome.Sent -> Unit
        }
        // Only an accepted send counts as sent_* — denied-claim and send-failure runs must
        // not inflate the pilot's send numbers.
        logStabilityGateEvent(
            if (waitedMs > 0) "stability_background_sent_after_sync"
            else "stability_background_sent_fresh",
            initialSyncAge,
            waitedMs,
        )
        Log.d(TAG, "Stability keysend sent successfully")

        val sentAt = System.currentTimeMillis() / 1000
        FCMService.getPrefs(this).edit().putLong("bg_last_stability_sent", sentAt).commit()

        // Do not release backing merely because LDK accepted the send. The durable marker
        // survives service shutdown and lets the next wake/foreground event finish settlement.
        val settlementDeadline = System.currentTimeMillis() + POLL_TIMEOUT_SECS * 1000L
        // One connection for the whole wait: every open re-runs table creation and pruning.
        DatabaseService(this).use { db ->
            check(db.writableDatabase.path == dbPath)
            while (
                !OutgoingStabilityPaymentRecovery.reconcile(db, node, channelsAuthoritative = true)
            ) {
                if (System.currentTimeMillis() >= settlementDeadline) {
                    // Throw so onStartCommand preserves the retry flag instead of clearing it.
                    throw BackingUpdateFailed(
                        "Stability payment is still pending; retaining its backing and retry marker"
                    )
                }
                Thread.sleep(250)
            }
        }
    }

    private fun lightningSyncAgeSecs(node: Node): Long? =
        StabilityFreshness.syncAgeSecs(
            node.status().latestLightningWalletSyncTimestamp?.toLong(),
            System.currentTimeMillis() / 1000,
        )

    private fun lightningSyncIsFresh(node: Node): Boolean =
        StabilityFreshness.isFresh(
            node.status().latestLightningWalletSyncTimestamp?.toLong(),
            System.currentTimeMillis() / 1000,
        )

    /**
     * Structured pilot metric for the background user_to_lsp freshness gate. No wallet secrets or
     * payment identifiers — platform, sync age, wait time, and strip state only.
     */
    private fun logStabilityGateEvent(event: String, prevSyncAgeSecs: Long?, waitedMs: Long) {
        val json =
            JSONObject()
                .put("event", event)
                .put("platform", "android")
                .put("prev_sync_age_secs", prevSyncAgeSecs ?: JSONObject.NULL)
                .put("waited_ms", waitedMs)
                .put("node_metrics_reset", nodeMetricsReset)
        Log.i(TAG, "STABILITY_GATE $json")
    }

    /**
     * Resolve any leftover pending-send marker. Returns true when no unresolved marker blocks a new
     * send; false means wait (a send may still be in flight).
     */
    private fun reconcilePendingOutgoingPayment(node: Node, dbPath: String): Boolean =
        DatabaseService(this).use { db ->
            check(db.writableDatabase.path == dbPath)
            OutgoingStabilityPaymentRecovery.reconcile(db, node, channelsAuthoritative = true)
        }

    /**
     * Applied inbound STABILITY_PAYMENT_V1 settlement ids (replay guard). Same schema as
     * DatabaseService.createStabilitySettlementsTable — IF NOT EXISTS for either process.
     */
    private fun ensureSettlementTable(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE IF NOT EXISTS stability_settlements (
                settlement_id TEXT PRIMARY KEY,
                created_at INTEGER DEFAULT (strftime('%s','now'))
            )
        """
        )
    }

    // Use the same schema, origin requirement and transactions as the foreground process.
    private fun claimPendingSendInDB(
        amountMsat: Long,
        price: Double,
        userChannelId: String,
    ): Boolean = DatabaseService(this).use { it.claimPendingSend(amountMsat, price, userChannelId) }

    private fun clearPendingSendInDB() = DatabaseService(this).use { it.clearPendingSend() }

    private data class ChannelState(
        val expectedUsd: Double,
        val receiverSats: Long,
        val nativeSats: Long,
        val backingSats: Long,
        val latestPrice: Double,
        val userChannelId: String,
        val channelId: String,
    )

    private fun loadChannelStateFromDB(): ChannelState? {
        val dbFile = File(Constants.userDataDir(this), "stablechannels.db")
        if (!dbFile.exists()) return null

        return try {
            val db =
                SQLiteDatabase.openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
            val cursor =
                db.rawQuery(
                    // Pick the single active channel deterministically. The user_channel_id it
                    // returns is
                    // the stable key every backing UPDATE targets by — the write never re-selects
                    // by recency.
                    "SELECT expected_usd, receiver_sats, latest_price, stable_sats, user_channel_id, channel_id FROM channels WHERE user_channel_id IS NOT NULL AND user_channel_id != '' ORDER BY updated_at DESC, channel_id DESC LIMIT 1",
                    null,
                )
            val result = cursor.use {
                if (it.moveToFirst()) {
                    ChannelState(
                        expectedUsd = it.getDouble(0),
                        receiverSats = it.getLong(1),
                        nativeSats = 0, // not in DB schema, computed at runtime
                        backingSats = it.getLong(3),
                        latestPrice = it.getDouble(2),
                        userChannelId = it.getString(4),
                        channelId = it.getString(5),
                    )
                } else null
            }
            db.close()
            result
        } catch (e: Exception) {
            Log.e(TAG, "Failed to read channel state from DB", e)
            null
        }
    }

    /**
     * Resolve the single active channel's user_channel_id — the stable key backing UPDATEs target.
     * Returns null when no channel row exists, in which case a backing update must fail (not
     * guess).
     */
    private fun activeUserChannelId(): String? =
        loadChannelStateFromDB()?.userChannelId?.takeIf { it.isNotEmpty() }

    private fun fetchMedianPrice(): Double {
        // Anchor to the app's last accepted price so the large-move circuit breaker also
        // protects the unattended path (mirrors the iOS notification extension).
        val lastTrustedPrice = PriceOracleAnchorStore.freshPrice(this)
        val usdPrices = fetchOracleFeeds(PriceOracle.DIRECT_USD_FEEDS)
        val price =
            try {
                PriceOracle.resolve(usdPrices, emptyList(), emptyList(), lastTrustedPrice).price
            } catch (error: PriceOracleException) {
                if (error.quarantinesPrice) {
                    Log.w(TAG, "Rejected direct USD price: ${error.message}")
                    return 0.0
                }
                Log.w(TAG, "Direct USD unavailable: ${error.message}; trying USDT fallback")
                val fallback =
                    fetchOracleFeeds(PriceOracle.BITCOIN_USDT_FEEDS + PriceOracle.USDT_USD_FEEDS)
                val usdtNames = PriceOracle.BITCOIN_USDT_FEEDS.map { it.name }.toSet()
                val pegNames = PriceOracle.USDT_USD_FEEDS.map { it.name }.toSet()
                try {
                    PriceOracle.resolve(
                            emptyList(),
                            fallback.filter { it.feedName in usdtNames },
                            fallback.filter { it.feedName in pegNames },
                            lastTrustedPrice,
                        )
                        .price
                } catch (fallbackError: Exception) {
                    Log.w(TAG, "Rejected USDT fallback: ${fallbackError.message}")
                    return 0.0
                }
            }
        if (price > 0.0) {
            PriceOracleAnchorStore.save(this, price)
        }
        return price
    }

    private fun fetchOracleFeeds(feeds: List<PriceFeedConfig>): List<NamedPrice> {
        val prices = java.util.Collections.synchronizedList(mutableListOf<NamedPrice>())
        val latch = java.util.concurrent.CountDownLatch(feeds.size)

        for (feed in feeds) {
            val request = Request.Builder().url(feed.urlFormat).build()
            httpClient
                .newCall(request)
                .enqueue(
                    object : okhttp3.Callback {
                        override fun onFailure(call: okhttp3.Call, e: java.io.IOException) {
                            Log.w(TAG, "${feed.name} failed: ${e.message}")
                            latch.countDown()
                        }

                        override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) {
                            try {
                                if (!response.isSuccessful) {
                                    Log.w(TAG, "${feed.name} failed: HTTP ${response.code}")
                                    return
                                }
                                val body = response.body?.string() ?: return
                                val json = JSONTokener(body).nextValue()
                                val price = extractPrice(json, feed.jsonPath)
                                if (price != null) {
                                    prices.add(NamedPrice(feed.name, price))
                                    Log.d(TAG, "${feed.name} succeeded")
                                }
                            } catch (error: Exception) {
                                Log.w(TAG, "${feed.name} failed: ${error.message}")
                            } finally {
                                response.close()
                                latch.countDown()
                            }
                        }
                    }
                )
        }

        latch.await(Constants.PRICE_FETCH_TIMEOUT_SECS + 1, TimeUnit.SECONDS)
        return synchronized(prices) { prices.toList() }
    }

    private fun extractPrice(json: Any, path: List<String>): Double? {
        var current: Any = json
        for (key in path) {
            current =
                when (current) {
                    is JSONObject -> current.opt(key) ?: return null
                    is JSONArray -> current.opt(key.toIntOrNull() ?: return null) ?: return null
                    else -> return null
                }
        }
        return when (current) {
            is Double -> current
            is Int -> current.toDouble()
            is Long -> current.toDouble()
            is String -> current.toDoubleOrNull()
            is JSONArray ->
                when (val first = current.opt(0)) {
                    is String -> first.toDoubleOrNull()
                    is Double -> first
                    is Int -> first.toDouble()
                    is Long -> first.toDouble()
                    else -> null
                }
            else -> null
        }
    }

    /**
     * Delete network_graph, scorer, and node_metrics from the LDK SQLite DB. The background service
     * doesn't need gossip (it only routes to the LSP, a direct peer). This reduces the DB from
     * ~10MB to ~30KB, preventing OOM on low-memory devices.
     *
     * Returns true when node_metrics was deleted: that resets LDK's persisted
     * latest_lightning_wallet_sync_timestamp, so the freshness gate must wait for a new sync on
     * this run instead of inheriting the foreground app's recent one.
     */
    private fun stripGossipFromDB(dataDir: File): Boolean {
        val ldkDbPath = File(dataDir, "ldk_node_data.sqlite")
        if (!ldkDbPath.exists()) return false

        try {
            val db =
                SQLiteDatabase.openDatabase(
                    ldkDbPath.absolutePath,
                    null,
                    SQLiteDatabase.OPEN_READWRITE,
                )

            // Check if network_graph exists and is large enough to matter
            val cursor =
                db.rawQuery(
                    "SELECT LENGTH(value) FROM ldk_node_data WHERE key = 'network_graph'",
                    null,
                )
            val graphSize = cursor.use { if (it.moveToFirst()) it.getInt(0) else 0 }

            val stripped = graphSize > 100_000
            if (stripped) {
                db.execSQL("DELETE FROM ldk_node_data WHERE key = 'network_graph'")
                db.execSQL("DELETE FROM ldk_node_data WHERE key = 'scorer'")
                db.execSQL("DELETE FROM ldk_node_data WHERE key = 'node_metrics'")
                Log.d(TAG, "Stripped gossip from LDK DB (saved ${graphSize / 1024}KB)")
            } else {
                Log.d(TAG, "Gossip data small ($graphSize bytes), skipping strip")
            }
            db.close()
            return stripped
        } catch (e: Exception) {
            Log.w(TAG, "Failed to strip gossip from LDK DB: ${e.message}")
            return false
        }
    }
}
