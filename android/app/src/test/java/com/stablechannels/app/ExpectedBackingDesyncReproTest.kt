package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.StabilityService
import com.stablechannels.app.util.Constants
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File
import kotlin.math.abs

/**
 * Replication of issue #296: `handlePaymentSuccessful()` reconciles an ordinary
 * Lightning send with `StabilityService.reconcileOutgoing` (which reduces BOTH
 * `expectedUSD` and `backingSats` in lockstep) but persists via
 * `saveChannelToDB(preserveBacking = true)` → `saveChannelPreservingBacking`,
 * which writes only `expected_usd`. After a process restart reloads the row,
 * the wallet holds a reduced target paired with the stale, unreduced backing.
 * A later stability check then computes a phantom above-par excess and pays it.
 *
 * This test drives the production persistence sequence with the real DB and the
 * exact figures from the field incident (channel 310bf6c2…, 2026-09-10) and
 * asserts the invariant the code is supposed to keep: the persisted
 * (expectedUSD, backingSats) pair must stay consistent at the write price
 * within the $0.25 stability floor. On a build with the bug it fails with the
 * incident's numbers — a ~$3.9 divergence after the first send, and a phantom
 * settlement of ~$8.53 at the end.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ExpectedBackingDesyncReproTest {
    private lateinit var db: DatabaseService

    private val channelId = "310bf6c20000000000000000000000000000000000000000000000000000abcd"
    private val userChannelId = "152086427531756288893474821763924984092"

    // Field incident figures (issue #296).
    private val tradeExpectedUsd = 28.5912
    private val tradeBackingSats = 36_418L
    private val priceAtSend1 = 78_491.2175
    private val priceAtSend2 = 78_400.45
    private val priceAtCheck = 78_221.6
    private val liveSatsAfterSend1 = 31_461L   // send #1: 6,370 sats + fee
    private val liveSatsAfterSend2 = 30_357L   // send #2: 1,100 sats + fee
    private val stabilityDollarFloor = 0.25    // legitimate drift never exceeds this here

    @Before
    fun setUp() {
        val context: Context = RuntimeEnvironment.getApplication()
        File(Constants.userDataDir(context), "stablechannels.db").delete()
        db = DatabaseService(context)
        // The accepted trade writes a consistent pair (both fields), as production does.
        db.saveChannel(
            channelId = channelId,
            userChannelId = userChannelId,
            expectedUSD = tradeExpectedUsd,
            backingSats = tradeBackingSats,
            note = null,
            receiverSats = tradeBackingSats,
            latestPrice = priceAtSend1
        )
    }

    /** Mirrors handlePaymentSuccessful(): reconcile the send, persist expected-only. */
    private fun ordinarySendAsProductionDoes(liveReceiverSats: Long, price: Double): StableChannel {
        val record = db.loadChannel(userChannelId)!!
        val sc = StableChannel(
            channelId = record.channelId,
            userChannelId = record.userChannelId,
            expectedUSD = USD(record.expectedUSD),
            backingSats = record.backingSats,
            stableReceiverBTC = Bitcoin(liveReceiverSats)
        )
        val reconciled = StabilityService.reconcileOutgoing(sc, price).first
        // AppState.kt:1969 — saveChannelToDB(preserveBacking = true)
        db.saveChannelPreservingBacking(
            channelId = reconciled.channelId,
            userChannelId = reconciled.userChannelId,
            expectedUSD = reconciled.expectedUSD.amount,
            note = null,
            receiverSats = liveReceiverSats,
            latestPrice = price
        )
        return reconciled
    }

    private fun persistedPairDivergenceUsd(price: Double): Double {
        val r = db.loadChannel(userChannelId)!!
        val backingValueUsd = r.backingSats.toDouble() / Constants.SATS_IN_BTC * price
        return abs(backingValueUsd - r.expectedUSD)
    }

    @Test
    fun ordinarySendKeepsPersistedPairConsistent() {
        ordinarySendAsProductionDoes(liveSatsAfterSend1, priceAtSend1)
        val divergence = persistedPairDivergenceUsd(priceAtSend1)
        assertTrue(
            "persisted expectedUSD/backingSats pair diverged by \$${"%.4f".format(divergence)} " +
                "after an ordinary send (reconcile reduced both, persistence kept only expected) — issue #296",
            divergence <= stabilityDollarFloor
        )
    }

    @Test
    fun restartThenSecondSendDoesNotManufactureAStabilityExcess() {
        // Send #1, then the process "restarts": state is whatever the DB says.
        ordinarySendAsProductionDoes(liveSatsAfterSend1, priceAtSend1)
        // Send #2 reconciles against the reloaded (possibly stale) pair.
        ordinarySendAsProductionDoes(liveSatsAfterSend2, priceAtSend2)

        // Splice-in restores the live balance; the stability check then measures
        // backing value against the persisted target — exactly the field check
        // that produced the $8.53 payment.
        val r = db.loadChannel(userChannelId)!!
        val backingValueUsd = r.backingSats.toDouble() / Constants.SATS_IN_BTC * priceAtCheck
        val phantomExcessUsd = backingValueUsd - r.expectedUSD

        assertTrue(
            "stability check computes a \$${"%.4f".format(phantomExcessUsd)} above-par excess " +
                "from the desynced pair (field incident paid \$8.53) — issue #296",
            phantomExcessUsd <= stabilityDollarFloor
        )
    }
}
