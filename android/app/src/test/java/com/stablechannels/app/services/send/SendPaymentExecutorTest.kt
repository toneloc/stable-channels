package com.stablechannels.app.services.send

import com.stablechannels.app.domain.lnurl.LnurlDescriptionBinding
import com.stablechannels.app.domain.lnurl.LnurlInvoiceQuote
import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.DecodedBolt11
import com.stablechannels.app.domain.send.FakeInvoiceDecoder
import com.stablechannels.app.domain.send.FakeSendNode
import com.stablechannels.app.domain.send.FakeSendRecorder
import com.stablechannels.app.domain.send.FakeSendWallet
import com.stablechannels.app.domain.send.InvoicePayability
import com.stablechannels.app.domain.send.SendDestination
import com.stablechannels.app.domain.send.SendError
import com.stablechannels.app.domain.send.SendFailure
import com.stablechannels.app.domain.send.SendPaymentOutcome
import com.stablechannels.app.domain.send.SendPaymentResult
import com.stablechannels.app.domain.send.balances
import com.stablechannels.app.domain.send.readyChannel
import java.net.URI
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class SendPaymentExecutorTest {
    private val node = FakeSendNode()
    private val wallet = FakeSendWallet()
    private val recorder = FakeSendRecorder()
    private val decoder = FakeInvoiceDecoder()
    private val audits = mutableListOf<String>()
    private val executor =
        SendPaymentExecutor(
            node,
            node,
            wallet,
            recorder,
            decoder,
            { NOW },
            { event, _ -> audits += event },
        )

    private companion object {
        const val NOW = 1_000_000L
        const val ADDRESS = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        val LN_ADDRESS =
            SendDestination.LightningAddress("alice", "example.com", URI("https://example.com"))
    }

    private fun assertRejected(expected: SendError, block: () -> Unit) {
        try {
            block()
            fail("Expected $expected")
        } catch (e: SendFailure) {
            assertEquals(expected, e.error)
        }
    }

    private fun quote(binding: LnurlDescriptionBinding = LnurlDescriptionBinding.MatchesMetadata) =
        LnurlInvoiceQuote("lnbc-quoted", 21_000, null, binding, NOW + 600)

    @Test
    fun `fixed amount invoice is paid as issued and recorded`() {
        val result = executor.execute(SendDestination.Bolt11("lnbc-fixed", 50_000), 0, null, null)

        assertEquals(listOf("surplusCheck 50000", "payBolt11 lnbc-fixed null"), node.calls)
        assertEquals(listOf("bolt11 pid-1 50000"), recorder.rows)
        assertEquals(SendPaymentResult(50, SendPaymentOutcome.Lightning("pid-1")), result)
    }

    @Test
    fun `amountless invoice is paid at the entered amount`() {
        executor.execute(SendDestination.Bolt11("lnbc-open", null), 1_234, null, null)

        assertEquals("payBolt11 lnbc-open 1234000", node.calls.last())
    }

    @Test
    fun `expired invoice is rejected before paying`() {
        decoder.invoices["lnbc-old"] =
            DecodedBolt11(BitcoinNetwork.TESTNET, 1_000, expiresAtEpochSecs = NOW)

        assertRejected(SendError.InvoiceNotPayable(InvoicePayability.EXPIRED)) {
            executor.execute(SendDestination.Bolt11("lnbc-old", 1_000), 1, null, null)
        }
        assertTrue(node.calls.isEmpty())
    }

    @Test
    fun `zero amount is rejected for amountless lightning`() {
        assertRejected(SendError.InvalidAmount) {
            executor.execute(SendDestination.Bolt12("lno1"), 0, null, null)
        }
        assertTrue(node.calls.isEmpty())
    }

    @Test
    fun `offer is paid and recorded as bolt12`() {
        val result = executor.execute(SendDestination.Bolt12("lno1"), 700, null, null)

        assertEquals("payBolt12 lno1 700000", node.calls.last())
        assertEquals(listOf("bolt12 pid-1 700000"), recorder.rows)
        assertEquals(700L, result.sentAmountSats)
    }

    @Test
    fun `lnurl pays exactly the quoted invoice`() {
        val result = executor.execute(LN_ADDRESS, 999_999, null, quote())

        assertEquals("payBolt11 lnbc-quoted 21000", node.calls.last())
        assertEquals(listOf("lnurl pid-1 21000"), recorder.rows)
        assertEquals(21L, result.sentAmountSats)
        assertTrue(audits.isEmpty())
    }

    @Test
    fun `lnurl invoice without matching metadata is audited`() {
        executor.execute(LN_ADDRESS, 21, null, quote(LnurlDescriptionBinding.Direct))

        assertEquals(listOf("LNURL_UNBOUND_INVOICE_PAID"), audits)
    }

    @Test
    fun `lnurl without a quote never pays`() {
        assertRejected(SendError.MissingLnurlInvoice) {
            executor.execute(LN_ADDRESS, 21, null, null)
        }
        assertTrue(node.calls.isEmpty())
    }

    @Test
    fun `onchain without a channel broadcasts at the chosen rate`() {
        wallet.snapshot = balances(onchainSats = 100_000)

        val result = executor.execute(SendDestination.Onchain(ADDRESS, null), 40_000, 3.5, null)

        assertEquals(
            listOf("sendOnchain $ADDRESS 40000 3.5", "broadcast 40000 false txid-1"),
            node.calls,
        )
        assertEquals(listOf("onchain txid-1 40000000 $ADDRESS"), recorder.rows)
        assertEquals(SendPaymentOutcome.Onchain("txid-1"), result.outcome)
    }

    @Test
    fun `onchain with a ready channel splices out of the best channel and tracks that channel`() {
        wallet.snapshot =
            balances(
                lightningSats = 900_000,
                channels = listOf(readyChannel(900_000, "big"), readyChannel(60_000, "small")),
            )

        val result = executor.execute(SendDestination.Onchain(ADDRESS, null), 50_000, 2.0, null)

        assertEquals(
            listOf("beginSpliceOut small 50000", "spliceOut small $ADDRESS 50000"),
            node.calls,
        )
        assertEquals(SendPaymentResult(50_000, SendPaymentOutcome.SpliceOut), result)
        assertTrue(recorder.rows.isEmpty())
    }

    @Test
    fun `failed splice start is rolled back`() {
        wallet.snapshot = balances(lightningSats = 90_000, channels = listOf(readyChannel(90_000)))
        node.spliceFailure = IllegalStateException("busy")

        try {
            executor.execute(SendDestination.Onchain(ADDRESS, null), 10_000, 2.0, null)
            fail("Expected the node error")
        } catch (e: IllegalStateException) {
            assertEquals("busy", e.message)
        }
        assertEquals("cancelSpliceStart", node.calls.last())
    }

    @Test
    fun `no onchain send while a sweep holds the funds`() {
        wallet.snapshot =
            balances(
                onchainSats = 90_000,
                channels = listOf(readyChannel(90_000)),
                isSweeping = true,
            )

        assertRejected(SendError.SpliceInProgress) {
            executor.execute(SendDestination.Onchain(ADDRESS, null), 10_000, 2.0, null)
        }
        assertRejected(SendError.SpliceInProgress) { executor.sendAllOnchain(ADDRESS, 2.0) }
        assertTrue(node.calls.isEmpty())
    }

    @Test
    fun `send all empties the wallet even with a ready channel`() {
        wallet.snapshot =
            balances(
                lightningSats = 90_000,
                onchainSats = 30_000,
                channels = listOf(readyChannel(90_000)),
            )

        val result = executor.sendAllOnchain(ADDRESS, 4.0)

        assertEquals(
            listOf("sendAllOnchain $ADDRESS 4.0", "broadcast 30000 true txid-1"),
            node.calls,
        )
        assertEquals(listOf("onchain txid-1 30000000 $ADDRESS"), recorder.rows)
        assertEquals(30_000L, result.sentAmountSats)
    }
}
