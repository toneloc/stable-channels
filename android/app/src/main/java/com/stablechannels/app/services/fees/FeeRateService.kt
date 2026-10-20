package com.stablechannels.app.services.fees

import com.stablechannels.app.domain.send.RecommendedFees
import com.stablechannels.app.util.Constants
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.runInterruptible
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import okhttp3.OkHttpClient
import okhttp3.Request

/** One HTTP provider of recommended fee rates. */
fun interface FeeRateSource {
    suspend fun fetch(): RecommendedFees
}

private val sourceClient = OkHttpClient.Builder().callTimeout(5, TimeUnit.SECONDS).build()

private suspend fun fetchJson(client: OkHttpClient, url: String): JsonObject =
    runInterruptible(Dispatchers.IO) {
        client.newCall(Request.Builder().url(url).build()).execute().use { response ->
            if (response.code != 200) throw IOException("HTTP ${response.code} from $url")
            Json.parseToJsonElement(response.body?.string().orEmpty()).jsonObject
        }
    }

/** A JSON number under [key]; strings, objects and missing keys give null. */
private fun JsonObject.rate(key: String): Double? =
    (this[key] as? JsonPrimitive)?.takeUnless { it.isString }?.doubleOrNull

/** Esplora `/fee-estimates`: confirmation target in blocks mapped to sat/vB. */
class EsploraFeeSource(
    private val baseUrl: String = Constants.PRIMARY_CHAIN_URL,
    private val client: OkHttpClient = sourceClient,
) : FeeRateSource {
    override suspend fun fetch(): RecommendedFees {
        val json = fetchJson(client, "$baseUrl/fee-estimates")
        val fastest = json.rate("1") ?: json.rate("2")
        val halfHour = json.rate("3") ?: json.rate("4")
        val hour = json.rate("6")
        if (fastest == null || halfHour == null || hour == null)
            throw IOException("Unexpected fee-estimates body")
        val minimum =
            json.rate("144")
                ?: json.rate("25")
                ?: json.rate("1008")
                ?: Constants.MIN_FEE_RATE_SAT_VB
        return RecommendedFees(fastest, halfHour, hour, minimumFee = minimum)
    }
}

/**
 * Mempool `/v1/fees/recommended`: `fastestFee`, `halfHourFee`, `hourFee`, `economyFee`,
 * `minimumFee`.
 */
class MempoolFeeSource(
    private val baseUrl: String = Constants.FALLBACK_CHAIN_URL,
    private val client: OkHttpClient = sourceClient,
) : FeeRateSource {
    override suspend fun fetch(): RecommendedFees {
        val json = fetchJson(client, "$baseUrl/v1/fees/recommended")
        val fastest = json.rate("fastestFee")
        val halfHour = json.rate("halfHourFee")
        val hour = json.rate("hourFee")
        val minimum = json.rate("minimumFee")
        if (fastest == null || halfHour == null || hour == null || minimum == null) {
            throw IOException("Unexpected fees/recommended body")
        }
        return RecommendedFees(fastest, halfHour, hour, json.rate("economyFee"), minimum)
    }
}

/**
 * Recommended fees from the first HTTP source to answer, cached for [cacheTtlMillis].
 *
 * Concurrent callers share one fetch. Rates pushed with [update] (the mempool websocket) win over a
 * fetch that was already running. When every source fails, the last good rates are kept; without
 * any, rates derived from [fallbackRateSatVb] are returned but not cached.
 */
class FeeRateService(
    private val sources: List<FeeRateSource> = listOf(EsploraFeeSource(), MempoolFeeSource()),
    private val cacheTtlMillis: Long = 60_000,
    private val fallbackRateSatVb: Double = 2.0,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO),
    private val nowMillis: () -> Long = System::currentTimeMillis,
) {
    private val mutex = Mutex()
    private var cached: RecommendedFees? = null
    private var cachedAtMillis = 0L
    private var generation = 0L
    private var inFlight: Deferred<RecommendedFees?>? = null

    suspend fun recommendedFees(): RecommendedFees {
        val (task, generationBefore) =
            mutex.withLock {
                fresh()?.let {
                    return it
                }
                // A completed fetch nobody collected (its caller was cancelled) is stale; start
                // anew.
                val task =
                    inFlight?.takeUnless { it.isCompleted }
                        ?: scope.async { fetchFirstSuccess() }.also { inFlight = it }
                task to generation
            }
        val fetched = task.await()
        return mutex.withLock {
            if (inFlight === task) inFlight = null
            // Updated or invalidated while waiting: never overwrite newer rates with this fetch.
            if (generation != generationBefore) return cached ?: fetched ?: fallbackFees()
            fresh()?.let {
                return it
            }
            if (fetched != null) {
                store(fetched)
                fetched
            } else {
                cached ?: fallbackFees()
            }
        }
    }

    /** Stores rates pushed by the mempool websocket. */
    suspend fun update(fees: RecommendedFees) {
        mutex.withLock { store(fees) }
    }

    suspend fun invalidate() {
        mutex.withLock {
            generation++
            cached = null
        }
    }

    private fun store(fees: RecommendedFees) {
        generation++
        cached = fees
        cachedAtMillis = nowMillis()
    }

    private fun fresh(): RecommendedFees? = cached?.takeIf {
        nowMillis() - cachedAtMillis < cacheTtlMillis
    }

    private suspend fun fetchFirstSuccess(): RecommendedFees? = coroutineScope {
        val results = Channel<RecommendedFees?>(sources.size)
        val jobs = sources.map { source ->
            launch { results.send(runCatching { source.fetch() }.getOrNull()) }
        }
        var winner: RecommendedFees? = null
        repeat(sources.size) { if (winner == null) winner = results.receive() }
        jobs.forEach { it.cancel() }
        winner
    }

    private fun fallbackFees() =
        RecommendedFees(
            fastestFee = fallbackRateSatVb * 1.3,
            halfHourFee = fallbackRateSatVb,
            hourFee = fallbackRateSatVb * 0.8,
            minimumFee = fallbackRateSatVb * 0.5,
        )
}
