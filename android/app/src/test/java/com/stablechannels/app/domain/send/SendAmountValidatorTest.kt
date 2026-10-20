package com.stablechannels.app.domain.send

import java.net.URI
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SendAmountValidatorTest {
    private fun channel(id: String, outboundSats: Long, isReady: Boolean = true) =
        SpliceChannelCandidate(
            channelId = id,
            userChannelId = id,
            counterpartyNodeId =
                "020202020202020202020202020202020202020202020202020202020202020202",
            outboundCapacityMsat = outboundSats * 1_000,
            isReady = isReady,
        )

    private fun balances(
        channels: List<SpliceChannelCandidate> = emptyList(),
        lightningSats: Long = 0,
        onchainSats: Long = 0,
        isSweeping: Boolean = false,
    ) =
        SendBalanceSnapshot(
            channels,
            lightningSats,
            onchainSats,
            lightningSats + onchainSats,
            isSweeping,
        )

    private val onchain =
        SendDestination.Onchain("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", null)
    private val bolt11 = SendDestination.Bolt11("lnbc1test", null)
    private val lnurl = SendDestination.LnurlPay(URI("https://example.com/lnurl"))

    private fun quote(request: SendAmountRequest, snapshot: SendBalanceSnapshot) =
        SendAmountValidator.quote(request, snapshot)

    private fun check(request: SendAmountRequest, snapshot: SendBalanceSnapshot): SendError? =
        SendAmountValidator.validate(request, quote(request, snapshot))

    // Quote

    @Test
    fun `lightning available is summed ready outbound`() {
        val snapshot =
            balances(
                channels =
                    listOf(
                        channel("a", 30_000),
                        channel("b", 50_000),
                        channel("c", 90_000, isReady = false),
                    ),
                lightningSats = 100_000,
                onchainSats = 10_000,
            )
        val q = quote(SendAmountRequest(bolt11, 10_000, 10.0), snapshot)

        assertEquals(80_000L, q.availableSats)
        // 10,000,000 msat / 100 + 50,000 msat = 150 sats
        assertEquals(150L, q.feeSats)
        assertEquals(10_150L, q.totalDebitSats)
    }

    @Test
    fun `lightning available is bounded by lightning balance`() {
        val snapshot = balances(listOf(channel("a", 80_000)), lightningSats = 60_000)

        assertEquals(60_000L, quote(SendAmountRequest(bolt11, 1_000, 1.0), snapshot).availableSats)
    }

    @Test
    fun `onchain splice out uses splice fee and largest single channel`() {
        val snapshot =
            balances(
                listOf(channel("a", 30_000), channel("b", 50_000)),
                lightningSats = 100_000,
                onchainSats = 10_000,
            )
        val q = quote(SendAmountRequest(onchain, 20_000, 10.0), snapshot)

        assertTrue(q.isSpliceOut)
        assertEquals(50_000L, q.availableSats)
        assertEquals(PaymentFeeEstimator.estimateSpliceOutFee(10.0), q.feeSats)
        assertEquals(21_800L, q.totalDebitSats)
        assertFalse(q.sendsEntireOnchainBalance)
    }

    @Test
    fun `sweeping falls back to onchain wallet`() {
        val snapshot =
            balances(
                listOf(channel("a", 50_000)),
                lightningSats = 50_000,
                onchainSats = 7_000,
                isSweeping = true,
            )
        val q = quote(SendAmountRequest(onchain, 1_000, 10.0), snapshot)

        assertFalse(q.isSpliceOut)
        assertEquals(7_000L, q.availableSats)
        assertEquals(PaymentFeeEstimator.estimateOnchainFee(10.0, isSendAll = false), q.feeSats)
        assertEquals(2_400L, q.totalDebitSats)
    }

    @Test
    fun `max onchain wallet uses send all vbytes and spends exactly the balance`() {
        val q =
            quote(
                SendAmountRequest(onchain, 0, 10.0, isMax = true),
                balances(onchainSats = 100_000),
            )

        assertEquals(PaymentFeeEstimator.estimateOnchainFee(10.0, isSendAll = true), q.feeSats)
        assertEquals(97_500L, q.amountSats)
        assertEquals(q.availableSats, q.amountSats + q.feeSats)
        assertEquals(100_000L, q.totalDebitSats)
        assertTrue(q.sendsEntireOnchainBalance)
    }

    @Test
    fun `max with payee max caps only the amount`() {
        val snapshot = balances(listOf(channel("a", 100_000)), lightningSats = 100_000)
        val q =
            quote(
                SendAmountRequest(
                    lnurl,
                    0,
                    1.0,
                    isMax = true,
                    payeeLimits = PayeeAmountLimits(1, 50_000),
                ),
                snapshot,
            )

        assertEquals(50_000L, q.amountSats)
        // The fee on the capped amount is still paid from the balance: 550 sats.
        assertEquals(550L, q.feeSats)
        assertEquals(50_550L, q.totalDebitSats)
        assertEquals(100_000L, q.availableSats)
    }

    @Test
    fun `max with payee max above balance is fee inclusive max`() {
        val snapshot = balances(listOf(channel("a", 100_000)), lightningSats = 100_000)
        val q =
            quote(
                SendAmountRequest(
                    lnurl,
                    0,
                    1.0,
                    isMax = true,
                    payeeLimits = PayeeAmountLimits(1, 200_000),
                ),
                snapshot,
            )

        assertTrue(q.amountSats < 100_000)
        assertTrue(q.totalDebitSats <= q.availableSats)
        // One more sat would not fit.
        val next = q.amountSats + 1
        assertTrue(next + PaymentFeeEstimator.maxLightningRoutingFeeSats(next) > q.availableSats)
    }

    @Test
    fun `send all from onchain wallet sends balance less send all fee`() {
        val snapshot =
            balances(listOf(channel("a", 80_000)), lightningSats = 80_000, onchainSats = 50_000)
        val q =
            quote(SendAmountRequest(onchain, 0, 10.0, sendsAllFromOnchainWallet = true), snapshot)

        val fee = PaymentFeeEstimator.estimateOnchainFee(10.0, isSendAll = true)
        assertFalse(q.isSpliceOut)
        assertEquals(50_000L, q.availableSats)
        assertEquals(50_000L - fee, q.amountSats)
        assertEquals(fee, q.feeSats)
        assertEquals(50_000L, q.totalDebitSats)
    }

    @Test
    fun `send all from onchain wallet smaller than fee sends zero`() {
        val q =
            quote(
                SendAmountRequest(onchain, 0, 10.0, sendsAllFromOnchainWallet = true),
                balances(onchainSats = 1_000),
            )

        assertEquals(0L, q.amountSats)
        assertEquals(1_000L, q.totalDebitSats)
    }

    @Test
    fun `fixed amount bolt11 fee comes from exact msat`() {
        val invoice = SendDestination.Bolt11("lnbc1fixed", amountMsat = 100_000_999)
        val snapshot = balances(listOf(channel("a", 200_000)), lightningSats = 200_000)
        val q = quote(SendAmountRequest(invoice, 100_000, 1.0), snapshot)

        // 1,050,009 msat rounds up to 1,051, not the 1,050 of the whole-sat amount.
        assertEquals(1_051L, q.feeSats)
        // Amount and fee are rounded up together: 101,051,008 msat is 101,052 sats.
        assertEquals(101_052L, q.totalDebitSats)
    }

    @Test
    fun `quoted amount overrides entered and max`() {
        val snapshot = balances(listOf(channel("a", 100_000)), lightningSats = 100_000)

        val entered =
            quote(SendAmountRequest(lnurl, 1_000, 1.0, quotedAmountSats = 2_000), snapshot)
        assertEquals(2_000L, entered.amountSats)
        assertEquals(70L, entered.feeSats)
        assertEquals(2_070L, entered.totalDebitSats)

        val max =
            quote(
                SendAmountRequest(lnurl, 0, 1.0, isMax = true, quotedAmountSats = 2_000),
                snapshot,
            )
        assertEquals(2_000L, max.amountSats)
    }

    // Validate

    @Test
    fun `nothing spendable is insufficient balance of zero`() {
        assertEquals(
            SendError.InsufficientBalance(0),
            check(SendAmountRequest(bolt11, 1_000, 1.0), balances()),
        )
    }

    @Test
    fun `zero amount is invalid`() {
        assertEquals(
            SendError.InvalidAmount,
            check(SendAmountRequest(onchain, 0, 10.0), balances(onchainSats = 10_000)),
        )
    }

    @Test
    fun `amount outside payee limits is rejected before balance`() {
        val snapshot = balances(listOf(channel("a", 1_000)), lightningSats = 1_000)
        val limits = PayeeAmountLimits(1_000, 5_000)
        val expected = SendError.OutsidePayeeLimits(1_000, 5_000)

        assertEquals(
            expected,
            check(SendAmountRequest(lnurl, 500, 1.0, payeeLimits = limits), snapshot),
        )
        assertEquals(
            expected,
            check(SendAmountRequest(lnurl, 6_000, 1.0, payeeLimits = limits), snapshot),
        )
    }

    @Test
    fun `amount above balance exceeds balance`() {
        assertEquals(
            SendError.AmountExceedsBalance(10_000),
            check(SendAmountRequest(onchain, 10_001, 10.0), balances(onchainSats = 10_000)),
        )
    }

    @Test
    fun `amount fits but amount plus fee does not is insufficient balance`() {
        // 9,000 + 1,400 fee > 10,000
        assertEquals(
            SendError.InsufficientBalance(10_000),
            check(SendAmountRequest(onchain, 9_000, 10.0), balances(onchainSats = 10_000)),
        )
    }

    @Test
    fun `amount plus fee equal to balance is valid`() {
        // 8,600 + 1,400 fee == 10,000
        assertNull(check(SendAmountRequest(onchain, 8_600, 10.0), balances(onchainSats = 10_000)))
    }

    @Test
    fun `fixed amount bolt11 checks the exact msat fee`() {
        // 100,000 + 1,050 fits 101,050, but the exact-msat fee of 1,051 does not.
        val invoice = SendDestination.Bolt11("lnbc1fixed", amountMsat = 100_000_999)
        val snapshot = balances(listOf(channel("a", 101_050)), lightningSats = 101_050)

        assertEquals(
            SendError.InsufficientBalance(101_050),
            check(SendAmountRequest(invoice, 100_000, 1.0), snapshot),
        )
    }

    @Test
    fun `fixed amount bolt11 one sat short of the rounded debit is rejected`() {
        val invoice = SendDestination.Bolt11("lnbc1fixed", amountMsat = 100_000_999)
        val request = SendAmountRequest(invoice, 100_000, 1.0)
        val tight = balances(listOf(channel("a", 200_000)), lightningSats = 101_051)
        val enough = balances(listOf(channel("a", 200_000)), lightningSats = 101_052)

        assertEquals(SendError.InsufficientBalance(101_051), check(request, tight))
        assertNull(check(request, enough))
    }

    @Test
    fun `send all from onchain wallet is never rejected here`() {
        val request = SendAmountRequest(onchain, 0, 10.0, sendsAllFromOnchainWallet = true)

        assertNull(check(request, balances(onchainSats = 0)))
        assertNull(check(request, balances(onchainSats = 500)))
    }
}
