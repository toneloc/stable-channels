package com.stablechannels.app.push

import android.app.Service
import android.content.Intent
import android.database.sqlite.SQLiteDatabase
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.stablechannels.app.R
import com.stablechannels.app.StableChannelsApp
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

class StabilityProcessingService : Service() {

    companion object {
        private const val TAG = "StabilityBgService"
        private const val POLL_TIMEOUT_SECS = 25
        private const val STABILITY_THRESHOLD_PERCENT = 0.1

        @Volatile
        var isRunning = false
            private set
    }

    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

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
            routeParameters = null
        )

        val builder = Builder.fromConfig(config)
        builder.setChainSourceEsplora(Constants.PRIMARY_CHAIN_URL, null)

        // If wallet uses mnemonic (seed_phrase), set it on the builder
        if (seedPhraseFile.exists()) {
            val words = seedPhraseFile.readText().trim()
            if (words.isNotEmpty()) {
                Log.d(TAG, "Using seed_phrase mnemonic")
                builder.setEntropyBip39Mnemonic(words, null)
            }
        }
        // No RGS gossip (saves ~5s startup + ~8MB RAM)
        // No LSPS2 (not needed for stability payments)

        val node = builder.build()
        node.start()

        try {
            // Connect to LSP
            try {
                node.connect(Constants.DEFAULT_LSP_PUBKEY, Constants.DEFAULT_LSP_ADDRESS, true)
            } catch (e: Exception) {
                Log.w(TAG, "LSP connect: ${e.message}")
            }

            val dbPath = File(dataDir, "stablechannels.db").absolutePath

            when (direction) {
                "lsp_to_user" -> handleLspToUser(node, dbPath)
                "user_to_lsp" -> handleUserToLsp(node, dbPath)
                "incoming_payment" -> handleIncomingPayment(node, dbPath)
                else -> Log.w(TAG, "Unknown direction: $direction")
            }
        } finally {
            node.stop()
        }
    }

    private fun handleLspToUser(node: Node, dbPath: String) {
        // Price dropped — LSP sends us sats. Just poll for incoming payment.
        Log.d(TAG, "Polling for incoming payment...")
        val deadline = System.currentTimeMillis() + POLL_TIMEOUT_SECS * 1000L

        while (System.currentTimeMillis() < deadline) {
            try {
                val event = node.nextEvent()
                when (event) {
                    is Event.PaymentReceived -> {
                        Log.d(TAG, "Payment received: ${event.amountMsat} msat")
                        node.eventHandled()
                        val price = fetchMedianPrice()
                        recordPaymentInDB(
                            dbPath, null, "stability", "received",
                            event.amountMsat.toLong(), price
                        )
                        return
                    }
                    else -> node.eventHandled()
                }
            } catch (_: Exception) {
                Thread.sleep(500)
            }
        }
        Log.d(TAG, "Poll timeout — no payment received")
    }

    private fun handleIncomingPayment(node: Node, dbPath: String) {
        // Wake push — stay online for POLL_TIMEOUT_SECS to receive any pending payments.
        Log.d(TAG, "Polling for incoming payments (wake push)...")
        val deadline = System.currentTimeMillis() + POLL_TIMEOUT_SECS * 1000L
        var received = false
        var price = 0.0

        while (System.currentTimeMillis() < deadline) {
            try {
                val event = node.nextEvent()
                when (event) {
                    is Event.PaymentReceived -> {
                        Log.d(TAG, "Payment received: ${event.amountMsat} msat")
                        node.eventHandled()
                        if (price <= 0) price = fetchMedianPrice()
                        recordPaymentInDB(
                            dbPath, null, "lightning", "received",
                            event.amountMsat.toLong(), price
                        )
                        received = true
                        // Keep polling — there might be more payments
                    }
                    else -> node.eventHandled()
                }
            } catch (_: Exception) {
                Thread.sleep(500)
            }
        }
        if (received) {
            Log.d(TAG, "Incoming payment(s) received during wake")
        } else {
            Log.d(TAG, "No incoming payments during wake poll")
        }
    }

    private fun handleUserToLsp(node: Node, dbPath: String) {
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

        if (percentFromPar < STABILITY_THRESHOLD_PERCENT) {
            Log.d(TAG, "Within threshold (${percentFromPar}%), skipping")
            return
        }

        if (dollarsFromPar <= 0) {
            Log.d(TAG, "Price went down, not up — nothing to pay")
            return
        }

        val amountMsat = (dollarsFromPar / price * Constants.SATS_IN_BTC * 1000).roundToLong()
        if (amountMsat <= 0) return

        Log.d(TAG, "Sending stability payment: $amountMsat msat ($$dollarsFromPar)")

        try {
            val tlv = CustomTlvRecord(Constants.STABLE_CHANNEL_TLV_TYPE.toULong(), emptyList())
            node.spontaneousPayment().sendWithCustomTlvs(
                amountMsat.toULong(),
                Constants.DEFAULT_LSP_PUBKEY,
                null,
                listOf(tlv)
            )
            Log.d(TAG, "Stability keysend sent successfully")
            recordPaymentInDB(
                dbPath, null, "stability", "sent",
                amountMsat, price
            )
            // Reset backing_sats to equilibrium after payment
            val newBacking = ((expectedUsd / price) * Constants.SATS_IN_BTC).toLong()
            updateBackingSatsInDB(dbPath, newBacking)
            Log.d(TAG, "Reset backingSats to $newBacking")
        } catch (e: Exception) {
            Log.e(TAG, "Stability keysend failed", e)
            throw e
        }
    }

    private data class ChannelState(
        val expectedUsd: Double,
        val receiverSats: Long,
        val nativeSats: Long,
        val backingSats: Long,
        val latestPrice: Double
    )

    private fun loadChannelStateFromDB(): ChannelState? {
        val dbFile = File(Constants.userDataDir(this), "stablechannels.db")
        if (!dbFile.exists()) return null

        return try {
            val db = SQLiteDatabase.openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
            val cursor = db.rawQuery(
                "SELECT expected_usd, receiver_sats, latest_price, native_sats, stable_sats FROM channels LIMIT 1",
                null
            )
            val result = cursor.use {
                if (it.moveToFirst()) {
                    ChannelState(
                        expectedUsd = it.getDouble(0),
                        receiverSats = it.getLong(1),
                        nativeSats = if (it.columnCount > 3) it.getLong(3) else 0,
                        backingSats = if (it.columnCount > 4) it.getLong(4) else 0,
                        latestPrice = it.getDouble(2)
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

    private fun fetchMedianPrice(): Double {
        val feeds = listOf(
            "https://www.bitstamp.net/api/v2/ticker/btcusd/" to listOf("last"),
            "https://api.coinbase.com/v2/prices/BTC-USD/spot" to listOf("data", "amount"),
            "https://blockchain.info/ticker" to listOf("USD", "last"),
            "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD" to listOf("result", "XXBTZUSD", "c"),
            "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd" to listOf("bitcoin", "usd")
        )

        val prices = mutableListOf<Double>()
        for ((url, path) in feeds) {
            try {
                val request = Request.Builder().url(url).build()
                val response = httpClient.newCall(request).execute()
                val body = response.body?.string() ?: continue
                val json = JSONObject(body)
                val price = extractPrice(json, path)
                if (price != null && price > 0) prices.add(price)
            } catch (_: Exception) {}
        }

        if (prices.isEmpty()) return 0.0
        prices.sort()
        val mid = prices.size / 2
        return if (prices.size % 2 == 0) {
            (prices[mid - 1] + prices[mid]) / 2.0
        } else {
            prices[mid]
        }
    }

    private fun updateBackingSatsInDB(dbPath: String, backingSats: Long) {
        try {
            val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)
            db.execSQL("UPDATE channels SET stable_sats = ? WHERE id = (SELECT MAX(id) FROM channels)", arrayOf(backingSats))
            db.close()
        } catch (e: Exception) {
            Log.e(TAG, "Failed to update backingSats in DB", e)
        }
    }

    private fun recordPaymentInDB(
        dbPath: String,
        paymentId: String?,
        paymentType: String,
        direction: String,
        amountMsat: Long,
        btcPrice: Double
    ) {
        try {
            val db = SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READWRITE)

            // Dedup: skip if payment_id already exists
            if (paymentId != null && paymentId.isNotEmpty()) {
                val cursor = db.rawQuery(
                    "SELECT id FROM payments WHERE payment_id = ?",
                    arrayOf(paymentId)
                )
                val exists = cursor.moveToFirst()
                cursor.close()
                if (exists) {
                    db.close()
                    Log.d(TAG, "recordPayment: already exists, skipping")
                    return
                }
            }

            val amountUsd = if (btcPrice > 0) {
                (amountMsat.toDouble() / 1000.0 / Constants.SATS_IN_BTC) * btcPrice
            } else 0.0

            db.execSQL(
                """INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status)
                   VALUES (?, ?, ?, ?, ?, ?, 'completed')""",
                arrayOf(paymentId, paymentType, direction, amountMsat, amountUsd, btcPrice)
            )
            Log.d(TAG, "recordPayment: saved $direction $amountMsat msat (${"%.2f".format(amountUsd)} USD)")
            db.close()
        } catch (e: Exception) {
            Log.e(TAG, "recordPayment failed", e)
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
}
