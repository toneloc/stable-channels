package com.stablechannels.app.services.websocket

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ReconnectionManagerTest {

    @Test
    fun `disconnected schedules reconnect with exponential delays`() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val manager = ReconnectionManager(this, dispatcher = dispatcher)
        var reconnectCount = 0
        manager.onReconnect = { reconnectCount += 1 }

        manager.disconnected()
        assertEquals(1, manager.reconnectAttempts)
        advanceTimeBy(999)
        runCurrent()
        assertEquals(0, reconnectCount)
        advanceTimeBy(1)
        runCurrent()
        assertEquals(1, reconnectCount)

        manager.disconnected()
        assertEquals(2, manager.reconnectAttempts)
        advanceTimeBy(1999)
        runCurrent()
        assertEquals(1, reconnectCount)
        advanceTimeBy(1)
        runCurrent()
        assertEquals(2, reconnectCount)
    }

    @Test
    fun `backoff delay is capped at max reconnect delay`() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val manager = ReconnectionManager(this, maxReconnectDelaySeconds = 3, dispatcher = dispatcher)
        var reconnectCount = 0
        manager.onReconnect = { reconnectCount += 1 }

        manager.disconnected()
        advanceTimeBy(1000)
        runCurrent()
        assertEquals(1, reconnectCount)

        manager.disconnected()
        advanceTimeBy(2000)
        runCurrent()
        assertEquals(2, reconnectCount)

        manager.disconnected()
        advanceTimeBy(2999)
        runCurrent()
        assertEquals(2, reconnectCount)
        advanceTimeBy(1)
        runCurrent()
        assertEquals(3, reconnectCount)
    }

    @Test
    fun `stop prevents pending reconnect callback`() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val manager = ReconnectionManager(this, dispatcher = dispatcher)
        var reconnectCount = 0
        manager.onReconnect = { reconnectCount += 1 }

        manager.disconnected()
        manager.stop()
        assertTrue(manager.isManualDisconnect)
        advanceTimeBy(2000)
        runCurrent()

        assertEquals(0, reconnectCount)
    }

    @Test
    fun `reset clears manual mode and attempts`() = runTest {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val manager = ReconnectionManager(this, dispatcher = dispatcher)

        manager.disconnected()
        manager.stop()
        assertTrue(manager.isManualDisconnect)
        assertEquals(1, manager.reconnectAttempts)

        manager.reset()
        assertFalse(manager.isManualDisconnect)
        assertEquals(0, manager.reconnectAttempts)
    }
}