package com.stablechannels.app.ui.transfer

import android.app.Application
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import com.stablechannels.app.AppState
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.services.PriceService
import java.util.Date
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.lightningdevkit.ldknode.ChannelDetails
import org.mockito.ArgumentMatchers.anyLong
import org.mockito.ArgumentMatchers.anyString
import org.mockito.Mockito.after
import org.mockito.Mockito.doAnswer
import org.mockito.Mockito.mock
import org.mockito.Mockito.never
import org.mockito.Mockito.timeout
import org.mockito.Mockito.verify
import org.mockito.Mockito.`when`
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class ReceiveScreenTrustedPriceTest {
    @get:Rule val compose = createComposeRule()

    private lateinit var prices: PriceService
    private lateinit var node: NodeService
    private lateinit var state: AppState
    private val online = MutableStateFlow(true)

    @Before
    fun setUp() {
        // The state at launch: a display price loaded from the database, no fresh price yet.
        prices = PriceService().apply { seedPrice(100_000.0) }
        node = mock(NodeService::class.java)
        val channel = mock(ChannelDetails::class.java)
        `when`(channel.isChannelReady).thenReturn(true)
        `when`(node.channels).thenReturn(listOf(channel))
        // Stop at the node call; the test only checks what it was asked for.
        doAnswer { throw IllegalStateException("node not running") }
            .`when`(node)
            .receivePayment(anyLong(), anyString())
        state = mock(AppState::class.java)
        `when`(state.nodeService).thenReturn(node)
        `when`(state.priceService).thenReturn(prices)
        `when`(state.isOnline).thenReturn(online)
    }

    @Suppress("UNCHECKED_CAST")
    private fun <T> priceField(name: String): MutableStateFlow<T> =
        PriceService::class.java.getDeclaredField(name).apply { isAccessible = true }.get(prices)
            as MutableStateFlow<T>

    /** What a successful price refresh sets. */
    private fun priceRefreshed() {
        priceField<Date>("_lastUpdate").value = Date()
        priceField<Double>("_accountingPrice").value = 100_000.0
    }

    private fun askForHundredDollars() {
        compose.setContent { MaterialTheme { ReceiveScreen(state) {} } }
        compose.onNode(hasSetTextAction()).performTextInput("100")
    }

    @Test
    fun withoutAFreshPriceTheButtonIsOffAndANoteSaysWhy() {
        askForHundredDollars()

        compose.onNodeWithText(RECEIVE_WAITING_FOR_PRICE).assertIsDisplayed()
        compose.onNodeWithText("Generate Invoice").assertIsNotEnabled().performClick()
        verify(node, after(500).never()).receivePayment(anyLong(), anyString())
    }

    @Test
    fun offlineNoteAsksToReconnect() {
        online.value = false
        askForHundredDollars()

        compose.onNodeWithText(RECEIVE_OFFLINE_NO_PRICE).assertIsDisplayed()
        compose.onNodeWithText("Generate Invoice").assertIsNotEnabled()
    }

    @Test
    fun freshPricePricesTheInvoice() {
        priceRefreshed()
        askForHundredDollars()

        compose.onNodeWithText(RECEIVE_WAITING_FOR_PRICE).assertDoesNotExist()
        compose.onNodeWithText("Generate Invoice").assertIsEnabled().performClick()
        // $100 at $100,000/BTC is 100,000 sats.
        verify(node, timeout(2_000)).receivePayment(100_000_000L, "Stable Channels")
    }

    @Test
    fun firstPaymentInvoiceUsesTheTrustedPriceToo() {
        // No ready channel, so the invoice opens one (JIT).
        `when`(node.channels).thenReturn(emptyList())
        doAnswer { throw IllegalStateException("node not running") }
            .`when`(node)
            .receiveViaJitChannel(anyLong(), anyString())
        priceRefreshed()
        compose.setContent { MaterialTheme { ReceiveScreen(state) {} } }
        compose.onNode(hasSetTextAction()).performTextInput("50")

        // A price that went stale before the tap opens nothing.
        priceField<Date>("_lastUpdate").value = Date(0)
        compose.onNodeWithText("Generate Invoice").assertIsEnabled().performClick()
        compose.onNodeWithText(RECEIVE_PRICE_OUT_OF_DATE).assertIsDisplayed()
        verify(node, after(500).never()).receiveViaJitChannel(anyLong(), anyString())

        // With a fresh price it does: $50 at $100,000/BTC is 50,000 sats.
        priceRefreshed()
        compose.onNodeWithText("Generate Invoice").performClick()
        verify(node, timeout(2_000)).receiveViaJitChannel(50_000_000L, "Stable Channels")
    }

    @Test
    fun priceThatGoesStaleBeforeTheTapIsNotUsed() {
        priceRefreshed()
        askForHundredDollars()
        compose.onNodeWithText("Generate Invoice").assertIsEnabled()

        // The screen still holds the last trusted price, but it is now too old to use.
        priceField<Date>("_lastUpdate").value = Date(0)
        compose.onNodeWithText("Generate Invoice").performClick()

        compose.onNodeWithText(RECEIVE_PRICE_OUT_OF_DATE).assertIsDisplayed()
        verify(node, after(500).never()).receivePayment(anyLong(), anyString())
        verify(node, never()).receiveViaJitChannel(anyLong(), anyString())

        // The next fresh price clears the message.
        priceRefreshed()
        compose.onNodeWithText(RECEIVE_PRICE_OUT_OF_DATE).assertDoesNotExist()
        compose.onNodeWithText("Generate Invoice").assertIsEnabled()
    }
}
