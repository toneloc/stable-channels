package com.stablechannels.app.services

import fr.acinq.bitcoin.Bitcoin
import fr.acinq.bitcoin.ByteVector
import fr.acinq.bitcoin.Chain
import fr.acinq.bitcoin.Crypto
import fr.acinq.bitcoin.Script
import fr.acinq.bitcoin.Transaction
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
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
 * recovery script does by hand, and finds the close itself in one of two ways:
 * - by a funding txid saved before the loss: what spent the funding output is the close, and its
 *   outputs are matched against our keys (a few requests),
 * - after a restore from seed, by looking up the scripts of all 1000 keys.
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
) {
    @Serializable
    data class Output(val txid: String, val vout: Int, val valueSats: Long, val keyIndex: Int)

    @Serializable
    data class State(
        val fundingTxid: String? = null,
        val fundingVout: Int? = null,
        val scan: Boolean = false,
        /** When the scan began: an output confirmed well before is an older close. */
        val scanFrom: Long = 0,
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
            get() = fundingTxid != null || scan || found.isNotEmpty()
    }

    private class Scan(val outputs: List<Output>, val newClose: Boolean, val complete: Boolean)

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
            start(savedFundingTxid, savedFundingVout, scan = false)
        }
        return state.active
    }

    /**
     * Starts looking for a close LDK doesn't track: by [fundingTxid] when known, and by scanning
     * all keys when [scan]. With [repeat] false it searches once, for a close that already
     * happened.
     */
    fun start(fundingTxid: String?, fundingVout: Int?, scan: Boolean, repeat: Boolean = true) {
        val time = now()
        val until = if (repeat) time + SEARCH_MS else time
        update {
            it.copy(
                fundingTxid = fundingTxid ?: it.fundingTxid,
                fundingVout = if (fundingTxid != null) fundingVout else it.fundingVout,
                scan = scan || it.scan,
                scanFrom = if (scan) time else it.scanFrom,
                until = maxOf(until, it.until),
                attempts = 0,
                nextAt = 0,
                generation = it.generation + 1,
            )
        }
        log(
            "TO_REMOTE_RECOVERY_START",
            mapOf("funding_txid" to fundingTxid, "scan" to scan, "repeat" to repeat),
        )
    }

    /**
     * After a restore from seed: starts over, as found outputs and the sweep address may belong to
     * the wallet that was replaced, then scans all keys.
     */
    fun startAfterRestore(fundingTxid: String?, fundingVout: Int?, repeat: Boolean) {
        update { State(generation = it.generation + 1) }
        start(fundingTxid, fundingVout, scan = true, repeat = repeat)
    }

    /**
     * Does whatever is due. Cheap to call often; returns at once while another run is going. Closes
     * of channels in [liveFundingTxids] are left to LDK.
     */
    suspend fun run(
        keys: () -> ToRemoteKeys?,
        destination: () -> String?,
        feeRateSatVb: () -> Long?,
        liveFundingTxids: Set<String> = emptySet(),
    ) {
        if (!state.active || !running.tryLock()) return
        try {
            withContext(Dispatchers.IO) {
                val k = keys() ?: return@withContext
                if (state.found.isNotEmpty()) settle(k, destination, feeRateSatVb)
                if (state.fundingTxid != null || state.scan) search(k, liveFundingTxids)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // A malformed explorer reply must not crash the app on every refresh.
            log("TO_REMOTE_ERROR", mapOf("error" to (e.message ?: e.javaClass.simpleName)))
        } finally {
            running.unlock()
        }
    }

    private suspend fun search(keys: ToRemoteKeys, liveFundingTxids: Set<String>) {
        val s = state
        if (now() < s.nextAt) return
        val found = mutableListOf<Output>()
        var fundingSpent = false
        if (s.fundingTxid != null) {
            findByFunding(s.fundingTxid, s.fundingVout, keys)?.let { (outputs, spent) ->
                found += outputs
                fundingSpent = spent
            }
        }
        val scan = if (s.scan) scan(keys, liveFundingTxids, s.scanFrom) else null
        found += scan?.outputs.orEmpty()
        val time = now()
        val pastDeadline = time >= s.until
        val fundingEnds = s.fundingTxid != null && (fundingSpent || pastDeadline)
        // An older close found on the keys doesn't end the wait for the new one. Past the deadline
        // the scan ends once a full scan found nothing new, or two weeks later anyway.
        val scanEnds =
            scan != null &&
                (scan.newClose || pastDeadline && (scan.complete || time >= s.until + SEARCH_MS))
        val added = found.filter { o -> s.found.none { it.txid == o.txid && it.vout == o.vout } }
        update {
            if (it.generation != s.generation) return@update it
            it.copy(
                found = (it.found + added).distinctBy { o -> o.txid to o.vout },
                fundingTxid = if (fundingEnds) null else it.fundingTxid,
                checkedFundingTxid = if (fundingEnds) s.fundingTxid else it.checkedFundingTxid,
                scan = it.scan && !scanEnds,
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
        val spends = getArray("/tx/$txid/outspends") ?: return null
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

    /**
     * Looks up the script of every key. An output whose close is unconfirmed or confirmed since
     * [scanFrom] counts as the new close. Closes of channels LDK still lists are left out.
     */
    private suspend fun scan(
        keys: ToRemoteKeys,
        liveFundingTxids: Set<String>,
        scanFrom: Long,
    ): Scan = coroutineScope {
        val limit = Semaphore(SCAN_PARALLEL)
        val results =
            keys.keys
                .map { key ->
                    async {
                        limit.withPermit {
                            // Esplora's scripthash is the plain SHA-256 of the scriptPubKey.
                            val hash = ByteVector(Crypto.sha256(key.scriptPubKey)).toHex()
                            getArray("/scripthash/$hash/utxo")?.let { utxos ->
                                (0 until utxos.length()).map {
                                    val utxo = utxos.getJSONObject(it)
                                    val status = utxo.optJSONObject("status")
                                    val confirmedAt =
                                        if (status?.optBoolean("confirmed") == true)
                                            status.optLong("block_time") * 1000
                                        else null
                                    Output(
                                        utxo.getString("txid"),
                                        utxo.getInt("vout"),
                                        utxo.getLong("value"),
                                        key.index,
                                    ) to confirmedAt
                                }
                            }
                        }
                    }
                }
                .awaitAll()
        var complete = results.none { it == null }
        val outputs =
            results.filterNotNull().flatten().filter { (output, _) ->
                if (liveFundingTxids.isEmpty()) return@filter true
                val inputs = getObject("/tx/${output.txid}")?.optJSONArray("vin")
                if (inputs == null) {
                    complete = false
                    return@filter false
                }
                (0 until inputs.length()).none {
                    inputs.getJSONObject(it).optString("txid") in liveFundingTxids
                }
            }
        Scan(
            outputs = outputs.map { it.first },
            newClose = outputs.any { (_, at) -> at == null || at >= scanFrom - CLOCK_SLACK_MS },
            complete = complete,
        )
    }

    private fun settle(
        keys: ToRemoteKeys,
        destination: () -> String?,
        feeRateSatVb: () -> Long?,
    ) {
        if (now() - lastSettleAt < SETTLE_INTERVAL_MS) return
        lastSettleAt = now()
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
            val address = state.destination ?: destination() ?: continue
            if (state.destination == null) update { it.copy(destination = address) }
            val script =
                Bitcoin.addressToPublicKeyScript(Chain.Mainnet.chainHash, address).right?.let {
                    Script.write(it)
                } ?: continue
            val feeRate = feeRateSatVb() ?: continue
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
            log(
                "TO_REMOTE_SWEEP_BROADCAST",
                mapOf(
                    "outpoint" to "${output.txid}:${output.vout}",
                    "result" to broadcast(ByteVector(Transaction.write(tx)).toHex()),
                    "sats" to output.valueSats,
                    "fee_rate" to feeRate,
                ),
            )
        }
        if (done.isEmpty()) return
        update { s ->
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

    private fun get(path: String): String? {
        for (base in chainUrls()) {
            try {
                val request = Request.Builder().url(base.trimEnd('/') + path).build()
                http.newCall(request).execute().use { response ->
                    if (response.isSuccessful) return response.body?.string()
                }
            } catch (_: Exception) {
                /* try next */
            }
        }
        return null
    }

    private fun getObject(path: String) =
        get(path)?.let { runCatching { JSONObject(it) }.getOrNull() }

    private fun getArray(path: String) =
        get(path)?.let { runCatching { JSONArray(it) }.getOrNull() }

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
        const val SCAN_PARALLEL = 4
        /**
         * Device and block clocks can differ; a close this close to the scan start counts as new.
         */
        const val CLOCK_SLACK_MS = 2 * 60 * 60 * 1000L

        private fun isTwoOfTwo(script: String) =
            script.length == 142 &&
                script.startsWith("5221") &&
                script.substring(70, 72) == "21" &&
                script.endsWith("52ae")

        /** 10 minutes, doubling up to a day: the LSP's close usually shows up within hours. */
        fun backoffMs(attempts: Int): Long =
            minOf(24 * 60 * 60 * 1000L, (10 * 60 * 1000L) shl (attempts - 1).coerceIn(0, 10))
    }
}
