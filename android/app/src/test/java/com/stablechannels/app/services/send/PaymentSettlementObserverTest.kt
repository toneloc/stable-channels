package com.stablechannels.app.services.send

import com.stablechannels.app.domain.send.PaymentSettlement
import com.stablechannels.app.services.PaymentOutcome
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class PaymentSettlementObserverTest {
    private val outcomes = MutableStateFlow<Map<String, PaymentOutcome>>(emptyMap())
    private val refreshes = mutableListOf<String>()
    private val observer = PaymentSettlementObserver(outcomes, { id, _ -> refreshes += id }, 2_000)

    private fun settle(id: String, succeeded: Boolean, at: Long = 10, message: String = "msg") =
        outcomes.update {
            it + (id to PaymentOutcome(succeeded, message, observedAtNanos = at))
        }

    @Test
    fun `settlement that landed before waiting is found`() = runTest {
        settle("p", succeeded = true)

        assertEquals(PaymentSettlement.Settled, observer.await("p", 5, 7_000))
    }

    @Test
    fun `an earlier attempt's result does not count`() = runTest {
        settle("p", succeeded = true, at = 1)

        assertEquals(PaymentSettlement.TimedOut, observer.await("p", 5, 7_000))
    }

    @Test
    fun `settlement arriving while waiting ends the wait`() = runTest {
        val result = async { observer.await("p", 5, 7_000) }
        advanceTimeBy(3_000)
        assertFalse(result.isCompleted)

        settle("p", succeeded = true)
        runCurrent()

        assertTrue(result.isCompleted)
        assertEquals(PaymentSettlement.Settled, result.await())
    }

    @Test
    fun `failure carries the node reason`() = runTest {
        settle("p", succeeded = false, message = "No route")

        assertEquals(PaymentSettlement.Failed("No route"), observer.await("p", 5, 7_000))
    }

    @Test
    fun `no result in time is a timeout and the node is polled meanwhile`() = runTest {
        assertEquals(PaymentSettlement.TimedOut, observer.await("p", 5, 7_000))

        assertEquals(List(4) { "p" }, refreshes) // at 0, 2, 4 and 6 seconds
    }
}
