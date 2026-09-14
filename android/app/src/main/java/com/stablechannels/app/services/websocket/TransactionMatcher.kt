package com.stablechannels.app.services.websocket

data class MatchResult(val target: String, val isTxid: Boolean)

class TransactionMatcher {
    fun matchAll(
        trackedAddresses: Set<String>,
        trackedTxids: Set<String>,
        msg: MempoolWSMessage,
        tx: MempoolWSTransaction,
    ): List<MatchResult> {
        val results = LinkedHashSet<MatchResult>()

        if (!msg.address.isNullOrBlank() && trackedAddresses.contains(msg.address)) {
            results.add(MatchResult(target = msg.address, isTxid = false))
        }

        tx.vout?.forEach { vout ->
            val addr = vout.scriptpubkeyAddress
            if (!addr.isNullOrBlank() && trackedAddresses.contains(addr)) {
                results.add(MatchResult(target = addr, isTxid = false))
            }
        }

        tx.vin?.forEach { vin ->
            val inputTxid = vin.txid
            if (!inputTxid.isNullOrBlank() && trackedTxids.contains(inputTxid)) {
                results.add(MatchResult(target = inputTxid, isTxid = true))
            }
        }

        if (!msg.txid.isNullOrBlank() && trackedTxids.contains(msg.txid)) {
            results.add(MatchResult(target = msg.txid, isTxid = true))
        }

        msg.multiAddressTransactions?.forEach { (addr, txGroup) ->
            if (!trackedAddresses.contains(addr)) {
                return@forEach
            }

            val inMempool = txGroup.mempool?.any { it.txid == tx.txid } == true
            val inConfirmed = txGroup.confirmed?.any { it.txid == tx.txid } == true
            val inRemoved = txGroup.removed?.any { it.txid == tx.txid } == true

            if (inMempool || inConfirmed || inRemoved) {
                results.add(MatchResult(target = addr, isTxid = false))
            }
        }

        return results.toList()
    }
}
