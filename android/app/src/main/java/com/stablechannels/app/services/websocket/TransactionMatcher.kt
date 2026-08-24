package com.stablechannels.app.services.websocket

data class MatchResult(val target: String, val isTxid: Boolean)

class TransactionMatcher {
    fun matchAll(
        trackedAddresses: Set<String>,
        trackedTxids: Set<String>,
        msg: MempoolWSMessage,
        tx: MempoolWSTransaction
    ): List<MatchResult> {
        val results = mutableListOf<MatchResult>()

        if (!msg.address.isNullOrBlank() && trackedAddresses.contains(msg.address)) {
            results.add(MatchResult(target = msg.address, isTxid = false))
        }

        tx.vout?.forEach { vout ->
            val addr = vout.scriptpubkeyAddress
            if (!addr.isNullOrBlank() && trackedAddresses.contains(addr)) {
                val res = MatchResult(target = addr, isTxid = false)
                if (!results.contains(res)) {
                    results.add(res)
                }
            }
        }

        tx.vin?.forEach { vin ->
            val inputTxid = vin.txid
            if (!inputTxid.isNullOrBlank() && trackedTxids.contains(inputTxid)) {
                val res = MatchResult(target = inputTxid, isTxid = true)
                if (!results.contains(res)) {
                    results.add(res)
                }
            }
        }

        if (!msg.txid.isNullOrBlank() && trackedTxids.contains(msg.txid)) {
            val res = MatchResult(target = msg.txid, isTxid = true)
            if (!results.contains(res)) {
                results.add(res)
            }
        }

        msg.multiAddressTransactions?.forEach { (addr, txGroup) ->
            if (!trackedAddresses.contains(addr)) {
                return@forEach
            }

            val inMempool = txGroup.mempool?.any { it.txid == tx.txid } == true
            val inConfirmed = txGroup.confirmed?.any { it.txid == tx.txid } == true
            val inRemoved = txGroup.removed?.any { it.txid == tx.txid } == true

            if (inMempool || inConfirmed || inRemoved) {
                val res = MatchResult(target = addr, isTxid = false)
                if (!results.contains(res)) {
                    results.add(res)
                }
            }
        }

        return results
    }
}
