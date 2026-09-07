package com.stablechannels.app

import org.junit.Assert.assertEquals
import org.lightningdevkit.ldknode.ConfirmationStatus
import org.lightningdevkit.ldknode.PaymentDetails
import org.lightningdevkit.ldknode.PaymentDirection
import org.lightningdevkit.ldknode.PaymentKind
import org.lightningdevkit.ldknode.PaymentStatus
import org.junit.Test

/**
 * Pins the fix for gpt-5-codex's review of PR #266: OnchainTxidMatcherTest's own
 * "resolving twice as LDK timestamps shift" test demonstrates that latestUpdateTimestamp is a
 * mutable last-modified time, not an immutable creation time — LDK can update it on later polls
 * (e.g. when a payment confirms), silently shifting the cost of an already-committed match.
 * AppState.onchainMatchTimestamp() closes that specific instability by preferring the confirmed
 * block's timestamp — set once by consensus and immutable thereafter — whenever it's available,
 * falling back to latestUpdateTimestamp only pre-confirmation where nothing better exists yet.
 */
class OnchainMatchTimestampTest {

    private fun paymentWithStatus(kind: PaymentKind, latestUpdateTimestamp: Long): PaymentDetails =
        PaymentDetails(
            id = "id1",
            kind = kind,
            amountMsat = 1_000uL,
            feePaidMsat = 0uL,
            direction = PaymentDirection.INBOUND,
            status = PaymentStatus.SUCCEEDED,
            latestUpdateTimestamp = latestUpdateTimestamp.toULong()
        )

    @Test
    fun confirmedPaymentUsesImmutableBlockTimestampNotLatestUpdateTimestamp() {
        val kind = PaymentKind.Onchain(
            txid = "tx1",
            status = ConfirmationStatus.Confirmed(
                blockHash = "0000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                height = 800_000u,
                timestamp = 12_345uL
            )
        )
        // latestUpdateTimestamp differs sharply from the block time — simulates LDK bumping it
        // on a later poll after the payment confirms. The block timestamp must win.
        val payment = paymentWithStatus(kind, latestUpdateTimestamp = 99_999L)

        assertEquals(12_345L, AppState.onchainMatchTimestamp(payment))
    }

    @Test
    fun confirmedPaymentCostIsStableAcrossRepeatedCallsEvenIfLatestUpdateTimestampMoves() {
        val kind = PaymentKind.Onchain(
            txid = "tx1",
            status = ConfirmationStatus.Confirmed(
                blockHash = "0000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                height = 800_000u,
                timestamp = 500_000uL
            )
        )
        val firstPoll = paymentWithStatus(kind, latestUpdateTimestamp = 500_050L)
        val secondPoll = paymentWithStatus(kind, latestUpdateTimestamp = 900_000L)

        // Same underlying confirmed payment polled twice with a shifted latestUpdateTimestamp
        // (exactly the OnchainTxidMatcherTest scenario) must yield the identical match cost.
        assertEquals(
            AppState.onchainMatchTimestamp(firstPoll),
            AppState.onchainMatchTimestamp(secondPoll)
        )
    }

    @Test
    fun unconfirmedPaymentFallsBackToLatestUpdateTimestamp() {
        val kind = PaymentKind.Onchain(txid = "tx1", status = ConfirmationStatus.Unconfirmed)
        val payment = paymentWithStatus(kind, latestUpdateTimestamp = 42_000L)

        assertEquals(42_000L, AppState.onchainMatchTimestamp(payment))
    }
}
