package com.stablechannels.app.push

import android.content.Context
import android.content.Intent
import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import com.stablechannels.app.util.Constants
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject

class FCMService : FirebaseMessagingService() {

    companion object {
        private const val TAG = "FCMService"
        private const val PREFS_NAME = "fcm_prefs"
        private const val KEY_FCM_TOKEN = "fcm_token"
        private const val KEY_NODE_ID = "node_id"
        private const val KEY_PENDING_PUSH_PAYMENT = "pending_push_payment"
        private const val KEY_MAIN_APP_LAST_ACTIVE = "main_app_last_active"
        private const val KEY_PENDING_OUTGOING_PAYMENT_ID = "pending_outgoing_stability_payment_id"
        private const val KEY_PENDING_OUTGOING_AMOUNT_MSAT = "pending_outgoing_stability_amount_msat"
        private const val KEY_PENDING_OUTGOING_BTC_PRICE = "pending_outgoing_stability_btc_price"
        private const val HEARTBEAT_THRESHOLD_SECS = 10

        private val httpClient = OkHttpClient()

        data class PendingOutgoingStabilityPayment(
            val paymentId: String,
            val amountMsat: Long,
            val btcPrice: Double
        )

        fun getPrefs(context: Context) =
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

        fun saveToken(context: Context, token: String) {
            getPrefs(context).edit().putString(KEY_FCM_TOKEN, token).apply()
        }

        fun getToken(context: Context): String? =
            getPrefs(context).getString(KEY_FCM_TOKEN, null)

        fun saveNodeId(context: Context, nodeId: String) {
            getPrefs(context).edit().putString(KEY_NODE_ID, nodeId).apply()
        }

        fun getNodeId(context: Context): String? =
            getPrefs(context).getString(KEY_NODE_ID, null)

        fun flagPendingPayment(context: Context) {
            getPrefs(context).edit().putBoolean(KEY_PENDING_PUSH_PAYMENT, true).apply()
        }

        fun clearPendingPayment(context: Context) {
            getPrefs(context).edit().putBoolean(KEY_PENDING_PUSH_PAYMENT, false).apply()
        }

        fun hasPendingPayment(context: Context): Boolean =
            getPrefs(context).getBoolean(KEY_PENDING_PUSH_PAYMENT, false)

        /**
         * Persisted before the keysend starts. An empty paymentId means the send outcome is
         * uncertain, so every sender must stop rather than risk sending the same adjustment twice.
         */
        fun savePendingOutgoingStabilityPayment(
            context: Context,
            paymentId: String,
            amountMsat: Long,
            btcPrice: Double
        ): Boolean =
            getPrefs(context).edit()
                .putString(KEY_PENDING_OUTGOING_PAYMENT_ID, paymentId)
                .putLong(KEY_PENDING_OUTGOING_AMOUNT_MSAT, amountMsat)
                .putLong(KEY_PENDING_OUTGOING_BTC_PRICE, java.lang.Double.doubleToRawLongBits(btcPrice))
                .commit()

        fun getPendingOutgoingStabilityPayment(context: Context): PendingOutgoingStabilityPayment? {
            val prefs = getPrefs(context)
            val hasMarker = prefs.contains(KEY_PENDING_OUTGOING_PAYMENT_ID) ||
                prefs.contains(KEY_PENDING_OUTGOING_AMOUNT_MSAT) ||
                prefs.contains(KEY_PENDING_OUTGOING_BTC_PRICE)
            if (!hasMarker) return null
            return PendingOutgoingStabilityPayment(
                paymentId = prefs.getString(KEY_PENDING_OUTGOING_PAYMENT_ID, "") ?: "",
                amountMsat = prefs.getLong(KEY_PENDING_OUTGOING_AMOUNT_MSAT, 0),
                btcPrice = java.lang.Double.longBitsToDouble(
                    prefs.getLong(KEY_PENDING_OUTGOING_BTC_PRICE, 0)
                )
            )
        }

        fun clearPendingOutgoingStabilityPayment(context: Context): Boolean =
            getPrefs(context).edit()
                .remove(KEY_PENDING_OUTGOING_PAYMENT_ID)
                .remove(KEY_PENDING_OUTGOING_AMOUNT_MSAT)
                .remove(KEY_PENDING_OUTGOING_BTC_PRICE)
                .commit()

        fun updateHeartbeat(context: Context) {
            val now = System.currentTimeMillis() / 1000
            getPrefs(context).edit().putLong(KEY_MAIN_APP_LAST_ACTIVE, now).apply()
        }

        fun registerTokenWithLSP(token: String, nodeId: String) {
            try {
                val json = JSONObject().apply {
                    put("device_token", token)
                    put("platform", "android")
                    put("node_id", nodeId)
                    put("environment", "production")
                }
                val body = json.toString()
                    .toRequestBody("application/json".toMediaType())
                val request = Request.Builder()
                    .url(Constants.LSP_PUSH_REGISTER_URL)
                    .post(body)
                    .build()
                val response = httpClient.newCall(request).execute()
                Log.d(TAG, "Push token registered with LSP: ${response.code}")
                response.close()
            } catch (e: Exception) {
                Log.e(TAG, "Failed to register push token with LSP", e)
            }
        }
    }

    override fun onNewToken(token: String) {
        Log.d(TAG, "New FCM token: ${token.take(16)}...")
        saveToken(this, token)

        val nodeId = getNodeId(this)
        if (nodeId != null) {
            Thread { registerTokenWithLSP(token, nodeId) }.start()
        }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        Log.d(TAG, "Push received: ${message.data}")

        val stabilityData = message.data["stability"] ?: return
        val direction = try {
            JSONObject(stabilityData).optString("direction", "lsp_to_user")
        } catch (_: Exception) {
            "lsp_to_user"
        }

        val prefs = getPrefs(this)
        val lastActive = prefs.getLong(KEY_MAIN_APP_LAST_ACTIVE, 0)
        val now = System.currentTimeMillis() / 1000

        if (now - lastActive < HEARTBEAT_THRESHOLD_SECS) {
            // Main app is running — let it handle on next stability cycle
            Log.d(TAG, "Main app active, flagging pending payment")
            flagPendingPayment(this)
            return
        }

        // Main app not running — start ForegroundService
        Log.d(TAG, "Starting StabilityProcessingService direction=$direction")
        val intent = Intent(this, StabilityProcessingService::class.java).apply {
            putExtra("direction", direction)
        }
        startForegroundService(intent)
    }
}
