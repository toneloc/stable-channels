package com.stablechannels.app.services

import android.database.sqlite.SQLiteDatabase
import java.io.File
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import uniffi.sc_force_close.ForceCloseTransactions
import uniffi.sc_force_close.StoredValue
import uniffi.sc_force_close.buildForceClose

/**
 * The app's own force close (#456). ldk-node does not broadcast the close of an anchor channel with
 * a peer in `trustedPeersNoReserve`, and the app lists the LSP there, so the close would wait for
 * the LSP to broadcast its own.
 *
 * The app force closes only when the LSP is offline or a normal close is stuck. After [start], the
 * channel is followed until a close of it confirms: the app builds its close with a fee bump and
 * broadcasts both to the explorers that don't have a close of the channel, again every [RETRY_MS].
 * The LSP's close can still replace ours in the mempool when it comes back, as it can pay a higher
 * fee from its wallet. After [MAX_BUILD_FAILURES] failed builds the app stops, as building stops
 * the node each time; the LSP's close still frees the funds.
 */
class OwnForceClose(
    private val http: OkHttpClient,
    private val chainUrls: () -> List<String>,
    private val loadState: () -> String?,
    private val saveState: (String?) -> Unit,
    private val now: () -> Long = System::currentTimeMillis,
    private val log: (String, Map<String, Any?>) -> Unit = AuditService::log,
) {
    @Serializable
    data class State(
        val channelId: String,
        val fundingTxid: String,
        val fundingVout: Int,
        val nextAt: Long,
        val closeTxid: String? = null,
        val closeHex: String? = null,
        val feeBumpTxid: String? = null,
        val feeBumpHex: String? = null,
        val broadcastAt: Long = 0,
        /** The unconfirmed close last seen spending the funding output: ours or the LSP's. */
        val spendTxid: String? = null,
        val buildFailures: Int = 0,
        /** When an explorer first showed our close as [spendTxid]. */
        val seenAt: Long? = null,
    ) {
        /** Our close is in a mempool, unconfirmed [SLOW_MS] after an explorer first showed it. */
        fun isSlow(now: Long): Boolean =
            closeTxid != null && spendTxid == closeTxid && seenAt != null && now - seenAt >= SLOW_MS
    }

    private val json = Json { ignoreUnknownKeys = true }
    private val running = Mutex()
    private val _state =
        MutableStateFlow(
            loadState()?.let { runCatching { json.decodeFromString<State>(it) }.getOrNull() }
        )
    val state: StateFlow<State?> = _state.asStateFlow()

    val isDue: Boolean
        get() = _state.value?.let { now() >= it.nextAt } == true

    fun start(channelId: String, fundingTxid: String, fundingVout: Int) {
        val at = now()
        save(State(channelId, fundingTxid, fundingVout, at))
    }

    /**
     * Stops following the close once a step in progress ends. A restore from seed calls this: the
     * restored node lacks the channel data to sweep what our close pays us, while the recovery
     * sweeps the LSP's close (#364).
     */
    suspend fun clear() = running.withLock { save(null) }

    /**
     * One step. [build] builds the channel's close, with the node stopped. [onConfirmed] gets the
     * channel and the close that confirmed: ours or the LSP's. Returns a message for the user, or
     * null when there is nothing new to tell.
     */
    suspend fun step(
        build: suspend (String) -> ForceCloseTransactions,
        onConfirmed: (channelId: String, closeTxid: String) -> Unit = { _, _ -> },
    ): String? {
        if (!running.tryLock()) return null
        try {
            var state = _state.value ?: return null
            val spend = fundingSpend(state)
            if (spend?.confirmed == true) {
                log(
                    "OWN_FORCE_CLOSE_DONE",
                    mapOf("channel_id" to state.channelId, "close_txid" to spend.txid),
                )
                save(state, null)
                spend.txid?.let { onConfirmed(state.channelId, it) }
                return null
            }
            if (state.closeHex == null) {
                val close =
                    try {
                        build(state.channelId)
                    } catch (e: Exception) {
                        return buildFailed(state, e)
                    }
                val built =
                    state.copy(
                        closeTxid = close.closeTxid,
                        closeHex = close.closeTxHex,
                        feeBumpTxid = close.anchorChildTxid,
                        feeBumpHex = close.anchorChildHex,
                    )
                save(state, built)
                if (_state.value != built) return null
                state = built
            }
            when {
                spend == null -> save(state, state.copy(nextAt = now() + CHECK_MS))
                spend.unspentAt.isNotEmpty() -> return send(state, spend.unspentAt, spend.txid)
                else -> save(state, state.seen(spend.txid).copy(nextAt = now() + CHECK_MS))
            }
            return null
        } finally {
            running.unlock()
        }
    }

    private fun buildFailed(current: State, e: Exception): String {
        val failures = current.buildFailures + 1
        log(
            "OWN_FORCE_CLOSE_FAILED",
            mapOf(
                "channel_id" to current.channelId,
                "error" to e.toString(),
                "failures" to failures,
            ),
        )
        if (failures >= MAX_BUILD_FAILURES) {
            save(current, null)
            return "Close could not be created. Funds arrive once the provider closes the channel."
        }
        save(current, current.copy(buildFailures = failures, nextAt = now() + RETRY_MS))
        return "Close not created yet. Retrying in 10 minutes."
    }

    /**
     * This state with [txid] as the close seen spending the funding output. Our close is timed from
     * when an explorer first shows it, for [State.isSlow].
     */
    private fun State.seen(txid: String?): State =
        copy(
            spendTxid = txid,
            seenAt = if (txid != null && txid == closeTxid) seenAt ?: now() else null,
        )

    /**
     * Broadcasts our close to [urls], the explorers that see the funding output unspent, again
     * every [RETRY_MS]. [spendTxid] is a close another explorer already has.
     */
    private fun send(current: State, urls: List<String>, spendTxid: String?): String? {
        val state = current.seen(spendTxid)
        if (now() < state.broadcastAt + RETRY_MS) {
            save(current, state.copy(nextAt = now() + CHECK_MS))
            return null
        }
        // The fee bump spends the close, so it goes second.
        val closeResults = broadcast(http, urls, state.closeHex!!)
        val bumpResults = broadcast(http, urls, state.feeBumpHex!!)
        log(
            "OWN_FORCE_CLOSE_BROADCAST",
            mapOf(
                "channel_id" to state.channelId,
                "close_txid" to state.closeTxid,
                "close_broadcast" to closeResults.toString(),
                "fee_bump_txid" to state.feeBumpTxid,
                "fee_bump_broadcast" to bumpResults.toString(),
            ),
        )
        save(current, state.copy(broadcastAt = now(), nextAt = now() + CHECK_MS))
        return when {
            // A close of the channel is already in a mempool: nothing new to tell.
            spendTxid != null -> null
            closeResults.values.any { !it.startsWith(ERROR) } -> "Close sent."
            else -> "Close not accepted yet. Retrying in 10 minutes."
        }
    }

    /** One explorer's answer: the tx spending the funding output, if any. */
    private data class Outspend(val txid: String?, val confirmed: Boolean)

    /**
     * What the explorers that answer see. [txid] is a close one of them has. Our close goes to
     * [unspentAt], those that see the funding output unspent, which does no harm if it is spent. A
     * close counts as confirmed only when all of them see it confirmed.
     */
    private data class Spend(val txid: String?, val confirmed: Boolean, val unspentAt: List<String>)

    private fun fundingSpend(state: State): Spend? {
        val answers = chainUrls().mapNotNull { url -> outspend(url, state)?.let { url to it } }
        val first = answers.firstOrNull()?.second ?: return null
        return Spend(
            txid = answers.firstNotNullOfOrNull { it.second.txid },
            confirmed = answers.all { it.second.confirmed && it.second.txid == first.txid },
            unspentAt = answers.filter { it.second.txid == null }.map { it.first },
        )
    }

    /**
     * One explorer's answer, or null when it gives none. Esplora reports an output of a tx it does
     * not have as unspent, so an unspent output counts only when it has the funding tx: the close
     * of a channel whose funding never reached the chain can't be sent.
     */
    private fun outspend(base: String, state: State): Outspend? =
        try {
            val url = "${base.trimEnd('/')}/tx/${state.fundingTxid}/outspend/${state.fundingVout}"
            http.newCall(Request.Builder().url(url).build()).execute().use { response ->
                if (!response.isSuccessful) return null
                val outspend = JSONObject(response.body?.string().orEmpty())
                if (!outspend.optBoolean("spent")) {
                    return if (hasTx(base, state.fundingTxid)) Outspend(null, false) else null
                }
                Outspend(
                    outspend.optString("txid").ifEmpty { null },
                    outspend.optJSONObject("status")?.optBoolean("confirmed") == true,
                )
            }
        } catch (_: Exception) {
            null
        }

    /** Whether the explorer has [txid], in the mempool or on chain. */
    private fun hasTx(base: String, txid: String): Boolean {
        val request = Request.Builder().url("${base.trimEnd('/')}/tx/$txid/hex").build()
        return http.newCall(request).execute().use { it.isSuccessful }
    }

    /** Saves [next] unless a [start] during the step replaced [current]. */
    private fun save(current: State, next: State?) {
        if (_state.value == current) save(next)
    }

    private fun save(state: State?) {
        _state.value = state
        saveState(state?.let { json.encodeToString(it) })
    }

    companion object {
        const val CHECK_MS = 60 * 1000L
        const val RETRY_MS = 10 * 60 * 1000L
        const val SLOW_MS = 60 * 60 * 1000L
        const val MAX_BUILD_FAILURES = 3
        private const val ERROR = "error: "

        /**
         * Builds the signed close of [channelId] and its fee bump from the node's stored data. The
         * node must be stopped and [LdkNodeOwner] held, so no other SQLite writes the data while it
         * is read.
         */
        fun build(dataDir: File, channelId: String, seed: ByteArray): ForceCloseTransactions =
            buildForceClose(channelId, seed, readMonitors(dataDir))

        /** ldk-node's channel monitors and their pending updates, read in one query. */
        internal fun readMonitors(dataDir: File): List<StoredValue> {
            val path = File(dataDir, "ldk_node_data.sqlite").absolutePath
            SQLiteDatabase.openDatabase(path, null, SQLiteDatabase.OPEN_READONLY).use { db ->
                db.rawQuery(
                        "SELECT primary_namespace, secondary_namespace, key, value FROM ldk_node_data " +
                            "WHERE primary_namespace IN ('monitors', 'monitor_updates')",
                        null,
                    )
                    .use { cursor ->
                        val values = mutableListOf<StoredValue>()
                        while (cursor.moveToNext()) {
                            values +=
                                StoredValue(
                                    cursor.getString(0),
                                    cursor.getString(1),
                                    cursor.getString(2),
                                    cursor.getBlob(3),
                                )
                        }
                        return values
                    }
            }
        }

        /** Sends [hex] to every explorer. Returns each explorer's answer: the txid or an error. */
        fun broadcast(
            http: OkHttpClient,
            chainUrls: List<String>,
            hex: String,
        ): Map<String, String> = chainUrls.associateWith { base ->
            try {
                val body = hex.toRequestBody("text/plain".toMediaType())
                val request = Request.Builder().url(base.trimEnd('/') + "/tx").post(body).build()
                http.newCall(request).execute().use { response ->
                    val text = response.body?.string().orEmpty().trim()
                    if (response.isSuccessful) text else "$ERROR${response.code} ${text.take(200)}"
                }
            } catch (e: Exception) {
                "$ERROR${e.message ?: e.javaClass.simpleName}"
            }
        }
    }
}
