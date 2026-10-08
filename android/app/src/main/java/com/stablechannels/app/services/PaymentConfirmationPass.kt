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

    /** [status] is the explorer's own status for the same transaction; null if it was missing. */
    private data class ReceiveTxLookup(
        val paysToAddress: Boolean,
        val status: TxConfirmationStatus?,
    )

    /** Outcome of a per-transaction explorer lookup. */
    private sealed interface TxLookup<out T> {
        data class Found<T>(val value: T) : TxLookup<T>

        /** Every explorer answered 404: the transaction is unknown to them. */
        data object NotFound : TxLookup<Nothing>

        /** Outage, error response or unreadable body: says nothing about the transaction. */
        data object Unavailable : TxLookup<Nothing>
    }

    /** [manual] marks a user-initiated, deadline-bounded pass that must never block on sync. */
    suspend fun run(manual: Boolean = false): ConfirmationPollResult {
        val db = database() ?: return ConfirmationPollResult.DatabaseUnavailable
        val staleFailed = db.failStaleTxidlessOnchain() > 0
        val tipHeight =
            fetchChainTipHeight()
                ?: run {
                    if (staleFailed) onRowsUpdated(false)
                    return ConfirmationPollResult.ChainTipUnavailable
                }
        val pending = db.getPaymentsNeedingConfirmation(limit = 100)
        var anyUpdated = staleFailed
        var failedLookups = 0
        var finished = false

        try {
            for (payment in pending) {
                currentCoroutineContext().ensureActive()
                val txid = payment.txid ?: continue
                var receiveStatus: TxConfirmationStatus? = null

                if (payment.paymentType == "onchain" && payment.direction == "received") {
                    val expectedAddress = payment.address?.trim().orEmpty()
                    if (expectedAddress.isNotEmpty()) {
                        val lookup =
                            when (val result = fetchReceiveTx(txid, expectedAddress)) {
                                is TxLookup.Found -> result.value
                                TxLookup.NotFound -> {
                                    if (db.failStaleOnchainRow(payment.id)) anyUpdated = true
                                    else failedLookups++
                                    continue
                                }
                                TxLookup.Unavailable -> {
                                    failedLookups++
                                    continue
                                }
                            }
                        if (!lookup.paysToAddress) {
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
                        receiveStatus = lookup.status
                    }
                }

                val txStatus =
                    receiveStatus
                        ?: when (val result = fetchTxConfirmationStatus(txid)) {
                            is TxLookup.Found -> result.value
                            // Only an authoritative "not found" may retire an old row; an outage
                            // must leave it pollable so a later recovery can still confirm it.
                            TxLookup.NotFound -> {
                                if (db.failStaleOnchainRow(payment.id)) anyUpdated = true
                                else failedLookups++
                                continue
                            }
                            TxLookup.Unavailable -> {
                                failedLookups++
                                continue
                            }
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

    private suspend fun fetchTxConfirmationStatus(txid: String): TxLookup<TxConfirmationStatus> {
        val normalizedTxid = txid.substringBefore(":").trim()
        if (normalizedTxid.isEmpty()) return TxLookup.Unavailable

        return lookupAcrossExplorers("/tx/$normalizedTxid/status") { body ->
            parseStatus(JSONObject(body))
        }
    }

    /**
     * Whether [txid] pays [address], or null if that could not be established. Used to confirm a
     * transaction belongs to a pending receive row before attaching it.
     */
    suspend fun paysToAddress(txid: String, address: String): Boolean? =
        (fetchReceiveTx(txid, address) as? TxLookup.Found)?.value?.paysToAddress

    /**
     * One `/tx/:txid` request answers both "does it pay [address]" and its confirmation status (the
     * response embeds the same `status` object as `/tx/:txid/status`), so receives need no second
     * request.
     */
    private suspend fun fetchReceiveTx(txid: String, address: String): TxLookup<ReceiveTxLookup> {
        val normalizedTxid = txid.substringBefore(":").trim()
        val targetAddress = QRCodeUtils.normalizeAddress(address)
        if (normalizedTxid.isEmpty() || targetAddress.isBlank()) return TxLookup.Unavailable

        return lookupAcrossExplorers("/tx/$normalizedTxid") { body ->
            val tx = JSONObject(body)
            val vouts = tx.optJSONArray("vout") ?: return@lookupAcrossExplorers null
            var paysToAddress = false
            for (i in 0 until vouts.length()) {
                val vout = vouts.optJSONObject(i) ?: continue
                val voutAddress =
                    QRCodeUtils.normalizeAddress(vout.optString("scriptpubkey_address", ""))
                if (voutAddress == targetAddress) {
                    paysToAddress = true
                    break
                }
            }
            ReceiveTxLookup(paysToAddress, tx.optJSONObject("status")?.let(::parseStatus))
        }
    }

    /**
     * Tries each explorer in turn and returns the first body [parse] accepts. [TxLookup.NotFound]
     * only if every explorer answered 404; any other failure makes the result unavailable.
     */
    private suspend fun <T> lookupAcrossExplorers(
        path: String,
        parse: (String) -> T?,
    ): TxLookup<T> {
        var notFound = 0
        var otherFailure = false
        for (baseUrl in chainUrls()) {
            when (val response = fetchResponse("${baseUrl.trimEnd('/')}$path")) {
                is HttpBody.NotFound -> notFound++
                is HttpBody.Failed -> otherFailure = true
                is HttpBody.Ok -> {
                    val parsed =
                        try {
                            parse(response.body)
                        } catch (_: Exception) {
                            null
                        }
                    if (parsed != null) return TxLookup.Found(parsed)
                    otherFailure = true
                }
            }
        }
        return if (notFound > 0 && !otherFailure) TxLookup.NotFound else TxLookup.Unavailable
    }

    private fun parseStatus(json: JSONObject): TxConfirmationStatus {
        val blockHeight =
            if (json.has("block_height") && !json.isNull("block_height")) {
                json.optInt("block_height", 0).takeIf { it > 0 }
            } else {
                null
            }
        return TxConfirmationStatus(json.optBoolean("confirmed", false), blockHeight)
    }

    /** Body of a successful response, or null on any failure. Rethrows cancellation. */
    private suspend fun fetchBody(url: String): String? = (fetchResponse(url) as? HttpBody.Ok)?.body

    /** Rethrows cancellation; every other failure is [HttpBody.Failed]. */
    private suspend fun fetchResponse(url: String): HttpBody =
        try {
            httpClient.awaitBody(Request.Builder().url(url).build())
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            HttpBody.Failed
        }
}

internal sealed interface HttpBody {
    data class Ok(val body: String) : HttpBody

    data object NotFound : HttpBody

    /** Non-success status other than 404, or a missing body. */
    data object Failed : HttpBody
}

/**
 * Runs [request] asynchronously and returns the body of a successful response (null otherwise). The
 * body is read on OkHttp's thread, so cancelling the coroutine cancels the [Call] and aborts both a
 * pending connection and a stalled body read instead of leaving a blocked thread behind.
 */
internal suspend fun OkHttpClient.awaitSuccessfulBody(request: Request): String? =
    (awaitBody(request) as? HttpBody.Ok)?.body

internal suspend fun OkHttpClient.awaitBody(request: Request): HttpBody =
    suspendCancellableCoroutine { continuation ->
        val call = newCall(request)
        continuation.invokeOnCancellation { call.cancel() }
        call.enqueue(
            object : Callback {
                override fun onFailure(call: Call, e: IOException) {
                    continuation.resumeWithException(e)
                }

                override fun onResponse(call: Call, response: Response) {
                    val result =
                        try {
                            response.use {
                                when {
                                    it.code == 404 -> HttpBody.NotFound
                                    !it.isSuccessful -> HttpBody.Failed
                                    else -> it.body?.string()?.let(HttpBody::Ok) ?: HttpBody.Failed
                                }
                            }
                        } catch (e: IOException) {
                            continuation.resumeWithException(e)
                            return
                        }
                    continuation.resume(result)
                }
            }
        )
    }
