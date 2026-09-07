package com.stablechannels.app.services

import android.util.Log
import okhttp3.OkHttpClient
import okhttp3.Request

/** Outcome of asking esplora whether it has ever heard of a txid. */
enum class TxBroadcastStatus { EXISTS, NOT_FOUND, INCONCLUSIVE }

/**
 * Checks whether esplora has ever heard of a txid (broadcast, mempool, or confirmed) — used to
 * tell a genuinely abandoned/never-broadcast splice tx apart from a stale failure event for a
 * splice that did make it on-chain.
 *
 * The result is deliberately tri-state rather than a boolean:
 * - EXISTS: any endpoint returned a successful (2xx) response for the tx.
 * - NOT_FOUND: every reachable endpoint returned an explicit 404 across [retries] consecutive
 *   rounds. A single round of 404s isn't proof — the tx may still be propagating right after
 *   negotiation — so this is only concluded once the same negative result is confirmed on retry.
 * - INCONCLUSIVE: anything else (timeouts, exceptions, 429/5xx, or a mix of endpoint results). We
 *   cannot prove the tx doesn't exist here, so callers must never treat this as a real failure —
 *   the safer failure mode is treating a real failure as a stale replay (recoverable manually)
 *   rather than mislabeling a possibly-real splice as failed.
 */
class SpliceBroadcastChecker(
    private val httpClient: OkHttpClient,
    private val retries: Int = 3,
    private val retryDelayMs: Long = 2_000L,
    private val sleep: (Long) -> Unit = { Thread.sleep(it) },
    private val logWarning: (String) -> Unit = { Log.w("SpliceBroadcastChecker", it) }
) {
    fun checkStatus(txid: String, endpointUrls: List<String>): TxBroadcastStatus {
        val normalizedTxid = txid.substringBefore(":")
        val urls = endpointUrls.filter { it.isNotBlank() }.distinct()
        if (urls.isEmpty()) return TxBroadcastStatus.INCONCLUSIVE

        repeat(retries) { attempt ->
            var allNotFound = true
            var anyReached = false
            for (baseUrl in urls) {
                try {
                    val request = Request.Builder()
                        .url("${baseUrl.trimEnd('/')}/tx/$normalizedTxid/status")
                        .build()
                    httpClient.newCall(request).execute().use { response ->
                        anyReached = true
                        if (response.isSuccessful) return TxBroadcastStatus.EXISTS
                        if (response.code != 404) allNotFound = false
                    }
                } catch (e: Exception) {
                    logWarning("Existence check failed: ${e.message}")
                    allNotFound = false
                }
            }
            if (!anyReached || !allNotFound) return TxBroadcastStatus.INCONCLUSIVE
            if (attempt < retries - 1) sleep(retryDelayMs)
        }
        return TxBroadcastStatus.NOT_FOUND
    }
}
