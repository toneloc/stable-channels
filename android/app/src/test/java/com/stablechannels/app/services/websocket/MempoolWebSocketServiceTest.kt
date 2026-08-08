package com.stablechannels.app.services.websocket

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestDispatcher
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TestWatcher
import org.junit.runner.Description

@OptIn(ExperimentalCoroutinesApi::class)
class MempoolWebSocketServiceTest {

    @get:Rule
    val mainDispatcherRule = MainDispatcherRule()

    @Test
    fun `emits receive for tracked address and dedups duplicate message`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val address = "bc1qtrackedaddress"
        val txid = validTxid('a')
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }

        setTrackedAddresses(service, setOf(address))

        val message =
            """
            {
              "address-transactions": [
                {
                  "txid": "$txid",
                  "vout": [
                    { "scriptpubkey_address": "$address", "value": 12345 }
                  ]
                }
              ]
            }
            """.trimIndent()

        invokeHandleMessage(service, message)
        invokeHandleMessage(service, message)
        advanceUntilIdle()

        assertEquals(1, events.size)
        assertEquals(WebSocketEvent.Receive(address, txid, 12345L), events.first())
    }

    @Test
    fun `emits removed event for tracked address`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val address = "bc1qremovedaddress"
        val txid = validTxid('b')
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }

        setTrackedAddresses(service, setOf(address))

        val message =
            """
            {
              "multi-address-transactions": {
                "$address": {
                  "removed": [
                    { "txid": "$txid" }
                  ]
                }
              }
            }
            """.trimIndent()

        invokeHandleMessage(service, message)
        advanceUntilIdle()

        assertEquals(1, events.size)
        assertEquals(WebSocketEvent.Removed(address, txid), events.first())
    }

    @Test
    fun `emits tracked outspend for tracked txid`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val trackedTxid = validTxid('c')
        val spendingTxid = validTxid('d')
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }

        setTrackedTxids(service, setOf(trackedTxid))

        val message =
            """
            {
              "tracked-txs": {
                "$trackedTxid": {
                  "utxoSpent": {
                    "0": { "txid": "$spendingTxid", "vin": 0 }
                  }
                }
              }
            }
            """.trimIndent()

        invokeHandleMessage(service, message)
        advanceUntilIdle()

        assertEquals(1, events.size)
        assertEquals(WebSocketEvent.TrackedOutspend(trackedTxid, spendingTxid), events.first())
    }

    @Test
    fun `uses last block height from blocks array`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val blockHeights = mutableListOf<Int>()
        service.onBlockHeader = { blockHeights.add(it) }

        val message =
            """
            {
              "blocks": [
                { "height": 810001 },
                { "height": 810002 }
              ]
            }
            """.trimIndent()

        invokeHandleMessage(service, message)
        advanceUntilIdle()

        assertEquals(listOf(810002), blockHeights)
    }

    @Test
    fun `ignores empty payloads without emitting events`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }

        invokeHandleMessage(service, "{}")
        advanceUntilIdle()

        assertTrue(events.isEmpty())
    }

    @Test
    fun `ignores malformed json payload without emitting events`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }

        invokeHandleMessage(service, "this is not json {{{")
        advanceUntilIdle()

        assertTrue(events.isEmpty())
    }

    @Test
    fun `ignores invalid txid in address transactions`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val address = "bc1qignoreinvalid"
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }
        setTrackedAddresses(service, setOf(address))

        invokeHandleMessage(
            service,
            """
            {
              "address-transactions": [
                { "txid": "short", "vout": [{ "scriptpubkey_address": "$address", "value": 1000 }] }
              ]
            }
            """.trimIndent()
        )
        advanceUntilIdle()

        assertTrue(events.isEmpty())
    }

    @Test
    fun `handles block and transaction in same payload`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val address = "bc1qcombined"
        val txid = validTxid('e')
        val events = mutableListOf<WebSocketEvent>()
        val blocks = mutableListOf<Int>()

        service.onTransactionDetected = { events.add(it) }
        service.onBlockHeader = { blocks.add(it) }
        setTrackedAddresses(service, setOf(address))

        invokeHandleMessage(
            service,
            """
            {
              "address-transactions": [
                {
                  "txid": "$txid",
                  "vout": [{ "scriptpubkey_address": "$address", "value": 25000 }]
                }
              ],
              "block": { "height": 800001 }
            }
            """.trimIndent()
        )
        advanceUntilIdle()

        assertEquals(listOf(WebSocketEvent.Receive(address, txid, 25000L)), events)
        assertEquals(listOf(800001), blocks)
    }

    @Test
    fun `sums matching vouts for same address`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val address = "bc1qmultivout"
        val txid = validTxid('f')
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }
        setTrackedAddresses(service, setOf(address))

        invokeHandleMessage(
            service,
            """
            {
              "address-transactions": [
                {
                  "txid": "$txid",
                  "vout": [
                    { "scriptpubkey_address": "$address", "value": 100000 },
                    { "scriptpubkey_address": "$address", "value": 50000 },
                    { "scriptpubkey_address": "bc1qother", "value": 999 }
                  ]
                }
              ]
            }
            """.trimIndent()
        )
        advanceUntilIdle()

        assertEquals(listOf(WebSocketEvent.Receive(address, txid, 150000L)), events)
    }

    @Test
    fun `handles multiple deposits to same address`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val address = "bc1qmultiple"
        val txid1 = validTxid('1')
        val txid2 = validTxid('2')
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }
        setTrackedAddresses(service, setOf(address))

        invokeHandleMessage(
            service,
            """
            {
              "address-transactions": [
                { "txid": "$txid1", "vout": [{ "scriptpubkey_address": "$address", "value": 1000 }] },
                { "txid": "$txid2", "vout": [{ "scriptpubkey_address": "$address", "value": 2000 }] }
              ]
            }
            """.trimIndent()
        )
        advanceUntilIdle()

        val receives = events.filterIsInstance<WebSocketEvent.Receive>()
        assertEquals(2, receives.size)
        assertEquals(setOf(1000L, 2000L), receives.map { it.amountSats }.toSet())
    }

    @Test
    fun `removed only payload emits removed and not receive`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val address = "bc1qremovedonly"
        val txid = validTxid('a')
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }
        setTrackedAddresses(service, setOf(address))

        invokeHandleMessage(
            service,
            """
            {
              "multi-address-transactions": {
                "$address": {
                  "mempool": [],
                  "confirmed": [],
                  "removed": [{ "txid": "$txid" }]
                }
              }
            }
            """.trimIndent()
        )
        advanceUntilIdle()

        assertEquals(1, events.size)
        assertTrue(events.first() is WebSocketEvent.Removed)
        assertNull(events.filterIsInstance<WebSocketEvent.Receive>().firstOrNull())
    }

    @Test
    fun `multiple tracked txids can emit outspends with same spending txid and dedup repeats`() = runTest {
        val service = MempoolWebSocketService(serviceScope = this)
        val trackedTxid1 = validTxid('b')
        val trackedTxid2 = validTxid('c')
        val spendingTxid = validTxid('d')
        val events = mutableListOf<WebSocketEvent>()
        service.onTransactionDetected = { events.add(it) }
        setTrackedTxids(service, setOf(trackedTxid1, trackedTxid2))

        val payload =
            """
            {
              "tracked-txs": {
                "$trackedTxid1": {
                  "utxoSpent": { "0": { "txid": "$spendingTxid", "vin": 0 } }
                },
                "$trackedTxid2": {
                  "utxoSpent": { "0": { "txid": "$spendingTxid", "vin": 0 } }
                }
              }
            }
            """.trimIndent()

        invokeHandleMessage(service, payload)
        invokeHandleMessage(service, payload)
        advanceUntilIdle()

        val outspends = events.filterIsInstance<WebSocketEvent.TrackedOutspend>()
        assertEquals(2, outspends.size)
        assertEquals(setOf(trackedTxid1, trackedTxid2), outspends.map { it.trackedTxid }.toSet())
    }

    @Test
    fun `send buffer is capped at fifty while disconnected`() {
        val service = MempoolWebSocketService()

        repeat(60) { idx ->
            invokeSend(service, "{ \"track-addresses\": [\"addr$idx\"] }")
        }

        assertEquals(50, pendingOutboundSize(service))
    }

    @Test
    fun `connect resets reconnect attempts`() {
        val service = MempoolWebSocketService()
        setReconnectAttempts(service, 5)

        service.connect()

        assertEquals(0, reconnectAttempts(service))
    }

  @Test
  fun `connect open syncs tracking and flushes queued messages`() {
    val factory = FakeWebSocketConnectionFactory()
    val service = MempoolWebSocketService(connectionFactory = factory)

    setTrackedAddresses(service, setOf("bc1qflush"))
    setTrackedTxids(service, setOf(validTxid('a')))
    invokeSend(service, "{ \"preopen\": true }")

    service.connect()
    assertFalse(service.isConnected)

    factory.callbacks?.onOpen?.invoke()

    val sent = factory.connection.sent
    val blockSub = sent.first { it.contains("mempool-blocks") }
    assertTrue(service.isConnected)
    assertTrue(sent.any { it.contains("track-addresses") })
    assertTrue(sent.any { it.contains("track-txs") })
    assertTrue(sent.any { it.contains("mempool-blocks") })
    assertFalse(blockSub.contains("\\\""))
    assertEquals("{ \"preopen\": true }", sent.last())
    assertEquals(0, pendingOutboundSize(service))
  }

  @Test
  fun `on closing schedules reconnect without attempting to echo close code`() {
    val factory = FakeWebSocketConnectionFactory()
    val service = MempoolWebSocketService(connectionFactory = factory)

    service.connect()
    factory.callbacks?.onOpen?.invoke()

    factory.callbacks?.onClosing?.invoke(1001, "going away")

    assertFalse(service.isConnected)
    assertEquals(1, reconnectAttempts(service))
    assertNull(factory.connection.closeCode)
    assertNull(factory.connection.closeReason)
  }

  @Test
  fun `on failure after open schedules reconnect`() {
    val factory = FakeWebSocketConnectionFactory()
    val service = MempoolWebSocketService(connectionFactory = factory)

    service.connect()
    factory.callbacks?.onOpen?.invoke()

    factory.callbacks?.onFailure?.invoke(IllegalStateException("boom"))

    assertFalse(service.isConnected)
    assertEquals(1, reconnectAttempts(service))
  }

  @Test
  fun `manual disconnect stops reconnect on subsequent closed callback`() {
    val factory = FakeWebSocketConnectionFactory()
    val service = MempoolWebSocketService(connectionFactory = factory)

    service.connect()
    factory.callbacks?.onOpen?.invoke()
    service.disconnect()
    factory.callbacks?.onClosed?.invoke(1000, "normal")

    assertFalse(service.isConnected)
    assertEquals(0, reconnectAttempts(service))
  }

    private fun invokeHandleMessage(service: MempoolWebSocketService, payload: String) {
        val method = service.javaClass.getDeclaredMethod("handleMessage", String::class.java)
        method.isAccessible = true
        method.invoke(service, payload)
    }

    private fun invokeSend(service: MempoolWebSocketService, payload: String) {
      val method = service.javaClass.getDeclaredMethod("send", String::class.java)
      method.isAccessible = true
      method.invoke(service, payload)
    }

    @Suppress("UNCHECKED_CAST")
    private fun setTrackedAddresses(service: MempoolWebSocketService, addresses: Set<String>) {
        val field = service.javaClass.getDeclaredField("trackedAddresses")
        field.isAccessible = true
        val set = field.get(service) as MutableSet<String>
        set.clear()
        set.addAll(addresses)
    }

    @Suppress("UNCHECKED_CAST")
    private fun setTrackedTxids(service: MempoolWebSocketService, txids: Set<String>) {
        val field = service.javaClass.getDeclaredField("trackedTxids")
        field.isAccessible = true
        val set = field.get(service) as MutableSet<String>
        set.clear()
        set.addAll(txids)
    }

    @Suppress("UNCHECKED_CAST")
    private fun pendingOutboundSize(service: MempoolWebSocketService): Int {
      val field = service.javaClass.getDeclaredField("pendingOutboundMessages")
      field.isAccessible = true
      val queue = field.get(service) as ArrayDeque<String>
      return queue.size
    }

    private fun setReconnectAttempts(service: MempoolWebSocketService, value: Int) {
      val managerField = service.javaClass.getDeclaredField("reconnectManager")
      managerField.isAccessible = true
      val manager = managerField.get(service)
      val attemptsField = manager.javaClass.getDeclaredField("reconnectAttempts")
      attemptsField.isAccessible = true
      attemptsField.setInt(manager, value)
    }

    private fun reconnectAttempts(service: MempoolWebSocketService): Int {
      val managerField = service.javaClass.getDeclaredField("reconnectManager")
      managerField.isAccessible = true
      val manager = managerField.get(service)
      val attemptsField = manager.javaClass.getDeclaredField("reconnectAttempts")
      attemptsField.isAccessible = true
      return attemptsField.getInt(manager)
    }

    private fun validTxid(char: Char): String = char.toString().repeat(64)
}

  private class FakeWebSocketConnectionFactory : WebSocketConnectionFactory {
    var callbacks: WebSocketCallbacks? = null
    val connection = FakeWebSocketConnection()

    override fun create(endpointUrl: String, callbacks: WebSocketCallbacks): WebSocketConnection {
      this.callbacks = callbacks
      return connection
    }
  }

  private class FakeWebSocketConnection : WebSocketConnection {
    val sent = mutableListOf<String>()
    var closeCode: Int? = null
    var closeReason: String? = null
    var canceled = false

    override fun send(text: String): Boolean {
      sent.add(text)
      return true
    }

    override fun close(code: Int, reason: String?): Boolean {
      closeCode = code
      closeReason = reason
      return true
    }

    override fun cancel() {
      canceled = true
    }
  }

@OptIn(ExperimentalCoroutinesApi::class)
class MainDispatcherRule(
    private val dispatcher: TestDispatcher = UnconfinedTestDispatcher()
) : TestWatcher() {
    override fun starting(description: Description) {
        Dispatchers.setMain(dispatcher)
    }

    override fun finished(description: Description) {
        Dispatchers.resetMain()
    }
}
