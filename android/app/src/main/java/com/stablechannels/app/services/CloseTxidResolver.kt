package com.stablechannels.app.services

import android.util.Log
import kotlinx.coroutines.*
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * Polls Esplora /tx/{fundingTxid}/outspend/{vout} to resolve the close
 * transaction ID for a force-closed channel. Runs in background until
 * the close TX is found or budget expires.
 */
class CloseTxidResolver(
    private val chainURLs: List<String>,
    private val onResolved: (paymentId: String, closeTxid: String) -> Unit
) {
    companion object {
        private const val TAG = "CloseTxidResolver"
        private const val MAX_ATTEMPTS = 20
        private val BACKOFF_SECONDS = listOf(5L, 10L, 15L, 30L, 60L, 60L, 60L, 60L, 60L, 60L)
    }

    private val client = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    /**
     * Start polling for the close TX. Call from a coroutine scope.
     * @param paymentId The payment ID to update when resolved
     * @param fundingTxid The funding transaction ID of the channel
     * @param vout The output index of the funding transaction
     * @param databaseService The database service to update the payment
     */
    suspend fun resolve(
        paymentId: String,
        fundingTxid: String,
        vout: Int,
        databaseService: DatabaseService
    ) = withContext(Dispatchers.IO) {
        Log.d(TAG, "Starting close TX resolution for paymentId=$paymentId, fundingTxid=$fundingTxid:$vout")

        for (attempt in 0 until MAX_ATTEMPTS) {
            try {
                val closeTxid = pollForCloseTx(fundingTxid, vout)
                if (closeTxid != null) {
                    Log.d(TAG, "Close TX resolved: $closeTxid (attempt ${attempt + 1})")
                    databaseService.updatePaymentTxid(paymentId, closeTxid)
                    onResolved(paymentId, closeTxid)
                    return@withContext
                }
            } catch (e: Exception) {
                Log.w(TAG, "Attempt ${attempt + 1} failed: ${e.message}")
            }

            // Backoff before retry
            val delaySec = BACKOFF_SECONDS.getOrElse(attempt) { 60L }
            Log.d(TAG, "Waiting ${delaySec}s before retry...")
            delay(delaySec * 1000)
        }

        Log.w(TAG, "Close TX resolution failed after $MAX_ATTEMPTS attempts")
    }

    /**
     * Poll a single Esplora endpoint for the close TX.
     * @return The close TX if found, null otherwise
     */
    private fun pollForCloseTx(fundingTxid: String, vout: Int): String? {
        for (baseURL in chainURLs) {
            try {
                val url = "${baseURL.trimEnd('/')}/tx/$fundingTxid/outspend/$vout"
                val request = Request.Builder().url(url).build()
                val response = client.newCall(request).execute()
                val body = response.body?.string() ?: continue

                if (!response.isSuccessful) continue

                val json = JSONObject(body)
                val spent = json.optBoolean("spent", false)

                if (spent) {
                    val txid = json.optString("txid", "")
                    if (txid.isNotEmpty() && isValidTxid(txid)) {
                        return txid
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "Failed to poll $baseURL: ${e.message}")
            }
        }
        return null
    }

    /**
     * Validate a transaction ID (64 hex characters).
     */
    private fun isValidTxid(txid: String): Boolean {
        return txid.length == 64 && txid.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }
    }
}
