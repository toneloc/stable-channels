package com.stablechannels.app.push

import android.app.Service
import android.content.Intent
import android.database.sqlite.SQLiteDatabase
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.stablechannels.app.R
import com.stablechannels.app.StableChannelsApp
import com.stablechannels.app.services.LdkNodeOwner
import com.stablechannels.app.services.TradeService
import com.stablechannels.app.util.Constants
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONArray
import org.json.JSONObject
import org.lightningdevkit.ldknode.*
import java.io.File
import java.util.concurrent.TimeUnit
import kotlin.math.abs
import kotlin.math.roundToLong
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.TimeoutCancellationException

class StabilityProcessingService : Service() {

    private enum class InsertResult { INSERTED, DUPLICATE, MISSING_CHANNEL, FAILED }

    /** Thrown when a stability payment DB write fails permanently; the polling catch re-throws
     *  this so it escapes handleLspToUser and reaches onStartCommand's flagPendingPayment path. */
    private class BackingUpdateFailed(msg: String) : Exception(msg)
    private class NodeOwnerBusy(msg: String) : Exception(msg)

    companion object {
        private const val TAG = "StabilityBgService"
        private const val POLL_TIMEOUT_SECS = 25
        private const val DB_RETRY_BACKOFF_MS = 500L

        @Volatile
        var isRunning = false
            private set
    }

    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    private fun isStabilityMarker(records: List<CustomTlvRecord>): Boolean =
        records.any {
            it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() &&
                it.value.contentEquals(byteArrayOf(1))
        }

    private fun hasStableControlMessage(records: List<CustomTlvRecord>): Boolean =
        records.any {
            it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() &&
                !it.value.contentEquals(byteArrayOf(1))
        }

    private fun handleStableControlMessage(node: Node, dbPath: String, records: List<CustomTlvRecord>): Boolean {
        val tlv = records.firstOrNull {
            it.typeNum == Constants.STABLE_CHANNEL_TLV_TYPE.toULong() &&
                !it.value.contentEquals(byteArrayOf(1))
        } ?: return false

        val parsed = TradeService.parseIncomingTLV(tlv.value, Constants.DEFAULT_LSP_PUBKEY) { msg, sig, pk ->
            node.verifySignature(msg.map { it.toUByte() }, sig, pk)
        } ?: return false

        val (type, expectedUsd, parsedUserChannelId) = parsed
        if (type != Constants.SYNC_MESSAGE_TYPE) return false

        var price = fetchMedianPrice()
        if (price <= 0.0) price = loadChannelStateFromDB()?.latestPrice ?: 0.0
        if (price <= 0.0) throw BackingUpdateFailed("Cannot apply SYNC_V1 without a BTC price")

        val userChannelId = parsedUserChannelId.takeIf { it.isNotEmpty() }
            ?: activeUserChannelId()
            ?: throw BackingUpdateFailed("Cannot apply SYNC_V1 without a channel row")

        applySyncMessageInDB(dbPath, userChannelId, expectedUsd, price)
        Log.d(TAG, "Applied background SYNC_V1 expected_usd=$expectedUsd")
        return true
    }

    private fun applySyncMessageInDB(dbPath: String, userChannelId: String, expectedUsd: Double, price: Double) {
        val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)
        db.execSQL("BEGIN IMMEDIATE")
        try {
            val backingSats = ((expectedUsd / price) * Constants.SATS_IN_BTC).roundToLong().coerceAtLeast(0L)
            val stmt = db.compileStatement(
                "UPDATE channels SET expected_usd = ?, stable_sats = ?, latest_price = ?, updated_at = strftime('%s','now') WHERE user_channel_id = ?"
            )
            stmt.bindDouble(1, expectedUsd)
            stmt.bindLong(2, backingSats)
            stmt.bindDouble(3, price)
            stmt.bindString(4, userChannelId)
            val rowsAffected = stmt.executeUpdateDelete()
            if (rowsAffected != 1) {
                throw BackingUpdateFailed("SYNC_V1 UPDATE affected $rowsAffected rows, expected 1")
            }
            db.execSQL("COMMIT")
        } catch (e: Exception) {
            try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
            throw e
        } finally {
            db.close()
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val direction = intent?.getStringExtra("direction") ?: "lsp_to_user"

        val notification = NotificationCompat.Builder(this, StableChannelsApp.STABILITY_CHANNEL_ID)
            .setContentTitle("Stability")
            .setContentText("Processing stability payment...")
            .setSmallIcon(R.mipmap.ic_launcher)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
        startForeground(StableChannelsApp.STABILITY_NOTIFICATION_ID, notification)

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
        }.start()

        return START_NOT_STICKY
    }

    private fun processStability(direction: String) {
        Log.d(TAG, "Processing stability: direction=$direction")

        val dataDir = Constants.userDataDir(this)
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
            stripGossipFromDB(dataDir)

            // Build lightweight LDK node (no RGS, no LSPS2)
            val anchorConfig = AnchorChannelsConfig(
                trustedPeersNoReserve = listOf(Constants.DEFAULT_LSP_PUBKEY),
                perChannelReserveSats = 25_000UL
            )

            val config = Config(
                storageDirPath = dataDir.absolutePath,
                network = Network.BITCOIN,
                listeningAddresses = null,
                announcementAddresses = null,
                nodeAlias = null,
                trustedPeers0conf = listOf(Constants.DEFAULT_LSP_PUBKEY),
                probingLiquidityLimitMultiplier = 3UL,
                anchorChannelsConfig = anchorConfig,
                routeParameters = null,
                torConfig = null,
                hrnConfig = HumanReadableNamesConfig(
                    HrnResolverConfig.Dns(
                        dnsServerAddress = "8.8.8.8:53",
                        enableHrnResolutionService = false
                    )
                )
            )

            val builder = Builder.fromConfig(config)
            builder.setChainSourceEsplora(Constants.PRIMARY_CHAIN_URL, null)

            // Derive node entropy (entropy is now passed to build()): prefer the seed_phrase
            // mnemonic if present, otherwise fall back to the existing keys_seed file.
            val seedWords = if (seedPhraseFile.exists()) seedPhraseFile.readText().trim() else ""
            val nodeEntropy = if (seedWords.isNotEmpty()) {
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
                startedNode.connect(Constants.DEFAULT_LSP_PUBKEY, Constants.DEFAULT_LSP_ADDRESS, true)
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
                val event = try {
                    runBlocking { withTimeout(1000L) { node.nextEventAsync() } }
                } catch (e: TimeoutCancellationException) {
                    continue
                }
                when (event) {
                    is Event.PaymentReceived -> {
                        Log.d(TAG, "Payment received: ${event.amountMsat} msat")
                        if (hasStableControlMessage(event.customRecords)) {
                            if (handleStableControlMessage(node, dbPath, event.customRecords)) {
                                node.eventHandled()
                                hasUnpersistedEvent = false
                                continue
                            }
                            throw BackingUpdateFailed("Unrecognized stable-control message — leaving for foreground")
                        }
                        if (event.amountMsat.toLong() < 1000L) {
                            node.eventHandled()
                            Log.d(TAG, "Ignored sub-sat incoming event")
                            continue
                        }
                        val isStabilityPayment = isStabilityMarker(event.customRecords)
                        val paymentId = event.paymentId ?: event.paymentHash
                        if (price <= 0) price = fetchMedianPrice()
                        if (isStabilityPayment) {
                            val amountSats = event.amountMsat.toLong() / 1000
                            val result = recordPaymentAtomicInDB(
                                dbPath, paymentId, "stability", "received",
                                event.amountMsat.toLong(), price, amountSats,
                                userChannelId = activeUserChannelId()
                            )
                            when (result) {
                                InsertResult.INSERTED, InsertResult.DUPLICATE -> {
                                    node.eventHandled()
                                    if (result == InsertResult.INSERTED) {
                                        Log.d(TAG, "Updated backingSats += $amountSats (delta)")
                                    }
                                }
                                InsertResult.MISSING_CHANNEL ->
                                    throw BackingUpdateFailed("No channel row for stability payment — not acknowledging, foreground will heal")
                                InsertResult.FAILED ->
                                    throw BackingUpdateFailed("DB write failed for stability payment — not acknowledging, LDK will retry")
                            }
                            return
                        } else {
                            Log.d(TAG, "Non-stability payment received, recording as lightning and continuing to poll")
                            val result = recordPaymentAtomicInDB(
                                dbPath, paymentId, "lightning", "received",
                                event.amountMsat.toLong(), price, null
                            )
                            when (result) {
                                InsertResult.INSERTED, InsertResult.DUPLICATE -> {
                                    node.eventHandled()
                                    hasUnpersistedEvent = false
                                }
                                InsertResult.MISSING_CHANNEL, InsertResult.FAILED -> {
                                    hasUnpersistedEvent = true
                                    Log.e(TAG, "DB write failed for non-stability payment — backing off before retry")
                                    Thread.sleep(DB_RETRY_BACKOFF_MS)
                                }
                            }
                        }
                    }
                    else -> node.eventHandled()
                }
            } catch (e: Exception) {
                if (e is BackingUpdateFailed) throw e  // permanent; let onStartCommand flag for retry
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

    /** Insert a payment and optionally update channel backing sats in one SQLite transaction.
     *  BEGIN IMMEDIATE is used so the write lock is held before the dedup SELECT,
     *  preventing TOCTOU races across processes. */
    private fun recordPaymentAtomicInDB(
        dbPath: String,
        paymentId: String?,
        paymentType: String,
        direction: String,
        amountMsat: Long,
        btcPrice: Double,
        backingDeltaSats: Long?,
        userChannelId: String? = null
    ): InsertResult {
        return try {
            val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)
            // BEGIN IMMEDIATE acquires the write lock before the dedup SELECT.
            db.execSQL("BEGIN IMMEDIATE")
            try {
                if (!paymentId.isNullOrEmpty()) {
                    val cursor = db.rawQuery("SELECT id FROM payments WHERE payment_id = ?", arrayOf(paymentId))
                    val exists = cursor.use { it.moveToFirst() }
                    if (exists) {
                        Log.d(TAG, "recordPaymentAtomicInDB: already exists, skipping")
                        db.execSQL("ROLLBACK")
                        db.close()
                        return InsertResult.DUPLICATE
                    }
                }
                val amountUsd = if (btcPrice > 0) (amountMsat.toDouble() / 1000.0 / Constants.SATS_IN_BTC) * btcPrice else 0.0
                db.execSQL(
                    "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status) VALUES (?, ?, ?, ?, ?, ?, 'completed')",
                    arrayOf<Any?>(paymentId, paymentType, direction, amountMsat, amountUsd, btcPrice)
                )
                if (backingDeltaSats != null) {
                    // Target the backing UPDATE by the explicit user_channel_id — never by recency —
                    // so a push-triggered payment can't credit/debit the wrong channel row.
                    if (userChannelId.isNullOrEmpty()) {
                        throw Exception("Backing delta requested without user_channel_id — rolling back")
                    }
                    val backingCursor = db.rawQuery(
                        "SELECT stable_sats FROM channels WHERE user_channel_id = ?",
                        arrayOf(userChannelId)
                    )
                    val currentBacking = backingCursor.use { if (it.moveToFirst()) it.getLong(0) else null }
                    if (currentBacking == null) {
                        Log.e(TAG, "recordPaymentAtomicInDB: no channel row for user_channel_id=$userChannelId — rolling back")
                        db.execSQL("ROLLBACK")
                        db.close()
                        return InsertResult.MISSING_CHANNEL
                    }
                    // Clamp instead of refusing: this runs after the payment already settled, so
                    // the sats truly moved — a floor of 0 keeps the ledger recordable.
                    val newBacking = maxOf(0L, currentBacking + backingDeltaSats)
                    if (currentBacking + backingDeltaSats < 0) {
                        Log.w(TAG, "BACKING_CLAMPED: current=$currentBacking delta=$backingDeltaSats clamped_to=$newBacking user_channel_id=$userChannelId")
                    }
                    val updateStmt = db.compileStatement(
                        "UPDATE channels SET stable_sats = ?, updated_at = strftime('%s','now') WHERE user_channel_id = ?"
                    )
                    updateStmt.bindLong(1, newBacking)
                    updateStmt.bindString(2, userChannelId)
                    val rowsAffected = updateStmt.executeUpdateDelete()
                    if (rowsAffected != 1) {
                        throw Exception("Backing UPDATE affected $rowsAffected rows, expected 1 — rolling back")
                    }
                }
                db.execSQL("COMMIT")
                db.close()
                Log.d(TAG, "recordPaymentAtomicInDB: saved $direction $amountMsat msat")
                InsertResult.INSERTED
            } catch (e: Exception) {
                try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
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
                val event = try {
                    runBlocking { withTimeout(1000L) { node.nextEventAsync() } }
                } catch (e: TimeoutCancellationException) {
                    continue
                }
                when (event) {
                    is Event.PaymentReceived -> {
                        Log.d(TAG, "Payment received: ${event.amountMsat} msat")
                        if (hasStableControlMessage(event.customRecords)) {
                            if (handleStableControlMessage(node, dbPath, event.customRecords)) {
                                node.eventHandled()
                                received = true
                                hasUnpersistedEvent = false
                                continue
                            }
                            throw BackingUpdateFailed("Unrecognized stable-control message — leaving for foreground")
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
                        val isStabilityPayment = isStabilityMarker(event.customRecords)
                        val result = if (isStabilityPayment) {
                            val amountSats = event.amountMsat.toLong() / 1000
                            recordPaymentAtomicInDB(
                                dbPath, pid, "stability", "received",
                                event.amountMsat.toLong(), price, amountSats,
                                userChannelId = activeUserChannelId()
                            )
                        } else {
                            recordPaymentAtomicInDB(dbPath, pid, "lightning", "received", event.amountMsat.toLong(), price, null)
                        }
                        when (result) {
                            InsertResult.INSERTED, InsertResult.DUPLICATE -> {
                                node.eventHandled()
                                received = true
                                hasUnpersistedEvent = false
                            }
                            InsertResult.MISSING_CHANNEL ->
                                throw BackingUpdateFailed("No channel row for stability payment — not acknowledging, foreground will heal")
                            InsertResult.FAILED -> {
                                if (isStabilityPayment) {
                                    throw BackingUpdateFailed("DB write failed for stability payment — not acknowledging, LDK will retry")
                                }
                                hasUnpersistedEvent = true
                                Log.e(TAG, "DB write failed for incoming payment — backing off before retry")
                                Thread.sleep(DB_RETRY_BACKOFF_MS)
                            }
                        }
                        // Keep polling — there might be more payments
                    }
                    else -> node.eventHandled()
                }
            } catch (e: Exception) {
                if (e is BackingUpdateFailed) throw e  // permanent; let onStartCommand flag for retry
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

        // Cooldown: skip if we sent a stability payment recently
        val prefs = FCMService.getPrefs(this)
        val lastSent = prefs.getLong("bg_last_stability_sent", 0)
        val now = System.currentTimeMillis() / 1000
        if (lastSent > 0 && (now - lastSent) < 120) {
            Log.d(TAG, "Cooldown: ${now - lastSent}s since last payment, skipping (120s required)")
            return
        }

        // Price rose — user owes LSP sats. Read channel state and send keysend.
        val channelState = loadChannelStateFromDB() ?: run {
            Log.w(TAG, "No channel state in DB")
            return
        }

        val price = fetchMedianPrice()
        if (price <= 0) {
            Log.w(TAG, "Could not fetch BTC price")
            return
        }

        val expectedUsd = channelState.expectedUsd

        if (expectedUsd < 0.01) {
            Log.d(TAG, "No stable position, skipping")
            return
        }

        // Use backingSats from DB directly — set at trade time, reset after payments
        val backingSats = channelState.backingSats

        Log.d(TAG, "Channel state: expectedUSD=$expectedUsd, backingSats=$backingSats")

        // Calculate stability check using backing_sats from DB
        val stableUsdValue = if (backingSats > 0) {
            (backingSats.toDouble() / Constants.SATS_IN_BTC) * price
        } else {
            0.0
        }

        val dollarsFromPar = stableUsdValue - expectedUsd
        val percentFromPar = if (expectedUsd > 0) abs(dollarsFromPar / expectedUsd) * 100.0 else 0.0

        if (percentFromPar < Constants.STABILITY_THRESHOLD_PERCENT
            || abs(dollarsFromPar) < Constants.STABILITY_THRESHOLD_USD) {
            Log.d(TAG, "Within threshold (${percentFromPar}%), skipping")
            return
        }

        if (dollarsFromPar <= 0) {
            Log.d(TAG, "Price went down, not up — nothing to pay")
            return
        }

        val amountMsat = Math.floor(dollarsFromPar / price * Constants.SATS_IN_BTC * 1000).toLong()
        if (amountMsat <= 0) return

        Log.d(TAG, "Sending stability payment: $amountMsat msat ($$dollarsFromPar)")

        // Atomically claim the send before starting it. If another process (foreground timer)
        // already holds the marker, the claim is denied and we skip this tick — this is the
        // check-and-set that prevents a double send.
        if (!claimPendingSendInDB(dbPath, amountMsat, price)) {
            Log.d(TAG, "Pending send already claimed by another sender — skipping this tick")
            return
        }

        val paymentIdString: String
        try {
            val tlv = CustomTlvRecord(Constants.STABLE_CHANNEL_TLV_TYPE.toULong(), byteArrayOf(1))
            val paymentId = node.spontaneousPayment().sendWithCustomTlvs(
                amountMsat.toULong(),
                Constants.DEFAULT_LSP_PUBKEY,
                null,
                listOf(tlv)
            )
            paymentIdString = paymentId.toString()
        } catch (e: Exception) {
            // sendWithCustomTlvs failed, so there is no successful payment to protect.
            try { clearPendingSendInDB(dbPath) } catch (_: Exception) {}
            Log.e(TAG, "Stability keysend failed", e)
            throw e
        }
        Log.d(TAG, "Stability keysend sent successfully")

        try {
            setPendingSendPaymentIdInDB(dbPath, paymentIdString)
        } catch (e: Exception) {
            throw BackingUpdateFailed(
                "Payment was sent but its ID could not be persisted; marker remains unresolved — reconcile will adopt it"
            )
        }
        val sentAt = System.currentTimeMillis() / 1000
        FCMService.getPrefs(this).edit().putLong("bg_last_stability_sent", sentAt).commit()

        val amountSats = amountMsat / 1000
        val result = recordPaymentAtomicInDB(
            dbPath, paymentIdString, "stability", "sent",
            amountMsat, price, -amountSats,
            userChannelId = channelState.userChannelId
        )
        if (result == InsertResult.FAILED || result == InsertResult.MISSING_CHANNEL) {
            throw BackingUpdateFailed(
                "Payment was sent but DB persistence failed; durable marker will drive reconciliation"
            )
        }
        clearPendingSendInDB(dbPath)
        Log.d(TAG, "Recorded outgoing payment and updated backingSats -= $amountSats atomically")
    }

    /** Resolve any leftover pending-send marker. Returns true when no unresolved marker
     *  blocks a new send; false means wait (a send may still be in flight). */
    private fun reconcilePendingOutgoingPayment(node: Node, dbPath: String): Boolean {
        val pending = loadPendingSendFromDB(dbPath) ?: return true
        var pendingPaymentId = pending.paymentId

        if (pendingPaymentId.isEmpty()) {
            // The previous sender died before persisting the payment ID. Resolve the outcome
            // against LDK's payment store instead of blocking forever.
            val now = System.currentTimeMillis() / 1000
            val candidates = try {
                node.listPayments()
            } catch (e: Exception) {
                Log.w(TAG, "listPayments failed during reconcile: ${e.message}")
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
                    setPendingSendPaymentIdInDB(dbPath, succeeded.id)
                    pendingPaymentId = succeeded.id
                    Log.d(TAG, "Reconcile: adopted succeeded keysend ${succeeded.id} for empty marker")
                }
                stillPending != null -> return false  // in flight — wait
                failed != null -> {
                    clearPendingSendInDB(dbPath)
                    Log.w(TAG, "Reconcile: marker's keysend ${failed.id} failed — cleared marker, no debit")
                    return true
                }
                now - pending.createdAt > 120 -> {
                    clearPendingSendInDB(dbPath)
                    Log.w(TAG, "Reconcile: no matching keysend after ${now - pending.createdAt}s — send never left device, cleared marker")
                    return true
                }
                else -> return false  // young marker — another process may be mid-send
            }
        }

        if (pending.amountMsat <= 0) {
            clearPendingSendInDB(dbPath)
            Log.w(TAG, "Reconcile: corrupt marker (amount_msat=${pending.amountMsat}) — cleared")
            return true
        }

        val amountSats = pending.amountMsat / 1000
        val result = recordPaymentAtomicInDB(
            dbPath, pendingPaymentId, "stability", "sent",
            pending.amountMsat, pending.price, -amountSats,
            userChannelId = activeUserChannelId()
        )
        if (result == InsertResult.FAILED || result == InsertResult.MISSING_CHANNEL) {
            Log.e(TAG, "Could not reconcile previously sent payment — will retry later")
            return false
        }
        clearPendingSendInDB(dbPath)
        Log.d(TAG, "Reconciled previously sent outgoing payment $pendingPaymentId")
        return true
    }

    // --- Pending outgoing stability send marker (raw-SQLite copies of DatabaseService's
    //     pending_stability_send operations, usable without the main app process) ---

    private data class PendingSend(
        val paymentId: String,
        val amountMsat: Long,
        val price: Double,
        val createdAt: Long
    )

    private fun ensurePendingSendTable(db: SQLiteDatabase) {
        // IF NOT EXISTS so either process (main app or this service) can create it.
        db.execSQL("""
            CREATE TABLE IF NOT EXISTS pending_stability_send (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                payment_id TEXT NOT NULL,
                amount_msat INTEGER NOT NULL,
                price REAL NOT NULL,
                created_at INTEGER NOT NULL
            )
        """)
    }

    /** Atomic check-and-set: returns false when a marker already exists (claim denied).
     *  BEGIN IMMEDIATE holds the write lock across the SELECT + INSERT. */
    private fun claimPendingSendInDB(dbPath: String, amountMsat: Long, price: Double): Boolean {
        val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)
        try {
            ensurePendingSendTable(db)
            db.execSQL("BEGIN IMMEDIATE")
            try {
                val cursor = db.rawQuery("SELECT id FROM pending_stability_send WHERE id = 1", null)
                val exists = cursor.use { it.moveToFirst() }
                if (exists) {
                    db.execSQL("ROLLBACK")
                    return false
                }
                db.execSQL(
                    "INSERT INTO pending_stability_send (id, payment_id, amount_msat, price, created_at) VALUES (1, '', ?, ?, ?)",
                    arrayOf<Any?>(amountMsat, price, System.currentTimeMillis() / 1000)
                )
                db.execSQL("COMMIT")
                return true
            } catch (e: Exception) {
                try { db.execSQL("ROLLBACK") } catch (_: Exception) {}
                throw e
            }
        } finally {
            db.close()
        }
    }

    private fun setPendingSendPaymentIdInDB(dbPath: String, paymentId: String) {
        val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)
        try {
            ensurePendingSendTable(db)
            db.execSQL("UPDATE pending_stability_send SET payment_id = ? WHERE id = 1", arrayOf(paymentId))
        } finally {
            db.close()
        }
    }

    private fun loadPendingSendFromDB(dbPath: String): PendingSend? {
        if (!File(dbPath).exists()) return null
        // READWRITE so ensurePendingSendTable can create the table on first touch.
        val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)
        try {
            ensurePendingSendTable(db)
            val cursor = db.rawQuery(
                "SELECT payment_id, amount_msat, price, created_at FROM pending_stability_send WHERE id = 1",
                null
            )
            return cursor.use {
                if (it.moveToFirst()) {
                    PendingSend(
                        paymentId = it.getString(0),
                        amountMsat = it.getLong(1),
                        price = it.getDouble(2),
                        createdAt = it.getLong(3)
                    )
                } else null
            }
        } finally {
            db.close()
        }
    }

    private fun clearPendingSendInDB(dbPath: String) {
        val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)
        try {
            ensurePendingSendTable(db)
            db.execSQL("DELETE FROM pending_stability_send WHERE id = 1")
        } finally {
            db.close()
        }
    }

    private data class ChannelState(
        val expectedUsd: Double,
        val receiverSats: Long,
        val nativeSats: Long,
        val backingSats: Long,
        val latestPrice: Double,
        val userChannelId: String
    )

    private fun loadChannelStateFromDB(): ChannelState? {
        val dbFile = File(Constants.userDataDir(this), "stablechannels.db")
        if (!dbFile.exists()) return null

        return try {
            val db = SQLiteDatabase.openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
            val cursor = db.rawQuery(
                // Pick the single active channel deterministically. The user_channel_id it returns is
                // the stable key every backing UPDATE targets by — the write never re-selects by recency.
                "SELECT expected_usd, receiver_sats, latest_price, stable_sats, user_channel_id FROM channels WHERE user_channel_id IS NOT NULL AND user_channel_id != '' ORDER BY updated_at DESC, channel_id DESC LIMIT 1",
                null
            )
            val result = cursor.use {
                if (it.moveToFirst()) {
                    ChannelState(
                        expectedUsd = it.getDouble(0),
                        receiverSats = it.getLong(1),
                        nativeSats = 0,  // not in DB schema, computed at runtime
                        backingSats = it.getLong(3),
                        latestPrice = it.getDouble(2),
                        userChannelId = it.getString(4)
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

    /** Resolve the single active channel's user_channel_id — the stable key backing UPDATEs target.
     *  Returns null when no channel row exists, in which case a backing update must fail (not guess). */
    private fun activeUserChannelId(): String? =
        loadChannelStateFromDB()?.userChannelId?.takeIf { it.isNotEmpty() }

    private fun fetchMedianPrice(): Double {
        val feeds = listOf(
            "https://www.bitstamp.net/api/v2/ticker/btcusd/" to listOf("last"),
            "https://api.coinbase.com/v2/prices/BTC-USD/spot" to listOf("data", "amount"),
            "https://blockchain.info/ticker" to listOf("USD", "last"),
            "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD" to listOf("result", "XXBTZUSD", "c"),
            "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd" to listOf("bitcoin", "usd")
        )

        val prices = java.util.Collections.synchronizedList(mutableListOf<Double>())
        val latch = java.util.concurrent.CountDownLatch(feeds.size)

        for ((url, path) in feeds) {
            val request = Request.Builder().url(url).build()
            httpClient.newCall(request).enqueue(object : okhttp3.Callback {
                override fun onFailure(call: okhttp3.Call, e: java.io.IOException) {
                    latch.countDown()
                }
                override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) {
                    try {
                        val body = response.body?.string() ?: return
                        val json = JSONObject(body)
                        val price = extractPrice(json, path)
                        if (price != null && price > 0) prices.add(price)
                    } catch (_: Exception) {
                    } finally {
                        latch.countDown()
                    }
                }
            })
        }

        latch.await(8, TimeUnit.SECONDS)

        if (prices.size < 3) return 0.0  // need at least 3 of 5 feeds
        val sorted = prices.sorted()
        val mid = sorted.size / 2
        return if (sorted.size % 2 == 0) {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    }

    private fun extractPrice(json: JSONObject, path: List<String>): Double? {
        var current: Any = json
        for (key in path) {
            current = when (current) {
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
            is JSONArray -> (current.opt(0) as? String)?.toDoubleOrNull()
            else -> null
        }
    }

    /**
     * Delete network_graph, scorer, and node_metrics from the LDK SQLite DB.
     * The background service doesn't need gossip (it only routes to the LSP, a direct peer).
     * This reduces the DB from ~10MB to ~30KB, preventing OOM on low-memory devices.
     */
    private fun stripGossipFromDB(dataDir: File) {
        val ldkDbPath = File(dataDir, "ldk_node_data.sqlite")
        if (!ldkDbPath.exists()) return

        try {
            val db = SQLiteDatabase.openDatabase(ldkDbPath.absolutePath, null, SQLiteDatabase.OPEN_READWRITE)

            // Check if network_graph exists and is large enough to matter
            val cursor = db.rawQuery("SELECT LENGTH(value) FROM ldk_node_data WHERE key = 'network_graph'", null)
            val graphSize = cursor.use { if (it.moveToFirst()) it.getInt(0) else 0 }

            if (graphSize > 100_000) {
                db.execSQL("DELETE FROM ldk_node_data WHERE key = 'network_graph'")
                db.execSQL("DELETE FROM ldk_node_data WHERE key = 'scorer'")
                db.execSQL("DELETE FROM ldk_node_data WHERE key = 'node_metrics'")
                Log.d(TAG, "Stripped gossip from LDK DB (saved ${graphSize / 1024}KB)")
            } else {
                Log.d(TAG, "Gossip data small ($graphSize bytes), skipping strip")
            }
            db.close()
        } catch (e: Exception) {
            Log.w(TAG, "Failed to strip gossip from LDK DB: ${e.message}")
        }
    }
}
