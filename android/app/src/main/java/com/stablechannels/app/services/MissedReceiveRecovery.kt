package com.stablechannels.app.services

import org.json.JSONArray
import org.json.JSONObject

/**
 * Finds deposits to our receive address that the websocket and balance-delta paths both missed
 * (socket down, or detection deferred across an app restart). Parsing and filtering are pure.
 */
object MissedReceiveRecovery {
    const val WINDOW_SECS = 24 * 60 * 60L
    private const val MIN_SATS = 1000L

    data class Receive(val txid: String, val sats: Long, val timeSecs: Long)

    /** Esplora `/address/{addr}/txs`: sum of outputs paying [address] per tx, within the window. */
    fun recentReceives(json: String, address: String, nowSecs: Long): List<Receive> {
        val txs = JSONArray(json)
        return buildList {
            for (i in 0 until txs.length()) {
                val tx = txs.getJSONObject(i)
                val txid = tx.optString("txid")
                if (txid.isBlank()) continue
                val sats = satsPaidTo(tx, address)
                val time = tx.optJSONObject("status")?.optLong("block_time", 0L) ?: 0L
                val receivedAt = if (time > 0) time else nowSecs
                if (sats >= MIN_SATS && nowSecs - receivedAt <= WINDOW_SECS) {
                    add(Receive(txid, sats, receivedAt))
                }
            }
        }
    }

    /** Sum of the outputs of an Esplora transaction that pay [address]. */
    fun satsPaidTo(tx: JSONObject, address: String): Long {
        val vouts = tx.optJSONArray("vout") ?: return 0L
        var sats = 0L
        for (j in 0 until vouts.length()) {
            val out = vouts.getJSONObject(j)
            if (out.optString("scriptpubkey_address") == address) {
                sats += out.optLong("value")
            }
        }
        return sats
    }
}
