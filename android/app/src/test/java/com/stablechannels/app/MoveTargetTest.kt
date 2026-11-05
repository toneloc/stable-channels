package com.stablechannels.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Move splices only into a ready channel whose funding has confirmed (#467). */
class MoveTargetTest {

    @Test
    fun aReadyChannelIsNotAMoveTargetBeforeItsFundingConfirms() {
        // A zero-conf channel is ready before its funding confirms.
        assertFalse(AppState.isMoveTarget(isChannelReady = true, confirmations = 0u))
        assertFalse(AppState.isMoveTarget(isChannelReady = true, confirmations = null))
    }

    @Test
    fun aReadyChannelWithConfirmedFundingIsAMoveTarget() {
        assertTrue(AppState.isMoveTarget(isChannelReady = true, confirmations = 1u))
    }

    @Test
    fun aChannelThatIsNotReadyIsNotAMoveTarget() {
        assertFalse(AppState.isMoveTarget(isChannelReady = false, confirmations = 6u))
    }
}
