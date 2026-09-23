package com.stablechannels.app

import android.database.sqlite.SQLiteDatabase
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.push.StabilityProcessingService
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.MissingChannelRowException
import com.stablechannels.app.services.StabilityService
import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class IncomingStabilityPaymentTest {
    private lateinit var db: DatabaseService
    private lateinit var dbFile: File
    private lateinit var background: StabilityProcessingService

    @Before
    fun setUp() {
        val context = RuntimeEnvironment.getApplication()
        dbFile = File(Constants.userDataDir(context), "stablechannels.db")
        SQLiteDatabase.deleteDatabase(dbFile)
        db = DatabaseService(context)
        background = Robolectric.buildService(StabilityProcessingService::class.java).get()
    }

    @After
    fun tearDown() {
        db.close()
        SQLiteDatabase.deleteDatabase(dbFile)
    }

    private fun seed(backing: Long = 9_000, target: Double = 10.0) {
        db.saveChannel(
            "ab".repeat(32),
            "7",
            target,
            backing,
            null,
            receiverSats = backing,
            latestPrice = 200_000.0,
        )
    }

    private fun receive(inBackground: Boolean, amount: Long = 5_000): Boolean {
        val result = persistReceipt(inBackground, amount = amount)
        assertTrue(
            "Unexpected persistence result: $result",
            result in listOf("INSERTED", "DUPLICATE"),
        )
        return result == "INSERTED"
    }

    private fun persistReceipt(
        inBackground: Boolean,
        amount: Long = 5_000,
        price: Double = 100_000.0,
        paymentId: String = "payment",
        settlementId: String = "settlement",
        userChannelId: String = "7",
    ): String {
        if (!inBackground) {
            return try {
                val result =
                    db.recordPaymentAndMaybeUpdateBacking(
                        paymentId = paymentId,
                        paymentType = "stability",
                        direction = "received",
                        amountMsat = amount * 1000,
                        btcPrice = price,
                        userChannelId = userChannelId,
                        backingDeltaSats = amount,
                        settlementId = settlementId,
                    )
                if (result.isNewPayment) "INSERTED" else "DUPLICATE"
            } catch (_: MissingChannelRowException) {
                "MISSING_CHANNEL"
            } catch (_: Exception) {
                "FAILED"
            }
        }
        val method =
            StabilityProcessingService::class
                .java
                .declaredMethods
                .single {
                    it.name == "recordPaymentAtomicInDB"
                }
                .apply { isAccessible = true }
        return method
            .invoke(
                background,
                dbFile.absolutePath,
                paymentId,
                "stability",
                "received",
                amount * 1000,
                price,
                amount,
                userChannelId,
                settlementId,
            )!!
            .toString()
    }

    private fun assertExcessStaysNative(inBackground: Boolean) {
        seed()
        assertTrue(receive(inBackground))
        val saved = db.loadChannel("7")!!
        assertEquals(10_000L, saved.backingSats)
        assertEquals(10.0, saved.expectedUSD, 0.0)
        val channel =
            StableChannel(
                expectedUSD = USD(saved.expectedUSD),
                backingSats = saved.backingSats,
                stableReceiverBTC = Bitcoin(14_000),
                isStableReceiver = true,
            )
        StabilityService.recomputeNative(channel)
        assertEquals(4_000L, channel.nativeChannelBTC.sats)
        assertEquals(
            StabilityService.StabilityAction.STABLE,
            StabilityService.checkStabilityAction(channel, 100_000.0).action,
        )
        assertFalse(receive(inBackground))
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(1, db.getRecentPayments().size)
        assertEquals(5_000_000L, db.getRecentPayments().single().amountMsat)
    }

    @Test fun foregroundExcessStaysNative() = assertExcessStaysNative(false)

    @Test fun backgroundExcessStaysNative() = assertExcessStaysNative(true)

    private fun assertExistingSurplusIsPreserved(inBackground: Boolean) {
        seed(backing = 11_000)
        assertTrue(receive(inBackground))
        assertEquals(11_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test fun foregroundPreservesExistingSurplus() = assertExistingSurplusIsPreserved(false)

    @Test fun backgroundPreservesExistingSurplus() = assertExistingSurplusIsPreserved(true)

    private fun bothPaths(test: (Boolean) -> Unit) {
        for (inBackground in listOf(false, true)) {
            db.writableDatabase.execSQL("DELETE FROM payments")
            db.writableDatabase.execSQL("DELETE FROM stability_settlements")
            db.writableDatabase.execSQL("DELETE FROM channels")
            test(inBackground)
        }
    }

    private fun count(table: String): Long =
        db.readableDatabase
            .rawQuery(
                "SELECT COUNT(*) FROM $table",
                null,
            )
            .use {
                it.moveToFirst()
                it.getLong(0)
            }

    @Test
    fun partialAndExactReceiptsOnlyFillTheShortfall() = bothPaths { background ->
        seed()
        assertTrue(receive(background, 500))
        assertEquals(9_500L, db.loadChannel("7")!!.backingSats)
        assertEquals(
            "INSERTED",
            persistReceipt(
                background,
                amount = 500,
                paymentId = "second",
                settlementId = "second",
            ),
        )
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun latestSavedTargetAndWalletPriceControlTheCredit() = bothPaths { background ->
        seed()
        // A correction landed after the caller's old snapshot. The saved price is also stale.
        db.writableDatabase.execSQL(
            "UPDATE channels SET expected_usd = 8, stable_sats = 7500 WHERE user_channel_id = '7'"
        )
        assertTrue(receive(background))
        assertEquals(8.0, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(8_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun receiptFloorsTheTarget() = bothPaths { background ->
        seed(target = 10.0009)
        assertEquals("INSERTED", persistReceipt(background))
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun zeroTargetsDoNotCreditOrEraseBacking() = bothPaths { background ->
        seed(backing = 0, target = 0.0)
        assertTrue(receive(background))
        assertEquals(0L, db.loadChannel("7")!!.backingSats)
        seed(backing = 2_000, target = 0.0)
        assertEquals(
            "INSERTED",
            persistReceipt(
                background,
                paymentId = "retained",
                settlementId = "retained",
            ),
        )
        assertEquals(2_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun duplicatePaymentAndSettlementIdsStayIdempotentAcrossRestartAndPaths() =
        bothPaths { background ->
            seed()
            assertEquals("INSERTED", persistReceipt(background, amount = 200))
            db.close()
            db = DatabaseService(RuntimeEnvironment.getApplication())
            // Still below target: repeating the credit would incorrectly add another 200 sats.
            // Dedup succeeds even if the price is temporarily unavailable on replay.
            assertEquals(
                "DUPLICATE",
                persistReceipt(
                    !background,
                    amount = 200,
                    price = 0.0,
                    settlementId = "different",
                ),
            )
            assertEquals(
                "DUPLICATE",
                persistReceipt(
                    !background,
                    amount = 200,
                    price = 0.0,
                    paymentId = "different",
                ),
            )
            assertEquals(9_200L, db.loadChannel("7")!!.backingSats)
            assertEquals(1L, count("payments"))
            assertEquals(1L, count("stability_settlements"))
        }

    @Test
    fun unavailablePriceLeavesReceiptRetryable() = bothPaths { background ->
        seed()
        for (price in listOf(0.0, -1.0, Double.NaN, Double.POSITIVE_INFINITY, Double.MIN_VALUE)) {
            assertEquals("FAILED", persistReceipt(background, price = price))
            assertEquals(0L, count("payments"))
            assertEquals(0L, count("stability_settlements"))
            assertEquals(9_000L, db.loadChannel("7")!!.backingSats)
        }
        assertTrue(receive(background))
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun failedBackingWriteRollsBackHistoryAndReplayGuard() = bothPaths { background ->
        seed()
        db.writableDatabase.execSQL(
            """
            CREATE TRIGGER reject_backing BEFORE UPDATE OF stable_sats ON channels
            BEGIN SELECT RAISE(ABORT, 'temporary write failure'); END
            """
                .trimIndent()
        )
        try {
            assertEquals("FAILED", persistReceipt(background))
            assertEquals(9_000L, db.loadChannel("7")!!.backingSats)
            assertEquals(0L, count("payments"))
            assertEquals(0L, count("stability_settlements"))
        } finally {
            db.writableDatabase.execSQL("DROP TRIGGER reject_backing")
        }
        assertTrue(receive(!background))
        assertFalse(receive(background))
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
    }

    @Test
    fun missingChannelCanBeRecreatedWithoutLosingCredit() = bothPaths { background ->
        assertEquals("MISSING_CHANNEL", persistReceipt(background))
        assertEquals(0L, count("payments"))
        assertEquals(0L, count("stability_settlements"))
        // A newer unrelated row must never receive this payment's backing credit.
        seed()
        db.saveChannel(
            "cd".repeat(32),
            "8",
            20.0,
            19_000,
            null,
            receiverSats = 50_000,
            latestPrice = 100_000.0,
        )
        assertTrue(receive(background))
        assertEquals(10_000L, db.loadChannel("7")!!.backingSats)
        assertEquals(19_000L, db.loadChannel("8")!!.backingSats)
    }

    private fun assertReceiptBeforeOutgoingAccounting(
        background: Boolean,
        belowSavedBacking: Boolean,
    ) {
        val price = if (belowSavedBacking) 100_000.0 else 50_000.0
        val amount = if (belowSavedBacking) 1_000L else 10_000L
        val liveReceiver = if (belowSavedBacking) 5_000L else 15_000L
        val expectedAfterSend = if (belowSavedBacking) 5.0 else 7.5
        seed(backing = if (belowSavedBacking) 9_000 else 10_000)
        // Both transfers settled in LDK, but the receipt event is processed first. In the
        // second case BTC fell after the send was admitted, so #322's pre-spend guard
        // cannot prevent the incoming top-up from crossing that already-in-flight send.
        assertEquals(
            "FAILED",
            persistReceipt(
                background,
                amount = amount,
                price = 0.0,
            ),
        )
        assertEquals(0L, count("stability_settlements"))
        assertEquals(
            "INSERTED",
            persistReceipt(
                background,
                amount = amount,
                price = price,
            ),
        )
        db.reconcileOutgoingBacking(
            "ab".repeat(32),
            "7",
            null,
            receiverSats = liveReceiver,
            latestPrice = price,
            price = price,
        )
        assertEquals(expectedAfterSend, db.loadChannel("7")!!.expectedUSD, 0.0)
        assertEquals(liveReceiver, db.loadChannel("7")!!.backingSats)
        // A retry or foreground/background handoff cannot restore the spent claim.
        assertEquals(
            "DUPLICATE",
            persistReceipt(
                !background,
                amount = amount,
                price = price,
            ),
        )
        db.reconcileOutgoingBacking(
            "ab".repeat(32),
            "7",
            null,
            receiverSats = liveReceiver,
            latestPrice = price,
            price = price,
        )
        assertEquals(expectedAfterSend, db.loadChannel("7")!!.expectedUSD, 0.0)
    }

    @Test
    fun foregroundReceiptDoesNotHideLaterSpend() =
        assertReceiptBeforeOutgoingAccounting(false, false)

    @Test
    fun backgroundReceiptDoesNotHideLaterSpend() =
        assertReceiptBeforeOutgoingAccounting(true, false)

    @Test
    fun foregroundReceiptCanPrecedeBackingDeduction() =
        assertReceiptBeforeOutgoingAccounting(false, true)

    @Test
    fun backgroundReceiptCanPrecedeBackingDeduction() =
        assertReceiptBeforeOutgoingAccounting(true, true)
}
