package com.stablechannels.app.services

/**
 * A candidate pairing between an unresolved payment row and an LDK-reported txid that could
 * plausibly belong to it (same amount, within the matching time window, and not already claimed
 * by another row) — see [OnchainTxidMatcher].
 */
data class TxidMatchEdge(val rowId: Long, val txid: String, val costSecs: Long)

/**
 * Resolves ambiguous row/txid candidate pairings using a maximum-cardinality, minimum-cost
 * bipartite assignment: first maximize the number of rows matched, then among all assignments
 * tied for that cardinality, minimize total timestamp distance. A row/txid pairing is only
 * committed if it appears in *every* assignment tied for (max cardinality, min cost) — i.e. it is
 * invariant. Amount + timestamp proximity is a heuristic, not a true identity (LDK's timestamp is
 * a last-modified time, not a creation time, and repeated round-number deposits are ordinary), so
 * this never guesses between two equally valid options — a row left with a genuine ambiguity
 * stays unresolved and is retried on the next poll with fresh candidates.
 *
 * Implementation: exhaustive branch-and-bound search over "assign row to an unused compatible
 * txid, or leave it unmatched." This is exponential in the worst case, but the graphs here are
 * tiny in practice (a handful of backlog rows per poll), and a hard exploration cap makes the
 * search bail out (leaving affected rows unresolved) rather than risk an incomplete search
 * silently producing a wrong "best" answer.
 */
object OnchainTxidMatcher {

    private const val MAX_EXPLORED_STATES = 200_000

    fun resolve(edges: List<TxidMatchEdge>): Map<Long, String> {
        if (edges.isEmpty()) return emptyMap()
        val rows = edges.map { it.rowId }.distinct()
        val byRow = edges.groupBy { it.rowId }

        val base = bestAssignment(rows, byRow, forbidden = null)
        if (base == null || base.hitCap) return emptyMap()

        val result = mutableMapOf<Long, String>()
        for ((rowId, txid) in base.assignment) {
            val alt = bestAssignment(rows, byRow, forbidden = rowId to txid)
            // Not proven invariant if: an equally good alternative exists, or the search was cut
            // off before it could rule one out (conservatively treat as ambiguous either way).
            val provenInvariant = alt != null && !alt.hitCap &&
                (alt.cardinality < base.cardinality ||
                    (alt.cardinality == base.cardinality && alt.cost > base.cost))
            if (provenInvariant) result[rowId] = txid
        }
        return result
    }

    private data class Solution(
        val cardinality: Int,
        val cost: Long,
        val assignment: Map<Long, String>,
        val hitCap: Boolean
    )

    private fun bestAssignment(
        rows: List<Long>,
        byRow: Map<Long, List<TxidMatchEdge>>,
        forbidden: Pair<Long, String>?
    ): Solution? {
        var statesExplored = 0
        var hitCap = false
        var best: Solution? = null

        // Fewest-options-first ordering prunes faster (classic MRV heuristic).
        val orderedRows = rows.sortedBy { byRow[it]?.size ?: 0 }
        val usedTxids = mutableSetOf<String>()
        val current = mutableMapOf<Long, String>()

        fun isBetter(cardinality: Int, cost: Long): Boolean {
            val b = best
            return b == null || cardinality > b.cardinality || (cardinality == b.cardinality && cost < b.cost)
        }

        fun backtrack(index: Int, cardinality: Int, cost: Long) {
            if (hitCap) return
            statesExplored++
            if (statesExplored > MAX_EXPLORED_STATES) {
                hitCap = true
                return
            }
            if (index == orderedRows.size) {
                if (isBetter(cardinality, cost)) {
                    best = Solution(cardinality, cost, current.toMap(), hitCap = false)
                }
                return
            }
            // Bound: even if every remaining row matched, can this branch still beat the best
            // known cardinality? If not, no point exploring further down this path.
            val b = best
            if (b != null && cardinality + (orderedRows.size - index) < b.cardinality) return

            val rowId = orderedRows[index]
            val options = byRow[rowId].orEmpty()
                .filterNot { forbidden != null && forbidden.first == rowId && forbidden.second == it.txid }

            // Branch: leave this row unmatched.
            backtrack(index + 1, cardinality, cost)

            // Branch: assign to each compatible, still-unused txid.
            for (edge in options) {
                if (edge.txid in usedTxids) continue
                usedTxids += edge.txid
                current[rowId] = edge.txid
                backtrack(index + 1, cardinality + 1, cost + edge.costSecs)
                current.remove(rowId)
                usedTxids -= edge.txid
            }
        }

        backtrack(0, 0, 0L)
        return best?.copy(hitCap = hitCap) ?: if (hitCap) Solution(0, 0L, emptyMap(), hitCap = true) else null
    }
}
