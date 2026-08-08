package com.stablechannels.app.services.websocket

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ProcessedTxStoreTest {

    @Test
    fun `records key and marks as recently processed`() {
        val store = ProcessedTxStore(ttlMs = 60_000L, maxEntries = 500)

        store.recordProcessedTx("tx_a")

        assertTrue(store.isRecentlyProcessed("tx_a"))
    }

    @Test
    fun `expires key after ttl`() {
        val store = ProcessedTxStore(ttlMs = 10L, maxEntries = 500)

        store.recordProcessedTx("tx_b")
        Thread.sleep(20)

        assertFalse(store.isRecentlyProcessed("tx_b"))
    }

    @Test
    fun `evicts oldest when exceeding cap`() {
        val store = ProcessedTxStore(ttlMs = 60_000L, maxEntries = 10)

        repeat(15) { idx ->
            store.recordProcessedTx("tx_$idx")
        }

        assertTrue(store.count() <= 10)
    }
}
