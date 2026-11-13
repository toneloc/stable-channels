package com.stablechannels.app.services

import org.junit.Assert.assertEquals
import org.junit.Test

class BackgroundStopPlanTest {
    @Test
    fun `nothing in flight gets the quick-switch grace and no service`() {
        assertEquals(
            BackgroundStopPlan(QUICK_SWITCH_GRACE_MS, null),
            backgroundStopPlan(
                isWaitingForPayment = false,
                isPickingMedia = false,
                negotiatingSplice = false,
                holdingForLspSplice = false,
            ),
        )
    }

    @Test
    fun `an LSP splice the push announced keeps the node for the long grace`() {
        assertEquals(
            BackgroundStopPlan(LONG_BACKGROUND_GRACE_MS, LdkBackgroundService.REASON_LSP_SPLICE),
            backgroundStopPlan(
                isWaitingForPayment = false,
                isPickingMedia = false,
                negotiatingSplice = false,
                holdingForLspSplice = true,
            ),
        )
    }

    @Test
    fun `the app's own move wins the notification text over an LSP splice`() {
        assertEquals(
            BackgroundStopPlan(LONG_BACKGROUND_GRACE_MS, LdkBackgroundService.REASON_SPLICE),
            backgroundStopPlan(
                isWaitingForPayment = true,
                isPickingMedia = false,
                negotiatingSplice = true,
                holdingForLspSplice = true,
            ),
        )
    }

    @Test
    fun `a payment wait or an open picker keeps the long grace as before`() {
        for (picking in listOf(false, true)) {
            assertEquals(
                BackgroundStopPlan(LONG_BACKGROUND_GRACE_MS, LdkBackgroundService.REASON_PAYMENT),
                backgroundStopPlan(
                    isWaitingForPayment = !picking,
                    isPickingMedia = picking,
                    negotiatingSplice = false,
                    holdingForLspSplice = false,
                ),
            )
        }
    }
}
