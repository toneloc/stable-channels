package com.stablechannels.app.services.websocket

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TransactionMatcherTest {

    private val matcher = TransactionMatcher()

    @Test
    fun `matches tracked address from vout`() {
        val trackedAddress = "bc1qtestaddress123"
        val tx = MempoolWSTransaction(
            txid = validTxid('a'),
            vout = listOf(MempoolWSVout(scriptpubkeyAddress = trackedAddress, value = 50_000))
        )
        val msg = MempoolWSMessage(addressTransactions = listOf(tx))

        val matches = matcher.matchAll(
            trackedAddresses = setOf(trackedAddress),
            trackedTxids = emptySet(),
            msg = msg,
            tx = tx
        )

        assertEquals(1, matches.size)
        assertEquals(MatchResult(target = trackedAddress, isTxid = false), matches.first())
    }

    @Test
    fun `matches tracked txid from vin`() {
        val trackedTxid = validTxid('b')
        val tx = MempoolWSTransaction(
            txid = validTxid('c'),
            vin = listOf(MempoolWSVin(txid = trackedTxid))
        )
        val msg = MempoolWSMessage(addressTransactions = listOf(tx))

        val matches = matcher.matchAll(
            trackedAddresses = emptySet(),
            trackedTxids = setOf(trackedTxid),
            msg = msg,
            tx = tx
        )

        assertEquals(1, matches.size)
        assertEquals(MatchResult(target = trackedTxid, isTxid = true), matches.first())
    }

    @Test
    fun `returns empty list when nothing matches`() {
        val tx = MempoolWSTransaction(txid = validTxid('d'))
        val msg = MempoolWSMessage(addressTransactions = listOf(tx))

        val matches = matcher.matchAll(
            trackedAddresses = setOf("bc1qother"),
            trackedTxids = setOf(validTxid('e')),
            msg = msg,
            tx = tx
        )

        assertTrue(matches.isEmpty())
    }

    @Test
    fun `matches tracked address from message address field`() {
        val trackedAddress = "bc1qdirectaddress"
        val tx = MempoolWSTransaction(txid = validTxid('f'))
        val msg = MempoolWSMessage(address = trackedAddress, addressTransactions = listOf(tx))

        val matches = matcher.matchAll(
            trackedAddresses = setOf(trackedAddress),
            trackedTxids = emptySet(),
            msg = msg,
            tx = tx
        )

        assertEquals(listOf(MatchResult(target = trackedAddress, isTxid = false)), matches)
    }

    @Test
    fun `matches tracked txid from message txid field`() {
        val trackedTxid = validTxid('1')
        val tx = MempoolWSTransaction(txid = validTxid('2'))
        val msg = MempoolWSMessage(txid = trackedTxid)

        val matches = matcher.matchAll(
            trackedAddresses = emptySet(),
            trackedTxids = setOf(trackedTxid),
            msg = msg,
            tx = tx
        )

        assertEquals(listOf(MatchResult(target = trackedTxid, isTxid = true)), matches)
    }

    @Test
    fun `multi address payload matches tracked address when tx appears in mempool group`() {
        val trackedAddress = "bc1qmulti"
        val tx = MempoolWSTransaction(txid = validTxid('3'))
        val msg = MempoolWSMessage(
            multiAddressTransactions = mapOf(
                trackedAddress to MempoolWSAddressTransactions(mempool = listOf(tx))
            )
        )

        val matches = matcher.matchAll(
            trackedAddresses = setOf(trackedAddress),
            trackedTxids = emptySet(),
            msg = msg,
            tx = tx
        )

        assertEquals(listOf(MatchResult(target = trackedAddress, isTxid = false)), matches)
    }

    @Test
    fun `address in message is prioritized before vout match`() {
        val directAddress = "bc1qdirectpriority"
        val voutAddress = "bc1qvoutpriority"
        val tx = MempoolWSTransaction(
            txid = validTxid('4'),
            vout = listOf(MempoolWSVout(scriptpubkeyAddress = voutAddress, value = 1000))
        )
        val msg = MempoolWSMessage(address = directAddress)

        val matches = matcher.matchAll(
            trackedAddresses = setOf(directAddress, voutAddress),
            trackedTxids = emptySet(),
            msg = msg,
            tx = tx
        )

        assertEquals(directAddress, matches.first().target)
    }

    @Test
    fun `dedups repeated address match from message and vout`() {
        val trackedAddress = "bc1qdedupresult"
        val tx = MempoolWSTransaction(
            txid = validTxid('5'),
            vout = listOf(MempoolWSVout(scriptpubkeyAddress = trackedAddress, value = 500))
        )
        val msg = MempoolWSMessage(address = trackedAddress)

        val matches = matcher.matchAll(
            trackedAddresses = setOf(trackedAddress),
            trackedTxids = emptySet(),
            msg = msg,
            tx = tx
        )

        assertEquals(1, matches.size)
        assertEquals(MatchResult(target = trackedAddress, isTxid = false), matches.first())
    }

    private fun validTxid(char: Char): String = char.toString().repeat(64)
}
