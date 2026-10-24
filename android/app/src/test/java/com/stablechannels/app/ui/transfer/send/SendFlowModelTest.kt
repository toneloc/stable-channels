package com.stablechannels.app.ui.transfer.send

import com.stablechannels.app.domain.lnurl.LnurlDescriptionBinding
import com.stablechannels.app.domain.lnurl.LnurlInvoiceQuote
import com.stablechannels.app.domain.lnurl.LnurlPayClient
import com.stablechannels.app.domain.lnurl.LnurlPayParams
import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.DecodedBolt11
import com.stablechannels.app.domain.send.DecodedBolt12
import com.stablechannels.app.domain.send.InvoicePayability
import com.stablechannels.app.domain.send.NetworkFeeSpeedTier
import com.stablechannels.app.domain.send.OutboundPaymentStatus
import com.stablechannels.app.domain.send.PaymentDestinationClassifier
import com.stablechannels.app.domain.send.RecommendedFees
import com.stablechannels.app.domain.send.SendAmountUnit
import com.stablechannels.app.domain.send.SendError
import com.stablechannels.app.domain.send.SendFlowStep
import com.stablechannels.app.domain.send.SendSuccessStatus
import com.stablechannels.app.services.PaymentOutcome
import com.stablechannels.app.services.send.FakeInvoiceDecoder
import com.stablechannels.app.services.send.FakeSendNode
import com.stablechannels.app.services.send.FakeSendRecorder
import com.stablechannels.app.services.send.FakeSendWallet
import com.stablechannels.app.services.send.PaymentSettlementObserver
import com.stablechannels.app.services.send.SendPaymentExecutor
import com.stablechannels.app.services.send.balances
import com.stablechannels.app.services.send.readyChannel
import java.net.URI
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private const val NOW = 1_000_000L
private const val ADDRESS = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"

private class FakeLnurl : LnurlPayClient {
    var paramsGate: CompletableDeferred<Unit>? = null
    var expiresAt = NOW + 600
    var invoiceCalls = 0

    override suspend fun fetchPayParams(url: URI): LnurlPayParams {
        paramsGate?.await()
        return LnurlPayParams("payRequest", "https://example.com/cb", 1_000, 1_000_000_000, "[]")
    }

    override suspend fun fetchInvoice(
        params: LnurlPayParams,
        amountMsat: Long,
        comment: String?,
    ): LnurlInvoiceQuote {
        invoiceCalls++
        return LnurlInvoiceQuote(
            "lntb-q$invoiceCalls",
            amountMsat,
            comment,
            LnurlDescriptionBinding.MatchesMetadata,
            expiresAt,
        )
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class SendFlowModelTest {
    private val dispatcher = StandardTestDispatcher()
    private val wallet = FakeSendWallet(network = BitcoinNetwork.TESTNET)
    private val node = FakeSendNode()
    private val lnurl = FakeLnurl()
    private val outcomes = MutableStateFlow<Map<String, PaymentOutcome>>(emptyMap())
    private var authorized = true
    private val decoder =
        FakeInvoiceDecoder(
            invoices =
                mutableMapOf(
                    "lntb-fixed" to fixed(50_000),
                    "lntb-open" to fixed(null),
                ),
            offers = mutableMapOf("lno1test" to DecodedBolt12(setOf(BitcoinNetwork.TESTNET))),
        )
    private val fees =
        RecommendedFees(fastestFee = 20.0, halfHourFee = 10.0, hourFee = 5.0, minimumFee = 1.0)

    private fun fixed(msat: Long?) =
        DecodedBolt11(
            BitcoinNetwork.TESTNET,
            msat,
            expiresAtEpochSecs = NOW + 3_600,
            paymentHash = "hash",
        )

    private val model by lazy {
        SendFlowModel(
            wallet = wallet,
            executor =
                SendPaymentExecutor(
                    node,
                    wallet,
                    FakeSendRecorder(),
                    decoder,
                    { NOW },
                    { _, _ -> },
                ),
            settlement = PaymentSettlementObserver(outcomes, { _, _ -> }),
            classifier = PaymentDestinationClassifier(decoder),
            decoder = decoder,
            lnurlClientFor = { lnurl },
            authenticator = { authorized },
            ensureLspConnected = {},
            io = dispatcher,
            nowEpochSecs = { NOW },
            nanoTime = { 0L },
        )
    }

    @Before fun setUp() = Dispatchers.setMain(dispatcher)

    @After fun tearDown() = Dispatchers.resetMain()

    private fun test(block: suspend TestScope.() -> Unit) = runTest(dispatcher) { block() }

    private fun settle(succeeded: Boolean, message: String = "done") = outcomes.update {
        it + ("pid-1" to PaymentOutcome(succeeded, message, observedAtNanos = 1))
    }

    private fun lightningFunds(sats: Long = 100_000) {
        wallet.snapshot = balances(lightningSats = sats, channels = listOf(readyChannel(sats)))
    }

    private fun TestScope.proceed(input: String) {
        model.setInput(input)
        model.startProceedFromRecipient()
        advanceUntilIdle()
    }

    private fun TestScope.enterSats(amount: String) {
        model.switchUnit(SendAmountUnit.SATS)
        model.setAmountText(amount)
        model.proceedFromAmount()
        advanceUntilIdle()
    }

    private val state
        get() = model.state.value

    // Each destination reaches the receipt

    @Test
    fun `fixed invoice goes straight to confirm and completes`() = test {
        lightningFunds()
        node.onLightningDispatch = { settle(true) }

        proceed("lntb-fixed")
        assertEquals(SendFlowStep.CONFIRM, state.step)
        model.send()
        advanceUntilIdle()

        assertEquals(SendFlowStep.SUCCESS, state.step)
        assertEquals(SendSuccessStatus.COMPLETED, state.receipt?.status)
        assertEquals(50L, state.receipt?.sentAmountSats)
    }

    @Test
    fun `offer is sent at the entered amount`() = test {
        lightningFunds()
        node.onLightningDispatch = { settle(true) }

        proceed("lno1test")
        enterSats("700")
        model.send()
        advanceUntilIdle()

        assertEquals("payBolt12 lno1test 700000", node.calls.last())
        assertEquals(SendSuccessStatus.COMPLETED, state.receipt?.status)
    }

    @Test
    fun `lightning address pays the invoice quoted on confirm`() = test {
        lightningFunds()
        node.onLightningDispatch = { settle(true) }

        proceed("alice@example.com")
        assertEquals(SendFlowStep.AMOUNT, state.step)
        enterSats("21")
        assertEquals("lntb-q1", state.lnurlQuote?.invoice)
        model.send()
        advanceUntilIdle()

        assertEquals("payBolt11 lntb-q1 21000", node.calls.last())
        assertEquals(SendSuccessStatus.COMPLETED, state.receipt?.status)
    }

    @Test
    fun `onchain send awaits confirmation with its txid`() = test {
        wallet.snapshot = balances(onchainSats = 100_000)
        model.applyRecommendedFees(fees)

        proceed(ADDRESS)
        enterSats("40000")
        model.send()
        advanceUntilIdle()

        assertEquals("sendOnchain $ADDRESS 40000 10.0", node.calls[0])
        assertEquals(SendSuccessStatus.AWAITING_CONFIRMATION, state.receipt?.status)
        assertEquals("txid-1", state.receipt?.txid)
    }

    @Test
    fun `splice-out receipt stays pending`() = test {
        lightningFunds()
        model.applyRecommendedFees(fees)

        proceed(ADDRESS)
        enterSats("40000")
        model.send()
        advanceUntilIdle()

        assertTrue(node.calls.any { it.startsWith("spliceOut") })
        assertEquals(SendSuccessStatus.PENDING, state.receipt?.status)
    }

    // Sending safety

    @Test
    fun `a double slide sends once`() = test {
        lightningFunds()
        node.onLightningDispatch = { settle(true) }
        proceed("lntb-fixed")

        model.send()
        model.send()
        advanceUntilIdle()

        assertEquals(1, node.calls.count { it.startsWith("payBolt11") })
    }

    @Test
    fun `cancelling during loading drops the lookup`() = test {
        lnurl.paramsGate = CompletableDeferred()
        model.setInput("alice@example.com")
        model.startProceedFromRecipient()
        runCurrent()
        assertTrue(state.isFetchingLnurl)

        model.cancelProceedFromRecipient()
        lnurl.paramsGate?.complete(Unit)
        advanceUntilIdle()

        assertEquals(SendFlowStep.RECIPIENT, state.step)
        assertFalse(state.isFetchingLnurl)
        assertNull(state.lnurlParams)
    }

    @Test
    fun `offline blocks the send before the node`() = test {
        lightningFunds()
        proceed("lntb-fixed")
        wallet.isOnline = false

        model.send()
        advanceUntilIdle()

        assertEquals(SendError.Offline, state.error)
        assertTrue(node.calls.none { it.startsWith("pay") })
    }

    @Test
    fun `denied authentication blocks the send`() = test {
        lightningFunds()
        proceed("lntb-fixed")
        authorized = false

        model.send()
        advanceUntilIdle()

        assertEquals(SendError.AuthenticationRequired, state.error)
        assertTrue(node.calls.none { it.startsWith("pay") })
    }

    @Test
    fun `an already paid invoice is refused`() = test {
        lightningFunds()
        wallet.outbound = mapOf("hash" to OutboundPaymentStatus.SUCCEEDED)

        proceed("lntb-fixed")

        assertEquals(SendError.InvoiceNotPayable(InvoicePayability.ALREADY_PAID), state.error)
        assertEquals(SendFlowStep.RECIPIENT, state.step)
    }

    @Test
    fun `an expired lnurl quote is replaced, never paid`() = test {
        lightningFunds()
        lnurl.expiresAt = NOW
        proceed("alice@example.com")
        enterSats("21")

        model.send()
        advanceUntilIdle()

        assertEquals(SendError.LnurlInvoiceRefreshed, state.error)
        assertTrue(node.calls.none { it.startsWith("pay") })
        assertEquals(2, lnurl.invoiceCalls)
    }

    // Settlement

    @Test
    fun `a settlement before the observer listens still completes`() = test {
        lightningFunds()
        // The node reports success while the send call is still returning.
        node.onLightningDispatch = { settle(true) }
        proceed("lntb-fixed")

        model.send()
        advanceUntilIdle()

        assertEquals(SendSuccessStatus.COMPLETED, state.receipt?.status)
    }

    @Test
    fun `a pending receipt completes when the payment settles`() = test {
        lightningFunds()
        proceed("lntb-fixed")
        model.send()
        advanceTimeBy(7_001)
        runCurrent()
        assertEquals(SendSuccessStatus.PENDING, state.receipt?.status)

        settle(true)
        runCurrent()

        assertEquals(SendSuccessStatus.COMPLETED, state.receipt?.status)
    }

    @Test
    fun `a pending receipt that fails returns to confirm with the reason`() = test {
        lightningFunds()
        proceed("lntb-fixed")
        model.send()
        advanceTimeBy(7_001)
        runCurrent()

        settle(false, "No route")
        runCurrent()

        assertEquals(SendFlowStep.CONFIRM, state.step)
        assertEquals(SendError.PaymentFailed("No route"), state.error)
        assertNull(state.receipt)
    }

    // Max and Send All

    @Test
    fun `max follows the fee of each tier`() = test {
        wallet.snapshot = balances(onchainSats = 100_000)
        model.applyRecommendedFees(fees)
        proceed(ADDRESS)
        model.applyPercentage(100)

        // A send-all transaction is 250 vB: economy 5, standard 10, priority 20 sat/vB.
        val expected =
            mapOf(
                NetworkFeeSpeedTier.ECONOMY to 98_750L,
                NetworkFeeSpeedTier.STANDARD to 97_500L,
                NetworkFeeSpeedTier.PRIORITY to 95_000L,
            )
        for ((tier, sats) in expected) {
            model.selectFeeTier(tier)
            assertEquals(sats, model.quote()?.amountSats)
        }

        model.proceedFromAmount()
        model.send()
        advanceUntilIdle()

        assertEquals("sendAllOnchain $ADDRESS 20.0", node.calls[0])
        assertEquals(95_000L, state.receipt?.sentAmountSats)
    }

    @Test
    fun `settings send all empties the wallet even with a channel`() = test {
        wallet.snapshot =
            balances(
                lightningSats = 90_000,
                onchainSats = 30_000,
                channels = listOf(readyChannel(90_000)),
            )
        model.applyRecommendedFees(fees)
        model.startSendAllFromOnchainWallet(ADDRESS)
        model.goTo(SendFlowStep.CONFIRM)

        model.send()
        advanceUntilIdle()

        assertEquals("sendAllOnchain $ADDRESS 10.0", node.calls[0])
        assertTrue(node.calls.none { it.startsWith("spliceOut") })
        assertEquals(30_000L - 2_500, state.receipt?.sentAmountSats)
    }

    // Amount entry

    @Test
    fun `a share of the balance uses what this destination can spend`() = test {
        // 60,000 sats over Lightning; the 500,000 on-chain sats cannot pay an invoice.
        wallet.snapshot =
            balances(
                lightningSats = 60_000,
                onchainSats = 500_000,
                channels = listOf(readyChannel(60_000)),
            )
        proceed("lno1test")
        model.switchUnit(SendAmountUnit.SATS)

        model.applyPercentage(50)

        assertEquals("30000", state.amountInputText)
    }

    @Test
    fun `a cleared bip21 amount is filled again`() = test {
        wallet.snapshot = balances(onchainSats = 100_000)
        model.setInput("bitcoin:$ADDRESS?amount=0.0001")
        model.setAmountText("")

        model.startProceedFromRecipient()
        advanceUntilIdle()

        assertEquals(SendAmountUnit.SATS, state.amountUnit)
        assertEquals("10000", state.amountInputText)
    }

    @Test
    fun `reset keeps the fee rates and the chosen tier`() = test {
        model.applyRecommendedFees(fees)
        model.selectFeeTier(NetworkFeeSpeedTier.PRIORITY)
        proceed(ADDRESS)

        model.resetFlow()

        assertEquals(SendFlowStep.RECIPIENT, state.step)
        assertEquals("", state.inputText)
        assertEquals(NetworkFeeSpeedTier.PRIORITY, state.selectedFeeTier)
        assertEquals(fees, state.recommendedFees)
    }
}
