package com.stablechannels.app.services

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MissedReceiveRecoveryTest {
    private val address = "bc1qme"
    private val now = 1_800_000_000L

    private fun tx(txid: String, vouts: List<Pair<String, Long>>, blockTime: Long?): String {
        val outs =
            vouts.joinToString(",") {
                """{"scriptpubkey_address":"${it.first}","value":${it.second}}"""
            }
        val status =
            if (blockTime == null) """{"confirmed":false}"""
            else """{"confirmed":true,"block_time":$blockTime}"""
        return """{"txid":"$txid","vout":[$outs],"status":$status}"""
    }

    @Test
    fun sumsOnlyOutputsPayingTheAddress() {
        val json = "[" + tx("a", listOf(address to 11_732L, "other" to 277_521L), now - 60) + "]"

        assertEquals(
            listOf(MissedReceiveRecovery.Receive("a", 11_732L, now - 60)),
            MissedReceiveRecovery.recentReceives(json, address, now),
        )
    }

    @Test
    fun unconfirmedCountsAsNow() {
        val json = "[" + tx("a", listOf(address to 5_000L), null) + "]"

        assertEquals(
            now,
            MissedReceiveRecovery.recentReceives(json, address, now).single().timeSecs,
        )
    }

    @Test
    fun dropsOldDustAndUnrelatedTxs() {
        val old = now - MissedReceiveRecovery.WINDOW_SECS - 1
        val json =
            "[" +
                listOf(
                        tx("old", listOf(address to 5_000L), old),
                        tx("dust", listOf(address to 999L), now - 60),
                        tx("other", listOf("someone" to 5_000L), now - 60),
                    )
                    .joinToString(",") +
                "]"

        assertTrue(MissedReceiveRecovery.recentReceives(json, address, now).isEmpty())
    }
}
