package com.stablechannels.app.services

import com.stablechannels.app.AppState
import com.stablechannels.app.util.QRCodeUtils
import java.io.IOException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import okhttp3.Call
import okhttp3.Callback
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import org.json.JSONObject

/**
 * One payment-confirmation pass: fetches the chain tip, then checks each payment awaiting
 * confirmation against the block explorer and stores the new confirmation state.
 *
 * Every HTTP request suspends and is cancelled with the calling coroutine, so a caller's deadline
 * (or the History screen going away) aborts an in-flight lookup and stops the remaining rows.
 */
class PaymentConfirmationPass(
    private val httpClient: OkHttpClient,
    private val chainUrls: () -> List<String>,
    private val database: () -> DatabaseService?,
    private val onReceiveTxidMismatch: (txid: String) -> Unit = {},
    /**
     * Called when rows changed. [syncInline] is true only for an automatic pass that finished: it
     * may block on the wallet sync. A manual (bounded) or cut-short pass must hand that work off.
     */
    private val onRowsUpdated: (syncInline: Boolean) -> Unit = {},
) {
    private data class TxConfirmationStatus(val confirmed: Boolean, val blockHeight: Int?)

    /** [manual] marks a user-initiated, deadline-bounded pass that must never block on sync. */
    suspend fun run(manual: Boolean = false): ConfirmationPollResult {
        val db = database() ?: return ConfirmationPollResult.DatabaseUnavailable
        val tipHeight = fetchChainTipHeight() ?: return ConfirmationPollResult.ChainTipUnavailable
        val pending = db.getPaymentsNeedingConfirmation(limit = 100)
        var anyUpdated = false
        var failedLookups = 0
        var finished = false

        try {
            for (payment in pending) {
                currentCoroutineContext().ensureActive()
                val txid = payment.txid ?: continue

                if (payment.paymentType == "onchain" && payment.direction == "received") {
                    val expectedAddress = payment.address?.trim().orEmpty()
                    if (expectedAddress.isNotEmpty()) {
                        when (fetchTxPaysToAddress(txid, expectedAddress)) {
                            false -> {
                                val cleared = db.clearPaymentTxidForRow(payment.id)
                                anyUpdated = anyUpdated || cleared
                                onReceiveTxidMismatch(txid)
                                AuditService.log(
                                    "ONCHAIN_TXID_ADDRESS_MISMATCH",
                                    mapOf(
                                        "payment_id" to payment.id,
                                        "txid" to txid,
                                        "address" to expectedAddress,
                                    ),
                                )
                                continue
                            }
                            null -> {
                                failedLookups++
                                continue
                            }
                            true -> {}
                        }
                    }
                }

                val txStatus = fetchTxConfirmationStatus(txid)
                if (txStatus == null) {
                    failedLookups++
                    continue
                }
                val required = AppState.requiredConfirmationsForType(payment.paymentType)

                val (newConfirmations, newStatus) =
                    if (!txStatus.confirmed) {
                        0 to "pending"
                    } else {
                        val blockHeight = txStatus.blockHeight
                        val confs =
                            if (blockHeight != null) {
                                (tipHeight - blockHeight + 1)
                                    .coerceAtLeast(0)
                                    .coerceAtMost(required)
                            } else {
                                payment.confirmations.coerceAtLeast(1).coerceAtMost(required)
                            }
                        confs to if (confs >= required) "completed" else "pending"
                    }

                if (payment.confirmations != newConfirmations || payment.status != newStatus) {
                    val updated =
                        db.updatePaymentConfirmationState(
                            paymentRowId = payment.id,
                            confirmations = newConfirmations,
                            status = newStatus,
                        )
                    anyUpdated = anyUpdated || updated
                }
            }
            finished = true
        } finally {
            // Rows already written must still reach the UI and balances if the pass is cut short.
            if (anyUpdated) onRowsUpdated(finished && !manual)
        }
        return ConfirmationPollResult.Completed(failedLookups)
    }

    private suspend fun fetchChainTipHeight(): Int? {
        for (baseUrl in chainUrls()) {
            val body = fetchBody("${baseUrl.trimEnd('/')}/blocks/tip/height") ?: continue
            body.trim().toIntOrNull()?.let {
                return it
            }
        }
        return null
    }

    private suspend fun fetchTxConfirmationStatus(txid: String): TxConfirmationStatus? {
        val normalizedTxid = txid.substringBefore(":").trim()
        if (normalizedTxid.isEmpty()) return null

        for (baseUrl in chainUrls()) {
            val body = fetchBody("${baseUrl.trimEnd('/')}/tx/$normalizedTxid/status") ?: continue
            try {
                val json = JSONObject(body)
                val confirmed = json.optBoolean("confirmed", false)
                val blockHeight =
                    if (json.has("block_height") && !json.isNull("block_height")) {
                        json.optInt("block_height", 0).takeIf { it > 0 }
                    } else {
                        null
                    }
                return TxConfirmationStatus(confirmed = confirmed, blockHeight = blockHeight)
            } catch (_: Exception) {}
        }
        return null
    }

    private suspend fun fetchTxPaysToAddress(txid: String, address: String): Boolean? {
        val normalizedTxid = txid.substringBefore(":").trim()
        val targetAddress = QRCodeUtils.normalizeAddress(address)
        if (normalizedTxid.isEmpty() || targetAddress.isBlank()) return null

        for (baseUrl in chainUrls()) {
            val body = fetchBody("${baseUrl.trimEnd('/')}/tx/$normalizedTxid") ?: continue
            try {
                val vouts = JSONObject(body).optJSONArray("vout") ?: continue
                for (i in 0 until vouts.length()) {
                    val vout = vouts.optJSONObject(i) ?: continue
                    val voutAddress =
                        QRCodeUtils.normalizeAddress(vout.optString("scriptpubkey_address", ""))
                    if (voutAddress == targetAddress) return true
                }
                return false
            } catch (_: Exception) {}
        }
        return null
    }

    /** Body of a successful response, or null on any failure. Rethrows cancellation. */
    private suspend fun fetchBody(url: String): String? =
        try {
            httpClient.awaitSuccessfulBody(Request.Builder().url(url).build())
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            null
        }
}

/**
 * Runs [request] asynchronously and returns the body of a successful response (null otherwise). The
 * body is read on OkHttp's thread, so cancelling the coroutine cancels the [Call] and aborts both a
 * pending connection and a stalled body read instead of leaving a blocked thread behind.
 */
internal suspend fun OkHttpClient.awaitSuccessfulBody(request: Request): String? =
    suspendCancellableCoroutine { continuation ->
        val call = newCall(request)
        continuation.invokeOnCancellation { call.cancel() }
        call.enqueue(
            object : Callback {
                override fun onFailure(call: Call, e: IOException) {
                    continuation.resumeWithException(e)
                }

                override fun onResponse(call: Call, response: Response) {
                    val body =
                        try {
                            response.use { if (it.isSuccessful) it.body?.string() else null }
                        } catch (e: IOException) {
                            continuation.resumeWithException(e)
                            return
                        }
                    continuation.resume(body)
                }
            }
        )
    }
