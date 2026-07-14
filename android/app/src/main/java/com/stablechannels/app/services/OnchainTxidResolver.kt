package com.stablechannels.app.services

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONArray
import java.util.concurrent.TimeUnit

object OnchainTxidResolver {

    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    /**
     * Polls the block explorer for a transaction hitting the specified address.
     * Searches both mempool and chain endpoints. Retries with exponential backoff.
     */
    suspend fun resolve(address: String, chainUrl: String): String? {
        return withContext(Dispatchers.IO) {
            val backoffs = listOf(2L, 8L, 30L, 60L, 120L, 300L)
            
            for (delaySecs in backoffs) {
                delay(delaySecs * 1000)
                
                val baseUrl = chainUrl.trimEnd('/')
                val endpoints = listOf(
                    "$baseUrl/address/$address/txs/chain",
                    "$baseUrl/address/$address/txs/mempool"
                )
                
                for (url in endpoints) {
                    try {
                        val request = Request.Builder().url(url).build()
                        val response = httpClient.newCall(request).execute()
                        
                        if (response.isSuccessful) {
                            val bodyStr = response.body?.string()
                            if (!bodyStr.isNullOrBlank()) {
                                val jsonArray = JSONArray(bodyStr)
                                if (jsonArray.length() > 0) {
                                    val firstTx = jsonArray.getJSONObject(0)
                                    val txid = firstTx.optString("txid")
                                    if (txid.isNotBlank()) {
                                        return@withContext txid
                                    }
                                }
                            }
                        }
                    } catch (e: Exception) {
                        // Ignore network/parsing errors, try next endpoint/iteration
                    }
                }
            }
            null
        }
    }
}
