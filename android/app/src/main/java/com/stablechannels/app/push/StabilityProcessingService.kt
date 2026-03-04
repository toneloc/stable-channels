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
        val seedFile = File(dataDir, "keys_seed")
        if (!seedFile.exists()) {
            Log.w(TAG, "No seed file, skipping")
            return
        }

        // Build lightweight LDK node (no RGS, no LSPS2)
        val config = Config()
        config.storageDirPath = dataDir.absolutePath
        config.network = Network.BITCOIN
        config.trustedPeers0conf = listOf(Constants.DEFAULT_LSP_PUBKEY)

        val anchorConfig = AnchorChannelsConfig()
        anchorConfig.trustedPeersNoReserve = listOf(Constants.DEFAULT_LSP_PUBKEY)
        anchorConfig.perChannelReserveSats = 25_000UL
        config.anchorChannelsConfig = anchorConfig

        val builder = Builder.fromConfig(config)
        builder.setEsploraServer(Constants.DEFAULT_CHAIN_URL)
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

            when (direction) {
                "lsp_to_user" -> handleLspToUser(node)
                "user_to_lsp" -> handleUserToLsp(node)
                else -> Log.w(TAG, "Unknown direction: $direction")
            }
        } finally {
            node.stop()
        }
    }

    private fun handleLspToUser(node: Node) {
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

    private fun handleUserToLsp(node: Node) {
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
        val stableReceiverSats = channelState.receiverSats
        val latestPrice = channelState.latestPrice

        if (expectedUsd < 0.01) {
            Log.d(TAG, "No stable position, skipping")
            return
        }

        // Calculate stability check
        val stableUsdValue = if (stableReceiverSats > 0) {
            (stableReceiverSats.toDouble() / Constants.SATS_IN_BTC) * price
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
            val tlv = CustomTlvRecord(Constants.STABLE_CHANNEL_TLV_TYPE.toULong(), byteArrayOf().toList())
            node.spontaneousPayment().sendWithCustomTlvs(
                amountMsat.toULong(),
                Constants.DEFAULT_LSP_PUBKEY,
                listOf(tlv),
                null
            )
            Log.d(TAG, "Stability keysend sent successfully")
        } catch (e: Exception) {
            Log.e(TAG, "Stability keysend failed", e)
            throw e
        }
    }

    private data class ChannelState(
        val expectedUsd: Double,
        val receiverSats: Long,
        val latestPrice: Double
    )

    private fun loadChannelStateFromDB(): ChannelState? {
        val dbFile = File(Constants.userDataDir(this), "stablechannels.db")
        if (!dbFile.exists()) return null

        return try {
            val db = SQLiteDatabase.openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
            val cursor = db.rawQuery(
                "SELECT expected_usd, receiver_sats, latest_price FROM channels LIMIT 1",
                null
            )
            val result = cursor.use {
                if (it.moveToFirst()) {
                    ChannelState(
                        expectedUsd = it.getDouble(0),
                        receiverSats = it.getLong(1),
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
