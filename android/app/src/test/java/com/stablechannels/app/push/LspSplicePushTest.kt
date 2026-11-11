package com.stablechannels.app.push

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LspSplicePushTest {
    @Test
    fun `a splice push for a running app that can hold becomes a hold`() {
        assertEquals(
            PushRoute.HOLD_IN_APP,
            routePush(
                LSP_SPLICE_IN_DIRECTION,
                mainAppActive = true,
                nodeOwnedInProcess = false,
                serviceRunning = false,
                appCanHold = true,
            ),
        )
        // Inside the quick-switch grace the heartbeat is stale but the app still owns the node.
        assertEquals(
            PushRoute.HOLD_IN_APP,
            routePush(
                LSP_SPLICE_IN_DIRECTION,
                mainAppActive = false,
                nodeOwnedInProcess = true,
                serviceRunning = false,
                appCanHold = true,
            ),
        )
    }

    @Test
    fun `a running app that cannot hold is only flagged, as for any other push`() {
        assertEquals(
            PushRoute.FLAG_FOR_APP,
            routePush(
                LSP_SPLICE_IN_DIRECTION,
                mainAppActive = true,
                nodeOwnedInProcess = false,
                serviceRunning = false,
                appCanHold = false,
            ),
        )
        assertEquals(
            PushRoute.FLAG_FOR_APP,
            routePush(
                "lsp_to_user",
                mainAppActive = true,
                nodeOwnedInProcess = false,
                serviceRunning = false,
                appCanHold = true,
            ),
        )
        assertEquals(
            PushRoute.FLAG_FOR_APP,
            routePush(
                LSP_SPLICE_IN_DIRECTION,
                mainAppActive = false,
                nodeOwnedInProcess = false,
                serviceRunning = true,
                appCanHold = true,
            ),
        )
    }

    @Test
    fun `with nothing owning the node the service is started`() {
        for (direction in
            listOf(LSP_SPLICE_IN_DIRECTION, "lsp_to_user", "incoming_payment", "user_to_lsp")) {
            assertEquals(
                direction,
                PushRoute.START_SERVICE,
                routePush(
                    direction,
                    mainAppActive = false,
                    nodeOwnedInProcess = false,
                    serviceRunning = false,
                    appCanHold = false,
                ),
            )
        }
    }

    @Test
    fun `the bridge only holds while an app is registered`() {
        LspSpliceHoldBridge.holder = null
        assertFalse(LspSpliceHoldBridge.canHold)
        assertFalse(LspSpliceHoldBridge.requestHold())
        var held = 0
        LspSpliceHoldBridge.holder = { held++ }
        assertTrue(LspSpliceHoldBridge.canHold)
        assertTrue(LspSpliceHoldBridge.requestHold())
        assertEquals(1, held)
        LspSpliceHoldBridge.holder = null
    }

    @Test
    fun `a splice is expected from the push until the window ends`() {
        assertFalse("no push yet", LspSpliceExpectation.expectedWithin(0L, 5_000L))
        assertTrue(LspSpliceExpectation.expectedWithin(5_000L, 5_000L))
        assertTrue(
            LspSpliceExpectation.expectedWithin(5_000L, 5_000L + LspSpliceExpectation.WINDOW_MS - 1)
        )
        assertFalse(
            LspSpliceExpectation.expectedWithin(5_000L, 5_000L + LspSpliceExpectation.WINDOW_MS)
        )
        assertFalse("clock before the push", LspSpliceExpectation.expectedWithin(5_000L, 4_999L))
    }

    @Test
    fun `the wait ends at the lock plus the settle window`() {
        val wait = LspSpliceWait(startMs = 1_000, waitMs = 60_000, settleMs = 10_000)
        assertFalse(wait.isDone(30_000))
        assertEquals(LspSpliceWait.Outcome.TIMED_OUT, wait.outcome)
        wait.onLocked(30_000)
        assertFalse("still settling", wait.isDone(39_999))
        assertTrue(wait.isDone(40_000))
        assertEquals(LspSpliceWait.Outcome.LOCKED, wait.outcome)
        // A second lock does not restart the settle window.
        wait.onLocked(45_000)
        assertTrue(wait.isDone(45_000))
    }

    @Test
    fun `the wait ends at once on a failed negotiation and at the deadline otherwise`() {
        val failed = LspSpliceWait(startMs = 1_000, waitMs = 60_000, settleMs = 10_000)
        failed.onFailed()
        assertTrue(failed.isDone(1_001))
        assertEquals(LspSpliceWait.Outcome.FAILED, failed.outcome)

        val idle = LspSpliceWait(startMs = 1_000, waitMs = 60_000, settleMs = 10_000)
        assertFalse(idle.isDone(60_999))
        assertTrue(idle.isDone(61_000))
        assertEquals(LspSpliceWait.Outcome.TIMED_OUT, idle.outcome)

        // A lock close to the deadline does not extend it.
        val late = LspSpliceWait(startMs = 1_000, waitMs = 60_000, settleMs = 10_000)
        late.onLocked(58_000)
        assertTrue(late.isDone(61_000))
        assertEquals(LspSpliceWait.Outcome.LOCKED, late.outcome)

        // A failure after the lock ends the wait at once but does not take the lock back.
        val lockedThenFailed = LspSpliceWait(startMs = 1_000, waitMs = 60_000, settleMs = 10_000)
        lockedThenFailed.onLocked(30_000)
        lockedThenFailed.onFailed()
        assertTrue(lockedThenFailed.isDone(30_001))
        assertEquals(LspSpliceWait.Outcome.LOCKED, lockedThenFailed.outcome)
    }
}
