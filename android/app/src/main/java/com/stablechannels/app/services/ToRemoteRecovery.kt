package com.stablechannels.app.services

import fr.acinq.bitcoin.Bitcoin
import fr.acinq.bitcoin.ByteVector
import fr.acinq.bitcoin.Chain
import fr.acinq.bitcoin.Script
import fr.acinq.bitcoin.Transaction
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject

/**
 * Finds and moves to the wallet our side of a channel the LSP force-closed after the app lost its
 * channel data (#364, #347), which LDK no longer tracks. It does in the app what the force close
 * recovery script does by hand. It finds the close from a funding txid saved before the loss: what
 * spent the funding output is the close, and its outputs are matched against our keys (a few
 * requests). A restore on a new device has no saved funding txid; getting it from the LSP is a
 * follow-up. Scanning the scripts of all 1000 keys instead would take 1000 requests, more than the
 * public explorers allow an hour.
 *
 * The search repeats on a growing delay until the LSP's close shows up. A found output is swept
 * once its close confirms, and stays counted on the Home card until the sweep confirms.
 */
class ToRemoteRecovery(
    private val http: OkHttpClient,
    private val chainUrls: () -> List<String>,
    private val loadState: () -> String?,
    private val saveState: (String) -> Unit,
    private val now: () -> Long = System::currentTimeMillis,
    private val log: (String, Map<String, Any?>) -> Unit = AuditService::log,
    /** A sweep was broadcast: its txid, the sats it pays the wallet and the address. */
    private val onSwept: (String, Long, String) -> Unit = { _, _, _ -> },
) {
    @Serializable
    data class Output(val txid: String, val vout: Int, val valueSats: Long, val keyIndex: Int)

    @Serializable
    data class State(
        val fundingTxid: String? = null,
        val fundingVout: Int? = null,
        /** The search stops after this time when nothing new is found. */
        val until: Long = 0,
        val attempts: Int = 0,
        val nextAt: Long = 0,
        /** Bumped by [start], so a search that began earlier doesn't undo it. */
        val generation: Int = 0,
        val found: List<Output> = emptyList(),
        /** Kept across attempts so a failing sweep doesn't use up wallet addresses. */
        val destination: String? = null,
        /** The last funding txid fully checked, so it is not looked up again. */
        val checkedFundingTxid: String? = null,
    ) {
        val active: Boolean
            get() = fundingTxid != null || found.isNotEmpty()
    }

    private val json = Json { ignoreUnknownKeys = true }
    private val running = Mutex()
    private var lastSettleAt = 0L

    @Volatile
    var state: State =
        loadState()?.let { runCatching { json.decodeFromString<State>(it) }.getOrNull() } ?: State()
        private set

    /** Sats found on chain and not yet confirmed in the wallet. */
    val pendingSats: Long
        get() = state.found.sumOf { it.valueSats }

    /** Whether a funding txid that LDK no longer knows still needs looking up. */
    fun needsCheck(fundingTxid: String): Boolean {
        val s = state
        return s.found.isEmpty() &&
            fundingTxid != s.fundingTxid &&
            fundingTxid != s.checkedFundingTxid
    }

    /**
     * Called on each balance refresh. A saved funding txid with no channel left in LDK means the
     * channel data was lost (#347), so that funding txid gets looked up. Returns whether recovery
     * may run now: never while LDK handles a close itself, so the two never race.
     */
    fun onRefresh(
        savedFundingTxid: String?,
        savedFundingVout: Int?,
        ldkHasChannels: Boolean,
        ldkHandlesClose: Boolean,
    ): Boolean {
        if (ldkHandlesClose) return false
        if (savedFundingTxid != null && !ldkHasChannels && needsCheck(savedFundingTxid)) {
            start(savedFundingTxid, savedFundingVout)
        }
        return state.active
    }

    /**
     * Starts looking for the close of the channel funded by [fundingTxid], which LDK doesn't track.
     * With [repeat] false it looks once, for a close that already happened.
     */
    fun start(fundingTxid: String, fundingVout: Int?, repeat: Boolean = true) {
        val until = if (repeat) now() + SEARCH_MS else now()
        update {
            it.copy(
                fundingTxid = fundingTxid,
                fundingVout = fundingVout,
                until = maxOf(until, it.until),
                attempts = 0,
                nextAt = 0,
                generation = it.generation + 1,
            )
        }
        log("TO_REMOTE_RECOVERY_START", mapOf("funding_txid" to fundingTxid, "repeat" to repeat))
    }

    /**
     * After a restore from seed: starts over, as found outputs and the sweep address may belong to
     * the wallet that was replaced, then looks for the close of [fundingTxid] when it is known.
     */
    fun startAfterRestore(fundingTxid: String?, fundingVout: Int?, repeat: Boolean) {
        update { State(generation = it.generation + 1) }
        if (fundingTxid != null) start(fundingTxid, fundingVout, repeat)
    }

    /** Does whatever is due. Cheap to call often; returns at once while another run is going. */
    suspend fun run(
        keys: () -> ToRemoteKeys?,
        destination: () -> String?,
        feeRateSatVb: () -> Long?,
    ) {
        if (!state.active || !running.tryLock()) return
        try {
            withContext(Dispatchers.IO) {
                val k = keys() ?: return@withContext
                if (state.found.isNotEmpty()) settle(k, destination, feeRateSatVb)
                if (state.fundingTxid != null) search(k)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Throwable) {
            // A malformed explorer reply, or a native library that fails to load, must not crash
            // the app on every refresh.
            log("TO_REMOTE_ERROR", mapOf("error" to (e.message ?: e.javaClass.simpleName)))
        } finally {
            running.unlock()
        }
    }

    private fun search(keys: ToRemoteKeys) {
        val s = state
        val fundingTxid = s.fundingTxid ?: return
        if (now() < s.nextAt) return
        val result = findByFunding(fundingTxid, s.fundingVout, keys)
        val found = result?.first.orEmpty()
        val time = now()
        // An explorer that can't be reached is not an answer, so even a one-time check is retried.
        val ends = result != null && (result.second || time >= s.until)
        val added = found.filter { o -> s.found.none { it.txid == o.txid && it.vout == o.vout } }
        update {
            if (it.generation != s.generation) return@update it
            it.copy(
                found = it.found + added,
                fundingTxid = if (ends) null else it.fundingTxid,
                checkedFundingTxid = if (ends) fundingTxid else it.checkedFundingTxid,
                attempts = it.attempts + 1,
                nextAt = time + backoffMs(it.attempts + 1),
            )
        }
        if (added.isNotEmpty()) {
            log(
                "TO_REMOTE_FOUND",
                mapOf(
                    "outpoints" to added.joinToString { "${it.txid}:${it.vout}" },
                    "sats" to added.sumOf { it.valueSats },
                ),
            )
        }
        if (!state.active) log("TO_REMOTE_RECOVERY_END", mapOf("found" to false))
    }

    /**
     * The outputs paying our keys in whatever spent the funding output, and whether the funding
     * output is spent. With no saved vout, the funding output is the one spent with a 2-of-2
     * multisig witness, as every channel close is. Null when the explorer can't be reached. Outputs
     * already swept and confirmed are left out.
     */
    private fun findByFunding(
        txid: String,
        vout: Int?,
        keys: ToRemoteKeys,
    ): Pair<List<Output>, Boolean>? {
        // A funding tx no explorer knows is not spent. That is an answer, unlike no reply.
        val spends = getArray("/tx/$txid/outspends", emptyIfMissing = true) ?: return null
        val indices = vout?.let { listOf(it) } ?: (0 until spends.length()).toList()
        var fundingSpent = false
        val found = mutableListOf<Output>()
        for (index in indices) {
            val spend = spends.optJSONObject(index) ?: continue
            if (!spend.optBoolean("spent")) continue
            val closeTxid = spend.getString("txid")
            val close = getObject("/tx/$closeTxid") ?: return null
            if (vout != null || spendsWithTwoOfTwo(close, txid, index)) fundingSpent = true
            val outputs = close.optJSONArray("vout") ?: continue
            val matched =
                (0 until outputs.length()).mapNotNull { n ->
                    val out = outputs.getJSONObject(n)
                    keys.match(ByteVector(out.getString("scriptpubkey")).toByteArray())?.let {
                        Output(closeTxid, n, out.getLong("value"), it.index)
                    }
                }
            if (matched.isEmpty()) continue
            val closeSpends = getArray("/tx/$closeTxid/outspends") ?: return null
            found += matched.filterNot { isSpentAndConfirmed(closeSpends.optJSONObject(it.vout)) }
        }
        return found to fundingSpent
    }

    /** Whether [tx] spends txid:index with a 2-of-2 multisig witness script. */
    private fun spendsWithTwoOfTwo(tx: JSONObject, txid: String, index: Int): Boolean {
        val inputs = tx.optJSONArray("vin") ?: return false
        return (0 until inputs.length()).any { i ->
            val input = inputs.getJSONObject(i)
            val witness = input.optJSONArray("witness")
            input.optString("txid") == txid &&
                input.optInt("vout", -1) == index &&
                witness != null &&
                witness.length() > 0 &&
                isTwoOfTwo(witness.getString(witness.length() - 1))
        }
    }

    private fun settle(
        keys: ToRemoteKeys,
        destination: () -> String?,
        feeRateSatVb: () -> Long?,
    ) {
        if (now() - lastSettleAt < SETTLE_INTERVAL_MS) return
        lastSettleAt = now()
        // A restore can reset the state while this waits on the explorer. What this settle found
        // then belongs to the replaced wallet, so it stops before saving or broadcasting anything.
        val generation = state.generation
        val done = mutableListOf<Output>()
        for (output in state.found) {
            val spend = getObject("/tx/${output.txid}/outspend/${output.vout}") ?: continue
            if (spend.optBoolean("spent")) {
                if (isSpentAndConfirmed(spend)) done += output
                continue
            }
            // The output is locked until one block after the close confirms (1 OP_CSV).
            val status = getObject("/tx/${output.txid}/status") ?: continue
            if (!status.optBoolean("confirmed")) continue
            if (state.generation != generation) return
            val address = state.destination ?: destination() ?: continue
            if (state.destination == null) {
                update { if (it.generation == generation) it.copy(destination = address) else it }
            }
            val script =
                Bitcoin.addressToPublicKeyScript(Chain.Mainnet.chainHash, address).right?.let {
                    Script.write(it)
                } ?: continue
            // The explorer's estimate can round to 0; 1 sat/vB is the floor the sweep pays.
            val feeRate = (feeRateSatVb() ?: continue).coerceAtLeast(1)
            val key = keys.keys[output.keyIndex]
            val tx =
                ToRemoteSweep.build(
                    key,
                    output.txid,
                    output.vout,
                    output.valueSats,
                    script,
                    feeRate,
                )
            if (tx == null) {
                // Waits for lower fees; dropped only when even 1 sat/vB would leave dust.
                val sweepable =
                    ToRemoteSweep.build(key, output.txid, output.vout, output.valueSats, script, 1)
                if (sweepable == null) {
                    log(
                        "TO_REMOTE_DUST",
                        mapOf(
                            "outpoint" to "${output.txid}:${output.vout}",
                            "sats" to output.valueSats,
                        ),
                    )
                    done += output
                }
                continue
            }
            if (state.generation != generation) return
            val result = broadcast(ByteVector(Transaction.write(tx)).toHex())
            log(
                "TO_REMOTE_SWEEP_BROADCAST",
                mapOf(
                    "outpoint" to "${output.txid}:${output.vout}",
                    "result" to result,
                    "sats" to output.valueSats,
                    "fee_rate" to feeRate,
                ),
            )
            if (!result.startsWith("error:")) {
                onSwept(tx.txid.toString(), tx.txOut.single().amount.sat, address)
            }
        }
        if (done.isEmpty()) return
        update { s ->
            if (s.generation != generation) return@update s
            val left = s.found.filterNot { it in done }
            s.copy(found = left, destination = if (left.isEmpty()) null else s.destination)
        }
        if (!state.active) log("TO_REMOTE_RECOVERY_END", mapOf("found" to true))
    }

    private fun isSpentAndConfirmed(spend: JSONObject?) =
        spend != null &&
            spend.optBoolean("spent") &&
            spend.optJSONObject("status")?.optBoolean("confirmed") == true

    @Synchronized
    private fun update(change: (State) -> State) {
        state = change(state)
        saveState(json.encodeToString(state))
    }

    /**
     * The reply, or with [emptyIfMissing] "[]" when an explorer has no such path and none replied.
     */
    private fun get(path: String, emptyIfMissing: Boolean = false): String? {
        var missing = false
        for (base in chainUrls()) {
            try {
                val request = Request.Builder().url(base.trimEnd('/') + path).build()
                http.newCall(request).execute().use { response ->
                    if (response.isSuccessful) return response.body?.string()
                    if (response.code == 404) missing = true
                }
            } catch (_: Exception) {
                /* try next */
            }
        }
        return if (missing && emptyIfMissing) "[]" else null
    }

    private fun getObject(path: String) =
        get(path)?.let { runCatching { JSONObject(it) }.getOrNull() }

    private fun getArray(path: String, emptyIfMissing: Boolean = false) =
        get(path, emptyIfMissing)?.let { runCatching { JSONArray(it) }.getOrNull() }

    /** Returns the txid, or the last error when no explorer accepted the transaction. */
    private fun broadcast(hex: String): String {
        var error = "unreachable"
        for (base in chainUrls()) {
            try {
                val body = hex.toRequestBody("text/plain".toMediaType())
                val request = Request.Builder().url(base.trimEnd('/') + "/tx").post(body).build()
                http.newCall(request).execute().use { response ->
                    val text = response.body?.string().orEmpty()
                    if (response.isSuccessful) return text.trim()
                    error = "${response.code} ${text.take(200)}"
                }
            } catch (e: Exception) {
                error = e.message ?: e.javaClass.simpleName
            }
        }
        return "error: $error"
    }

    companion object {
        /** How long to keep looking for the LSP's close. */
        const val SEARCH_MS = 14 * 24 * 60 * 60 * 1000L
        const val SETTLE_INTERVAL_MS = 60 * 1000L

        private fun isTwoOfTwo(script: String) =
            script.length == 142 &&
                script.startsWith("5221") &&
                script.substring(70, 72) == "21" &&
                script.endsWith("52ae")

        /**
         * 1 minute, as the LSP closes within seconds of a reconnect, then 10 minutes doubling up to
         * a day for an LSP that is slow or offline.
         */
        fun backoffMs(attempts: Int): Long =
            if (attempts <= 1) 60 * 1000L
            else minOf(24 * 60 * 60 * 1000L, (10 * 60 * 1000L) shl (attempts - 2).coerceIn(0, 10))
    }
}
