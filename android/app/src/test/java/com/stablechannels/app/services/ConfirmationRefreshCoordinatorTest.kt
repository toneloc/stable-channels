package com.stablechannels.app.services

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ConfirmationRefreshCoordinatorTest {

    @Test
    fun `manual refresh suspends until the pass completes and returns its result`() = runTest {
        val gate = CompletableDeferred<Unit>()
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) {
                gate.await()
                ConfirmationPollResult.Completed()
            }

        val refresh = async { coordinator.refresh() }
        runCurrent()
        assertFalse(refresh.isCompleted)

        gate.complete(Unit)
        assertEquals(ConfirmationPollResult.Completed(), refresh.await())
    }

    @Test
    fun `manual refresh waits for an in-flight automatic poll then runs a fresh pass`() = runTest {
        var passes = 0
        var running = 0
        var maxConcurrent = 0
        val firstGate = CompletableDeferred<Unit>()
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) {
                passes++
                running++
                maxConcurrent = maxOf(maxConcurrent, running)
                if (passes == 1) firstGate.await()
                running--
                ConfirmationPollResult.Completed(failedLookups = passes - 1)
            }

        val auto = async { coordinator.pollIfIdle(force = true) }
        runCurrent()
        val manual = async { coordinator.refresh() }
        runCurrent()
        assertFalse("manual refresh must not skip while busy", manual.isCompleted)
        assertEquals(1, passes)

        firstGate.complete(Unit)
        assertEquals(ConfirmationPollResult.Completed(0), auto.await())
        // The manual result comes from its own pass, not the one already in flight.
        assertEquals(ConfirmationPollResult.Completed(1), manual.await())
        assertEquals(2, passes)
        assertEquals(1, maxConcurrent)
    }

    @Test
    fun `automatic poll is skipped while another pass is running`() = runTest {
        var passes = 0
        val gate = CompletableDeferred<Unit>()
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) {
                passes++
                gate.await()
                ConfirmationPollResult.Completed()
            }

        val manual = launch { coordinator.refresh() }
        runCurrent()
        assertNull(coordinator.pollIfIdle(force = true))
        gate.complete(Unit)
        manual.join()
        assertEquals(1, passes)
    }

    @Test
    fun `automatic polls stay throttled while forced and manual ones are not`() = runTest {
        var now = 1_000_000L
        var passes = 0
        val coordinator =
            ConfirmationRefreshCoordinator(minIntervalMs = 15_000L, nowMs = { now }) {
                passes++
                ConfirmationPollResult.Completed()
            }

        assertEquals(ConfirmationPollResult.Completed(), coordinator.pollIfIdle(force = false))
        now += 5_000L
        assertNull(coordinator.pollIfIdle(force = false))
        assertEquals(ConfirmationPollResult.Completed(), coordinator.pollIfIdle(force = true))
        assertEquals(ConfirmationPollResult.Completed(), coordinator.refresh())
        now += 15_000L
        assertEquals(ConfirmationPollResult.Completed(), coordinator.pollIfIdle(force = false))
        assertEquals(4, passes)
    }

    @Test
    fun `failed chain lookups are reported and do not advance the throttle`() = runTest {
        var result: ConfirmationPollResult = ConfirmationPollResult.ChainTipUnavailable
        var passes = 0
        val coordinator =
            ConfirmationRefreshCoordinator(minIntervalMs = 15_000L, nowMs = { 1_000_000L }) {
                passes++
                result
            }

        assertEquals(ConfirmationPollResult.ChainTipUnavailable, coordinator.refresh())
        // A failed pass must not suppress the next automatic poll.
        assertEquals(
            ConfirmationPollResult.ChainTipUnavailable,
            coordinator.pollIfIdle(force = false),
        )

        result = ConfirmationPollResult.Completed(failedLookups = 2)
        assertEquals(ConfirmationPollResult.Completed(2), coordinator.refresh())
        assertEquals(3, passes)
    }

    @Test
    fun `every completed automatic and manual pass publishes its latest result`() = runTest {
        val results = mutableListOf<ConfirmationPollResult>()
        val coordinator =
            ConfirmationRefreshCoordinator(
                nowMs = { 0L },
                onResult = results::add,
            ) {
                if (results.isEmpty()) {
                    ConfirmationPollResult.Completed(failedLookups = 1)
                } else {
                    ConfirmationPollResult.Completed()
                }
            }

        coordinator.pollIfIdle(force = true)
        coordinator.refresh()

        assertEquals(
            listOf(ConfirmationPollResult.Completed(1), ConfirmationPollResult.Completed()),
            results,
        )
    }

    @Test
    fun `a pass that throws releases the lock for the next refresh`() = runTest {
        var shouldThrow = true
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) {
                if (shouldThrow) error("boom")
                ConfirmationPollResult.Completed()
            }

        val failure = runCatching { coordinator.refresh() }
        assertTrue(failure.isFailure)
        shouldThrow = false
        assertEquals(ConfirmationPollResult.Completed(), coordinator.refresh())
    }

    @Test
    fun `refresh error messages are only produced for failures`() {
        assertNull(ConfirmationPollResult.Completed().refreshErrorMessage())
        assertEquals(
            "Couldn't check 1 transaction. Pull to try again.",
            ConfirmationPollResult.Completed(1).refreshErrorMessage(),
        )
        assertEquals(
            "Couldn't check 3 transactions. Pull to try again.",
            ConfirmationPollResult.Completed(3).refreshErrorMessage(),
        )
        assertEquals(
            "Couldn't reach the block explorer. Pull to try again.",
            ConfirmationPollResult.ChainTipUnavailable.refreshErrorMessage(),
        )
        assertEquals(
            "Payment history is unavailable right now.",
            ConfirmationPollResult.DatabaseUnavailable.refreshErrorMessage(),
        )
        assertEquals(
            "Checking confirmations took too long. Pull to try again.",
            ConfirmationPollResult.TimedOut.refreshErrorMessage(),
        )
    }

    @Test
    fun `only manual refresh runs the pass in manual mode`() = runTest {
        val modes = mutableListOf<Boolean>()
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) { manual ->
                modes += manual
                ConfirmationPollResult.Completed()
            }

        coordinator.pollIfIdle(force = true)
        coordinator.refresh()
        assertEquals(listOf(false, true), modes)
    }

    @Test
    fun `manual refresh deadline includes waiting for an in-flight pass`() = runTest {
        var passes = 0
        val gate = CompletableDeferred<Unit>()
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) {
                passes++
                if (passes == 1) gate.await()
                ConfirmationPollResult.Completed()
            }

        val auto = async { coordinator.pollIfIdle(force = true) }
        runCurrent()
        val manual = async { coordinator.refresh(deadlineMs = 20_000L) }
        advanceTimeBy(19_999L)
        assertFalse(manual.isCompleted)
        advanceTimeBy(2L)
        assertEquals(ConfirmationPollResult.TimedOut, manual.await())
        // The manual refresh never got its own pass; the automatic one is unaffected.
        assertEquals(1, passes)
        assertFalse(auto.isCompleted)

        gate.complete(Unit)
        assertEquals(ConfirmationPollResult.Completed(), auto.await())
        assertEquals(ConfirmationPollResult.Completed(), coordinator.refresh())
        assertEquals(2, passes)
    }

    @Test
    fun `manual refresh deadline cancels a stalled pass and releases the lock`() = runTest {
        var passes = 0
        var cancelled = false
        val coordinator =
            ConfirmationRefreshCoordinator(minIntervalMs = 15_000L, nowMs = { 1_000_000L }) {
                passes++
                if (passes == 1) {
                    try {
                        CompletableDeferred<Unit>().await()
                    } finally {
                        cancelled = true
                    }
                }
                ConfirmationPollResult.Completed()
            }

        assertEquals(ConfirmationPollResult.TimedOut, coordinator.refresh(deadlineMs = 20_000L))
        assertTrue(cancelled)
        // A timed-out pass does not advance the automatic-poll throttle.
        assertEquals(ConfirmationPollResult.Completed(), coordinator.pollIfIdle(force = false))
        assertEquals(2, passes)
    }

    @Test
    fun `cancelling a waiting manual refresh leaves the lock usable`() = runTest {
        val gate = CompletableDeferred<Unit>()
        var passes = 0
        val coordinator =
            ConfirmationRefreshCoordinator(nowMs = { 0L }) {
                passes++
                gate.await()
                ConfirmationPollResult.Completed()
            }

        val auto = async { coordinator.pollIfIdle(force = true) }
        runCurrent()
        val manual = launch { coordinator.refresh() }
        runCurrent()
        manual.cancel()
        manual.join()
        gate.complete(Unit)
        auto.await()
        assertEquals(ConfirmationPollResult.Completed(), coordinator.refresh())
        assertEquals(2, passes)
    }
}
