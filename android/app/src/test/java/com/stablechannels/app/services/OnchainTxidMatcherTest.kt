package com.stablechannels.app.services

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class OnchainTxidMatcherTest {

    @Test
    fun `single unambiguous edge is resolved`() {
        val result = OnchainTxidMatcher.resolve(listOf(TxidMatchEdge(1L, "tx1", 5L)))
        assertEquals(mapOf(1L to "tx1"), result)
    }

    @Test
    fun `greedy cardinality counterexample resolves both rows correctly`() {
        // R1 is compatible with C1 (very close) and C2 (far); R2 is compatible only with C1.
        // Naive greedy-by-proximity would award C1 to R1 (lowest cost edge overall) and
        // permanently strand R2, even though R1->C2, R2->C1 satisfies both. This is the exact
        // scenario cited in review as breaking amount+timestamp greedy matching.
        val edges = listOf(
            TxidMatchEdge(rowId = 1L, txid = "C1", costSecs = 1L),
            TxidMatchEdge(rowId = 1L, txid = "C2", costSecs = 86_400L),
            TxidMatchEdge(rowId = 2L, txid = "C1", costSecs = 86_400L)
        )

        val result = OnchainTxidMatcher.resolve(edges)

        assertEquals(mapOf(1L to "C2", 2L to "C1"), result)
    }

    @Test
    fun `two rows with two equal-cost candidates each is left unresolved`() {
        // Both rows are equally compatible with both candidates at the same cost — there is no
        // way to determine which txid belongs to which row, so nothing should be assigned.
        val edges = listOf(
            TxidMatchEdge(1L, "C1", 10L),
            TxidMatchEdge(1L, "C2", 10L),
            TxidMatchEdge(2L, "C1", 10L),
            TxidMatchEdge(2L, "C2", 10L)
        )

        val result = OnchainTxidMatcher.resolve(edges)

        assertTrue(result.isEmpty())
    }

    @Test
    fun `candidate goes to the closer-cost row, not the delayed one competing for it`() {
        // Row 1's real deposit genuinely is C1 (small delta). Row 2 is a newer/delayed row whose
        // true LDK payment hasn't been indexed yet, so at match time its only visible candidate
        // is the same C1, at a much larger delta. There's no ambiguity here — C1 unique-costs to
        // row 1 — so the matcher must award it there and leave row 2 unresolved (to be retried
        // once its real candidate actually appears), not accidentally hand C1 to the delayed row.
        val edges = listOf(
            TxidMatchEdge(rowId = 1L, txid = "C1", costSecs = 5L),
            TxidMatchEdge(rowId = 2L, txid = "C1", costSecs = 50_000L)
        )

        val result = OnchainTxidMatcher.resolve(edges)

        assertEquals(mapOf(1L to "C1"), result)
    }

    @Test
    fun `a row that is the only claimant of a shared-looking candidate resolves via propagation`() {
        // Three rows, three candidates, but only row 3 has a single option (C3). Assigning row 3
        // to C3 doesn't free anything else up here, but demonstrates propagation still holds when
        // an unrelated row/candidate pair is fully ambiguous alongside it.
        val edges = listOf(
            TxidMatchEdge(1L, "C1", 1L),
            TxidMatchEdge(1L, "C2", 1L),
            TxidMatchEdge(2L, "C1", 1L),
            TxidMatchEdge(2L, "C2", 1L),
            TxidMatchEdge(3L, "C3", 1L)
        )

        val result = OnchainTxidMatcher.resolve(edges)

        assertEquals(mapOf(3L to "C3"), result)
    }

    @Test
    fun `resolving twice as LDK timestamps shift reflects current costs, not a prior winner`() {
        // LDK's latestUpdateTimestamp is a last-modified time, not a creation time, so it can
        // shift between polls (e.g. confirmation status changes) — the matcher is pure/stateless
        // and must reflect the current costs each call, not "stick" with an earlier round's pick.
        val round1 = listOf(TxidMatchEdge(1L, "C1", 100L), TxidMatchEdge(2L, "C1", 5L))
        assertEquals(mapOf(2L to "C1"), OnchainTxidMatcher.resolve(round1))

        val round2 = listOf(TxidMatchEdge(1L, "C1", 5L), TxidMatchEdge(2L, "C1", 100L))
        assertEquals(mapOf(1L to "C1"), OnchainTxidMatcher.resolve(round2))
    }

    @Test
    fun `no edges resolves nothing`() {
        assertTrue(OnchainTxidMatcher.resolve(emptyList()).isEmpty())
    }

    @Test
    fun `chained propagation resolves a longer forced sequence`() {
        // R1 only fits C1. Once C1 is claimed, R2 (which fit C1 and C2) is forced onto C2. Once
        // C2 is claimed, R3 (which fit C2 and C3) is forced onto C3.
        val edges = listOf(
            TxidMatchEdge(1L, "C1", 1L),
            TxidMatchEdge(2L, "C1", 1L),
            TxidMatchEdge(2L, "C2", 1L),
            TxidMatchEdge(3L, "C2", 1L),
            TxidMatchEdge(3L, "C3", 1L)
        )

        val result = OnchainTxidMatcher.resolve(edges)

        assertEquals(mapOf(1L to "C1", 2L to "C2", 3L to "C3"), result)
    }
}
