package com.stablechannels.app

import com.stablechannels.app.services.ConfirmationPollResult
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class PaymentConfirmationRefreshTest {

    @Test
    fun `manual refresh reports failure instead of success when history is unavailable`() =
        runBlocking {
            val state = AppState(RuntimeEnvironment.getApplication())

            assertEquals(
                ConfirmationPollResult.DatabaseUnavailable,
                state.refreshPaymentConfirmations(),
            )
        }
}
