package com.stablechannels.app.services

import android.content.Context
import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.lightningdevkit.ldknode.Event
import org.lightningdevkit.ldknode.OutPoint
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class LspSpliceRecordTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService
    private val channel = "ab".repeat(32)
    private val txid = "cd".repeat(32)
    private val oldTxid = "ef".repeat(32)
    private val funding = OutPoint(txid, 1u)
    private val negotiated = Event.SpliceNegotiated(channel, "7", "peer", funding)
    private val ready = Event.ChannelReady(channel, "7", "peer", funding)
    private val valueBefore = 100_000L
    private val valueAfter = 150_000L

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
        db = DatabaseService(context)
        db.saveChannel(channel, "7", 10.0, 10_000, null, 10_000, 100_000.0)
    }

    @After
    fun tearDown() {
        db.close()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
    }

    private fun move(): Long =
        db.recordPendingSplice(
            "splice_in",
            50_000_000,
            50.0,
            100_000.0,
            "7",
            channel,
            previousFundingTxid = oldTxid,
        )

    private fun spliceOut(): Long =
        db.recordPendingSplice(
            "splice_out",
            30_000_000,
            30.0,
            100_000.0,
            "7",
            channel,
            previousFundingTxid = oldTxid,
        )

    private fun payment(id: Long) = db.getRecentPayments(100).single { it.id == id }

    /** Moves the Move's creation [secs] into the past. */
    private fun age(id: Long, secs: Long) =
        db.writableDatabase.execSQL(
            "UPDATE payments SET created_at = created_at - ? WHERE id = ?",
            arrayOf(secs, id),
        )

    private val nowSecs: Long
        get() = System.currentTimeMillis() / 1000

    /** The node lists the channel at [value] on [onFunding]: the old funding until the lock. */
    private fun record(
        event: Event,
        lspSplice: Boolean = false,
        value: Long,
        onFunding: String = if (event is Event.ChannelReady) txid else oldTxid,
        seen: ChannelValue? = null,
        now: Long = 1_000,
    ) =
        SpliceEventRecorder.record(
            db,
            event,
            lspSplice = lspSplice,
            channelValue = { _, _ -> ChannelValue(onFunding, value) },
            channelValueSeen = { _, _ -> seen },
            nowEpochSecs = now,
        ) {
            null
        }

    @Test
    fun aNegotiationNoMoveClaimsIsTheLspsAndTheReadyEventLocksItWithItsSize() {
        assertTrue(record(negotiated, value = valueBefore))
        val seen = db.getLspSplice(txid)!!
        assertEquals(("negotiated" to valueBefore), (seen.status to seen.valueBeforeSats))
        assertNull(seen.addedSats)
        assertTrue(db.hasPendingLspSplice(1_000))
        assertTrue("nothing in the user's history", db.getRecentPayments(100).isEmpty())

        assertTrue(record(ready, value = valueAfter, now = 1_010))
        val locked = db.getLspSplice(txid)!!
        assertEquals(("locked" to 50_000L), (locked.status to locked.addedSats))
        assertEquals(1_010L, locked.lockedAt)
        assertFalse(db.hasPendingLspSplice(1_010))
        // Replays change nothing.
        assertTrue(record(ready, value = valueAfter + 1, now = 1_020))
        assertTrue(record(negotiated, value = valueBefore + 1, now = 1_020))
        assertEquals(locked, db.getLspSplice(txid))
        assertTrue(db.getRecentPayments(100).isEmpty())
    }

    @Test
    fun aLockThatLandsBeforeTheNegotiationIsHandledTakesTheValueBeforeFromTheListingSeenLast() {
        // 0-conf: the node already lists the channel on the new funding at its new size.
        assertTrue(
            record(
                negotiated,
                value = valueAfter,
                onFunding = txid,
                seen = ChannelValue(oldTxid, valueBefore),
            )
        )
        assertEquals(valueBefore, db.getLspSplice(txid)!!.valueBeforeSats)
        assertTrue(record(ready, value = valueAfter, now = 1_010))
        assertEquals(50_000L, db.getLspSplice(txid)!!.addedSats)
    }

    @Test
    fun aValueOnlySeenOnTheNewFundingLeavesTheSizeUnknownRatherThanZero() {
        assertTrue(
            record(
                negotiated,
                value = valueAfter,
                onFunding = txid,
                seen = ChannelValue(txid, valueAfter),
            )
        )
        assertNull(db.getLspSplice(txid)!!.valueBeforeSats)
        // Nor is a listing on some other funding at the ready event the value after.
        assertTrue(record(ready, value = valueAfter + 5, onFunding = oldTxid, now = 1_010))
        val locked = db.getLspSplice(txid)!!
        assertEquals("locked", locked.status)
        assertNull(locked.valueAfterSats)
        assertNull(locked.addedSats)
    }

    @Test
    fun aNegotiatedLspSpliceStopsHoldingTheRepairBackAfterADay() {
        assertTrue(record(negotiated, value = valueBefore, now = 1_000))
        val day = SpliceEventRecorder.LSP_SPLICE_PENDING_MAX_AGE_SECS
        assertTrue(db.hasPendingLspSplice(1_000 + day - 1))
        assertFalse(db.hasPendingLspSplice(1_000 + day))
    }

    @Test
    fun aSpliceThePushAnnouncedLeavesAMoveStillWaitingForItsTransactionAlone() {
        val id = move()
        assertTrue(record(negotiated, lspSplice = true, value = valueBefore))
        assertNull("the Move keeps waiting for its own transaction", payment(id).txid)
        assertEquals("pending", payment(id).status)
        assertEquals("negotiated", db.getLspSplice(txid)!!.status)
        // Nor does the LSP's lock claim it, though the funding moved on from the Move's.
        assertTrue(record(ready, lspSplice = true, value = valueAfter))
        assertNull(payment(id).txid)
        assertEquals("locked", db.getLspSplice(txid)!!.status)
        // The Move's own negotiation still finds it.
        val own = OutPoint("12".repeat(32), 0u)
        assertTrue(record(negotiated.copy(newFundingTxo = own), value = valueAfter))
        assertEquals(own.txid, payment(id).txid)
        assertNull(db.getLspSplice(own.txid))
    }

    @Test
    fun theUsersOwnSpliceOutInsideThePushWindowIsItsOwnOnceTheChannelIsReadySmaller() {
        val now = nowSecs
        val id = spliceOut()
        assertTrue(record(negotiated, lspSplice = true, value = valueBefore, now = now))
        assertNull("the negotiation names no size, so it is not given yet", payment(id).txid)
        assertEquals("negotiated", db.getLspSplice(txid)!!.status)
        // Past the pre-negotiation timeout, the splice-out still waits for the ready event.
        age(id, 3_600)
        assertTrue(db.hasPendingSplice())
        assertEquals("pending", payment(id).status)

        // Ready on the new funding, smaller: the splice was the user's splice-out.
        assertTrue(record(ready, lspSplice = true, value = valueBefore - 30_500, now = now + 600))
        assertEquals(txid, payment(id).txid)
        assertEquals("pending", payment(id).status)
        assertNull("not the LSP's splice", db.getLspSplice(txid))
        assertFalse(db.hasPendingLspSplice(now + 600))
        // Replays change nothing.
        assertTrue(record(ready, lspSplice = true, value = valueBefore - 30_500, now = now + 610))
        assertTrue(record(negotiated, lspSplice = true, value = valueBefore, now = now + 610))
        assertNull(db.getLspSplice(txid))
        assertEquals(txid, payment(id).txid)
    }

    @Test
    fun theLspsSpliceInsideThePushWindowIsNotGivenToALeftoverSpliceOut() {
        val now = nowSecs
        val id = spliceOut()
        assertTrue(record(negotiated, lspSplice = true, value = valueBefore, now = now))
        // Ready larger, after the window: the LSP's splice-in, which a Move waiting for its
        // transaction would otherwise be given.
        assertTrue(record(ready, lspSplice = false, value = valueAfter, now = now + 900))
        assertNull(payment(id).txid)
        val locked = db.getLspSplice(txid)!!
        assertEquals(("locked" to 50_000L), (locked.status to locked.addedSats))
        // No longer waiting on the LSP's splice, the leftover expires as before.
        age(id, 3_600)
        assertFalse(db.hasPendingSplice())
        assertEquals("failed", payment(id).status)
    }

    @Test
    fun aSpliceOutIsOnlyGivenTheLspsSpliceWhenItIsTheOneMoveWaiting() {
        val now = nowSecs
        val first = spliceOut()
        val second = spliceOut()
        assertTrue(record(negotiated, lspSplice = true, value = valueBefore, now = now))
        assertTrue(record(ready, lspSplice = true, value = valueBefore - 30_500, now = now + 600))
        assertNull(payment(first).txid)
        assertNull(payment(second).txid)
        assertEquals("locked", db.getLspSplice(txid)!!.status)
    }

    @Test
    fun aSpliceOutWaitsForTheReadyEventOnlyAsLongAsTheLspsSpliceHoldsTheRepairBack() {
        val day = SpliceEventRecorder.LSP_SPLICE_PENDING_MAX_AGE_SECS
        val id = spliceOut()
        assertTrue(record(negotiated, lspSplice = true, value = valueBefore, now = nowSecs - day))
        age(id, day + 60)
        assertFalse(db.hasPendingSplice())
        assertEquals("failed", payment(id).status)
    }

    @Test
    fun withoutThePushAMoveWaitingForItsTransactionTakesTheNegotiationAsBefore() {
        val id = move()
        assertTrue(record(negotiated, value = valueBefore))
        assertEquals(txid, payment(id).txid)
        assertNull("the user's own splice is not recorded as the LSP's", db.getLspSplice(txid))
    }

    @Test
    fun aReplayOfTheMovesOwnNegotiationIsNotTakenForTheLsps() {
        val id = move()
        assertTrue(record(negotiated, value = valueBefore))
        assertEquals(txid, payment(id).txid)
        // The same event again, now inside the window of an LSP push.
        assertTrue(record(negotiated, lspSplice = true, value = valueBefore))
        assertNull(db.getLspSplice(txid))
        assertEquals(txid, payment(id).txid)
    }
}
