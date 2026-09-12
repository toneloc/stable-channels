package com.stablechannels.app

import com.stablechannels.app.services.TradeFailureNotice
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TradeFailureNoticeTest {

    @Test
    fun `a startup message cannot overwrite a rejection, and the rejection is not burned`() {
        // (5) The capsule already holds a live message (start() is re-invocable from ErrorView's
        // retry, where "Network unstable…" is plausible). The rejection must not displace it —
        // and must not be marked seen, or it could never be shown at all.
        assertFalse(
            TradeFailureNotice.shouldShow(
                failurePaymentId = "aa".repeat(32),
                lastShownPaymentId = null,
                capsuleOccupied = true
            )
        )
        // The next launch, with a free capsule, still shows it.
        assertTrue(
            TradeFailureNotice.shouldShow(
                failurePaymentId = "aa".repeat(32),
                lastShownPaymentId = null,
                capsuleOccupied = false
            )
        )
    }

    @Test
    fun `a rejection already shown in the foreground does not reappear after relaunch`() {
        // (6) The live trade flow marks it seen as it displays it, so startup stays quiet.
        assertFalse(
            TradeFailureNotice.shouldShow(
                failurePaymentId = "bb".repeat(32),
                lastShownPaymentId = "bb".repeat(32),
                capsuleOccupied = false
            )
        )
        // A different, newer failure is still worth surfacing.
        assertTrue(
            TradeFailureNotice.shouldShow(
                failurePaymentId = "cc".repeat(32),
                lastShownPaymentId = "bb".repeat(32),
                capsuleOccupied = false
            )
        )
    }
}
