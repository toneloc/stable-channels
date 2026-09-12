package com.stablechannels.app

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import com.stablechannels.app.models.Bitcoin
import com.stablechannels.app.models.StableChannel
import com.stablechannels.app.models.USD
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.TradeControlApplyStatus
import com.stablechannels.app.services.TradeControlMessage
import com.stablechannels.app.services.TradeCorrelation
import com.stablechannels.app.services.TradeProtocol
import com.stablechannels.app.services.TradeOutcome
import com.stablechannels.app.util.Constants
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class TradeDatabaseServiceTest {
    private lateinit var context: Context
    private lateinit var dbFile: File

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        dbFile = File(Constants.userDataDir(context), "stablechannels.db")
        deleteDatabaseFiles()
    }

    @After
    fun tearDown() {
        deleteDatabaseFiles()
    }

    @Test
    fun versionTwoSchemaMigratesWithoutLosingRows() {
        val legacy = SQLiteDatabase.openOrCreateDatabase(dbFile, null)
        legacy.execSQL(
            """
            CREATE TABLE channels (
                channel_id TEXT PRIMARY KEY,
                user_channel_id TEXT UNIQUE,
                expected_usd REAL DEFAULT 0,
                stable_sats INTEGER DEFAULT 0,
                note TEXT,
                receiver_sats INTEGER NOT NULL DEFAULT 0,
                latest_price REAL NOT NULL DEFAULT 0.0,
                created_at INTEGER DEFAULT (strftime('%s','now')),
                updated_at INTEGER DEFAULT (strftime('%s','now'))
            )
            """.trimIndent()
        )
        legacy.execSQL(
            """
            CREATE TABLE trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT,
                action TEXT NOT NULL,
                amount_usd REAL NOT NULL,
                amount_btc REAL NOT NULL,
                btc_price REAL NOT NULL,
                fee_usd REAL DEFAULT 0,
                payment_id TEXT,
                status TEXT DEFAULT 'pending',
                created_at INTEGER DEFAULT (strftime('%s','now'))
            )
            """.trimIndent()
        )
        legacy.execSQL(
            "INSERT INTO channels (channel_id, user_channel_id, expected_usd, stable_sats) VALUES (?, ?, ?, ?)",
            arrayOf<Any>("legacy-channel", "legacy-user-channel", 25.0, 25_000)
        )
        legacy.execSQL(
            "INSERT INTO trades (channel_id, action, amount_usd, amount_btc, btc_price) VALUES (?, ?, ?, ?, ?)",
            arrayOf<Any>("legacy-channel", "buy", 5.0, 0.00005, 100_000.0)
        )
        legacy.version = 2
        legacy.close()

        val upgraded = DatabaseService(context)
        val channelColumns = upgraded.readableDatabase.rawQuery(
            "PRAGMA table_info(channels)", null
        ).use { cursor ->
            buildSet { while (cursor.moveToNext()) add(cursor.getString(1)) }
        }
        val tradeColumns = upgraded.readableDatabase.rawQuery(
            "PRAGMA table_info(trades)", null
        ).use { cursor ->
            buildSet { while (cursor.moveToNext()) add(cursor.getString(1)) }
        }
        assertTrue(channelColumns.contains("sync_version"))
        assertTrue(tradeColumns.contains("trade_id"))
        assertTrue(tradeColumns.contains("uncertainty_reason"))
        val channel = upgraded.loadChannel("legacy-user-channel")
        assertNotNull(channel)
        channel!!
        assertEquals(25.0, channel.expectedUSD, 0.0)
        assertEquals(25_000L, channel.backingSats)
        val tradeCount = upgraded.readableDatabase.rawQuery(
            "SELECT COUNT(*) FROM trades", null
        ).use { cursor -> cursor.moveToFirst(); cursor.getLong(0) }
        assertEquals(1L, tradeCount)
        upgraded.close()
    }

    @Test
    fun feePaymentDoesNotApplyAllocationBeforeCorrelatedAcceptance() {
        val identifier = "ab".repeat(32)
        val paymentId = "cd".repeat(32)
        val tradeId = "ef".repeat(32)
        val now = System.currentTimeMillis() / 1000L
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "7",
            expectedUSD = 50.0,
            backingSats = 55_000,
            note = null,
            receiverSats = 100_000,
            latestPrice = 100_000.0
        )
        val prepared = TradeProtocol.prepare(
            spendableSats = 100_000,
            sc = StableChannel(
                channelId = identifier,
                userChannelId = "7",
                expectedUSD = USD(50.0),
                stableReceiverBTC = Bitcoin(100_000),
                backingSats = 55_000
            ),
            action = "sell",
            amountUsd = 10.0,
            amountBtc = 0.000099,
            feeUsd = 0.1,
            newExpectedUsd = 59.9,
            quotePrice = 100_000.0,
            now = now,
            tradeId = tradeId
        )
        assertNotNull(prepared)
        prepared!!
        val tradeDbId = service.recordPreparedTrade(prepared)
        val adopted = service.adoptUnattachedPreparedTrade(paymentId, prepared.feeMsat)
        assertNotNull(adopted)
        assertEquals(tradeDbId, adopted?.tradeDbId)
        assertTrue(service.tradePaymentExists(paymentId))
        assertTrue(service.tradeIsUnresolved(tradeDbId))

        val before = service.loadChannel("7")
        assertNotNull(before)
        before!!
        assertEquals(50.0, before.expectedUSD, 0.0)
        assertEquals(55_000L, before.backingSats)
        assertEquals("fee_paid", service.unresolvedTradePayments()[paymentId]?.status)

        val sync = TradeControlMessage.Sync(
            channelId = identifier,
            userChannelId = "7",
            expectedUsd = prepared.newExpectedUsd,
            backingSats = prepared.newBackingSats + 1,
            syncVersion = 1,
            correlation = TradeCorrelation(tradeId, paymentId, prepared.requestHash)
        )
        assertTrue(service.markTradeResponseNotCommittable(sync))
        assertEquals("uncertain", service.unresolvedTradePayments()[paymentId]?.status)
        val accepted = service.applyCorrelatedTradeAcceptance(sync)
        assertEquals(TradeControlApplyStatus.APPLIED, accepted.status)
        assertEquals(prepared.newBackingSats, accepted.localBackingSats)
        assertEquals(prepared.newBackingSats + 1, accepted.peerBackingSats)

        val after = service.loadChannel("7")
        assertNotNull(after)
        after!!
        assertEquals(prepared.newExpectedUsd, after.expectedUSD, 0.000000001)
        assertEquals(prepared.newBackingSats, after.backingSats)
        assertEquals(1L, after.syncVersion)
        assertNull(service.unresolvedTradePayments()[paymentId])
        assertFalse(service.tradeIsUnresolved(tradeDbId))
        assertEquals(
            TradeControlApplyStatus.DUPLICATE,
            service.applyCorrelatedTradeAcceptance(sync).status
        )

        val superseded = TradeProtocol.prepare(
            spendableSats = 100_000,
            sc = StableChannel(
                channelId = identifier,
                userChannelId = "7",
                expectedUSD = USD(prepared.newExpectedUsd),
                stableReceiverBTC = Bitcoin(100_000),
                backingSats = prepared.newBackingSats
            ),
            action = "buy",
            amountUsd = 1.0,
            amountBtc = 0.0000099,
            feeUsd = 0.01,
            newExpectedUsd = prepared.newExpectedUsd - 1.0,
            quotePrice = 100_000.0,
            now = now + 1,
            tradeId = "aa".repeat(32)
        )!!
        val supersededDbId = service.recordPreparedTrade(superseded)
        val supersededPaymentId = "bb".repeat(32)
        assertTrue(service.attachTradePaymentId(supersededDbId, supersededPaymentId))
        val staleAcceptance = TradeControlMessage.Sync(
            channelId = identifier,
            userChannelId = "7",
            expectedUsd = superseded.newExpectedUsd,
            backingSats = superseded.newBackingSats,
            syncVersion = 1,
            correlation = TradeCorrelation(
                superseded.tradeId, supersededPaymentId, superseded.requestHash
            )
        )
        val staleResult = service.applyCorrelatedTradeAcceptance(staleAcceptance)
        assertEquals(TradeControlApplyStatus.APPLIED, staleResult.status)
        assertFalse(staleResult.allocationApplied!!)
        val afterSupersededAcceptance = service.loadChannel("7")!!
        assertEquals(prepared.newExpectedUsd, afterSupersededAcceptance.expectedUSD, 0.000000001)
        assertEquals(prepared.newBackingSats, afterSupersededAcceptance.backingSats)
        assertEquals(1L, afterSupersededAcceptance.syncVersion)
        assertFalse(service.tradeIsUnresolved(supersededDbId))
        service.close()
    }

    @Test
    fun terminalTradeOutcomeReflectsResultsCommittedOutsideTheHandler() {
        val identifier = "ab".repeat(32)
        val now = System.currentTimeMillis() / 1000L
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "7",
            expectedUSD = 50.0,
            backingSats = 55_000,
            note = null,
            receiverSats = 100_000,
            latestPrice = 100_000.0
        )

        // Rejected trade: outcome must surface with the persisted reason code.
        val rejectedTrade = TradeProtocol.prepare(
            spendableSats = 100_000,
            sc = StableChannel(
                channelId = identifier,
                userChannelId = "7",
                expectedUSD = USD(50.0),
                stableReceiverBTC = Bitcoin(100_000),
                backingSats = 55_000
            ),
            action = "sell",
            amountUsd = 10.0,
            amountBtc = 0.000099,
            feeUsd = 0.1,
            newExpectedUsd = 59.9,
            quotePrice = 100_000.0,
            now = now,
            tradeId = "ef".repeat(32)
        )!!
        val rejectedDbId = service.recordPreparedTrade(rejectedTrade)
        val rejectedPaymentId = "cd".repeat(32)
        assertTrue(service.attachTradePaymentId(rejectedDbId, rejectedPaymentId))
        assertNull(service.terminalTradeOutcome(rejectedPaymentId))

        val rejection = TradeControlMessage.Rejected(
            channelId = identifier,
            correlation = TradeCorrelation(
                rejectedTrade.tradeId, rejectedPaymentId, rejectedTrade.requestHash
            ),
            reasonCode = "quote_deviation",
            decidedAt = now
        )
        assertEquals(TradeControlApplyStatus.APPLIED, service.applyTradeRejection(rejection).status)
        assertEquals(TradeOutcome(false, TradeProtocol.rejectionMessage("quote_deviation")), service.terminalTradeOutcome(rejectedPaymentId))

        // Accepted trade: outcome must flip to accepted with no reason code.
        val acceptedTrade = TradeProtocol.prepare(
            spendableSats = 100_000,
            sc = StableChannel(
                channelId = identifier,
                userChannelId = "7",
                expectedUSD = USD(50.0),
                stableReceiverBTC = Bitcoin(100_000),
                backingSats = 55_000
            ),
            action = "sell",
            amountUsd = 10.0,
            amountBtc = 0.000099,
            feeUsd = 0.1,
            newExpectedUsd = 59.9,
            quotePrice = 100_000.0,
            now = now + 1,
            tradeId = "aa".repeat(32)
        )!!
        val acceptedDbId = service.recordPreparedTrade(acceptedTrade)
        val acceptedPaymentId = "bb".repeat(32)
        assertTrue(service.attachTradePaymentId(acceptedDbId, acceptedPaymentId))
        assertNull(service.terminalTradeOutcome(acceptedPaymentId))

        val sync = TradeControlMessage.Sync(
            channelId = identifier,
            userChannelId = "7",
            expectedUsd = acceptedTrade.newExpectedUsd,
            backingSats = acceptedTrade.newBackingSats,
            syncVersion = 1,
            correlation = TradeCorrelation(
                acceptedTrade.tradeId, acceptedPaymentId, acceptedTrade.requestHash
            )
        )
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            service.applyCorrelatedTradeAcceptance(sync).status
        )
        assertEquals(TradeOutcome(true, ""), service.terminalTradeOutcome(acceptedPaymentId))
        service.close()
    }

    @Test
    fun failedPaymentRecoversPreparedTradeWhenPaymentIdAttachmentWasLost() {
        val channelId = "12".repeat(32)
        val paymentId = "34".repeat(32)
        val service = DatabaseService(context)
        val prepared = TradeProtocol.prepare(
            spendableSats = 100_000,
            sc = StableChannel(
                channelId = channelId,
                userChannelId = "9",
                expectedUSD = USD(25.0),
                stableReceiverBTC = Bitcoin(100_000),
                backingSats = 25_000
            ),
            action = "buy",
            amountUsd = 5.0,
            amountBtc = 0.0000495,
            feeUsd = 0.05,
            newExpectedUsd = 20.0,
            quotePrice = 100_000.0,
            tradeId = "56".repeat(32)
        )
        assertNotNull(prepared)
        val tradeDbId = service.recordPreparedTrade(prepared!!)

        val failed = service.failUnattachedPreparedTrade(paymentId, prepared.feeMsat)

        assertNotNull(failed)
        assertEquals(tradeDbId, failed?.tradeDbId)
        assertEquals("send_failed", failed?.status)
        assertTrue(service.tradePaymentExists(paymentId))
        assertNull(service.unresolvedTradePayments()[paymentId])
        assertFalse(service.tradeIsUnresolved(tradeDbId))
        service.close()
    }

    @Test
    fun correlatedAcceptanceIsInvalidNotRetryWhenChannelHasBeenDeleted() {
        // Simulates a signed trade-sync arriving for a channel that has since closed:
        // deleteChannel() removes the row, and this must resolve permanently (INVALID)
        // rather than RETRY, since a closed channel's row can never reappear and would
        // otherwise retry forever, blocking every later LDK event behind it.
        val identifier = "ab".repeat(32)
        val paymentId = "cd".repeat(32)
        val tradeId = "ef".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "7",
            expectedUSD = 50.0,
            backingSats = 55_000,
            note = null,
            receiverSats = 100_000,
            latestPrice = 100_000.0
        )
        val prepared = TradeProtocol.prepare(
            sc = StableChannel(
                channelId = identifier,
                userChannelId = "7",
                expectedUSD = USD(50.0),
                stableReceiverBTC = Bitcoin(100_000),
                backingSats = 55_000
            ),
            spendableSats = 100_000,
            action = "sell",
            amountUsd = 10.0,
            amountBtc = 0.000099,
            feeUsd = 0.1,
            newExpectedUsd = 59.9,
            quotePrice = 100_000.0,
            tradeId = tradeId
        )!!
        val tradeDbId = service.recordPreparedTrade(prepared)
        service.attachTradePaymentId(tradeDbId, paymentId)
        service.deleteChannel("7")

        val sync = TradeControlMessage.Sync(
            channelId = identifier,
            userChannelId = "7",
            expectedUsd = prepared.newExpectedUsd,
            backingSats = prepared.newBackingSats,
            syncVersion = 1,
            correlation = TradeCorrelation(tradeId, paymentId, prepared.requestHash)
        )
        assertEquals(
            TradeControlApplyStatus.INVALID,
            service.applyCorrelatedTradeAcceptance(sync).status
        )
        service.close()
    }

    @Test
    fun uncorrelatedSyncIsInvalidNotRetryWhenChannelHasBeenDeleted() {
        val identifier = "ab".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "7",
            expectedUSD = 50.0,
            backingSats = 55_000,
            note = null,
            receiverSats = 100_000,
            latestPrice = 100_000.0
        )
        service.deleteChannel("7")

        val sync = TradeControlMessage.Sync(
            channelId = identifier,
            userChannelId = "7",
            expectedUsd = 40.0,
            backingSats = 40_000,
            syncVersion = 1,
            correlation = null
        )
        assertEquals(
            TradeControlApplyStatus.INVALID,
            service.applyUncorrelatedSyncIfNewer(sync, trustedPrice = 100_000.0).status
        )
        service.close()
    }

    @Test
    fun demotingASettlementToLightningMakesTheBackingCreditUnrecoverable() {
        // Regression for the receive-side classification bug: when local channel state was
        // unreadable, receivers recorded the settlement as an ordinary Lightning receipt. This
        // pins WHY that is unsafe — the payment_id is now taken, so the later retry that does
        // have channel state dedups and the backing credit is lost for good. Receivers must
        // leave the event unacked instead of demoting it.
        val identifier = "ab".repeat(32)
        val settlementId = "cd".repeat(32)
        val paymentId = "ef".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "7",
            expectedUSD = 50.0,
            backingSats = 55_000,
            note = null,
            receiverSats = 100_000,
            latestPrice = 100_000.0
        )

        // The demotion: same payment id, recorded as lightning with no backing delta.
        val demoted = service.recordPaymentAndMaybeUpdateBacking(
            paymentId = paymentId,
            paymentType = "lightning",
            direction = "received",
            amountMsat = 25_000
        )
        assertTrue(demoted.isNewPayment)
        assertEquals(55_000L, service.loadChannel("7")?.backingSats)

        // The retry, now with channel state available, cannot repair it: the payment id dedups.
        val retry = service.recordPaymentAndMaybeUpdateBacking(
            paymentId = paymentId,
            paymentType = "stability",
            direction = "received",
            amountMsat = 25_000,
            userChannelId = "7",
            backingDeltaSats = 25,
            settlementId = settlementId
        )
        assertFalse(retry.isNewPayment)
        assertEquals(55_000L, service.loadChannel("7")?.backingSats)
        assertEquals(
            "lightning",
            service.getRecentPayments().first { it.paymentId == paymentId }.paymentType
        )
        service.close()
    }

    @Test
    fun outgoingReconcileComposesCorrectlyWithAConcurrentStabilityDebit() {
        // Regression for a review finding on PR #299 (gpt-6-astra / opus-5): computing the
        // outgoing reconcile against a snapshot of in-memory state and then persisting it as a
        // delta double-counts whatever the stability timer (a separate in-process coroutine)
        // already committed straight to this row via recordPaymentAndMaybeUpdateBacking().
        // reconcileOutgoingBacking() must instead read expected_usd/stable_sats fresh and do the
        // whole computation inside its own transaction, so it composes correctly no matter what
        // ran immediately before it.
        val identifier = "ab".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "7",
            expectedUSD = 90.0,
            backingSats = 100_000,
            note = null,
            receiverSats = 100_000,
            latestPrice = 100_000.0
        )

        // Simulate the stability timer's own concurrent debit landing first: it commits
        // directly to the DB (and, in real code, only updates in-memory state afterward).
        service.recordPaymentAndMaybeUpdateBacking(
            paymentId = "11".repeat(32),
            paymentType = "stability",
            direction = "sent",
            amountMsat = 10_000_000,
            userChannelId = "7",
            backingDeltaSats = -10_000
        )
        assertEquals(90_000L, service.loadChannel("7")?.backingSats)

        // The ordinary send's own overflow, measured against the live (post-send) receiver
        // balance and whatever backing is actually in the DB right now (90,000 sats, $90 —
        // already corrected by the timer above), not a stale pre-timer snapshot (100,000 sats).
        val result = service.reconcileOutgoingBacking(
            channelId = identifier,
            userChannelId = "7",
            note = null,
            receiverSats = 80_000,
            latestPrice = 100_000.0,
            price = 100_000.0
        )

        assertNotNull(result)
        assertEquals(90.0, result!!.oldExpectedUSD, 0.0001)
        assertEquals(10.0, result.usdDeducted, 0.0001)
        assertEquals(80.0, result.newExpectedUSD, 0.0001)
        assertEquals(80_000L, result.newBackingSats)

        val persisted = service.loadChannel("7")
        assertEquals(80.0, persisted?.expectedUSD ?: -1.0, 0.0001)
        assertEquals(80_000L, persisted?.backingSats)
        service.close()
    }

    @Test
    fun clampBackingToLiveReceiverHealsAStrandedWithdrawalExactlyOnce() {
        // Issue #311: a splice-out completed without its stable-books deduction, leaving backing
        // above the balance that actually backs it. The repair deducts the excess once, pins
        // backing to the live balance (the LSP's preserve-sats convention rather than re-pegging
        // sats from the USD target, which is what makes StabilityService.reconcileOutgoing()
        // deduct again on a retry), and is a no-op afterwards.
        val identifier = "cd".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "11",
            expectedUSD = 75.3093,
            backingSats = 75_309,
            note = null,
            receiverSats = 75_309,
            latestPrice = 100_000.0
        )

        // A $15 withdrawal left the channel; the deduction never ran.
        val repaired = service.clampBackingToLiveReceiver("11", receiverSats = 69_056, price = 100_000.0)

        assertNotNull(repaired)
        assertEquals(6_253L, repaired!!.overflowSats)
        assertEquals(6.253, repaired.usdDeducted, 0.0001)
        assertEquals(69.0563, repaired.newExpectedUSD, 0.0001)
        assertEquals(69_056L, repaired.newBackingSats)

        val healed = service.loadChannel("11")
        assertEquals(69.0563, healed?.expectedUSD ?: -1.0, 0.0001)
        assertEquals(69_056L, healed?.backingSats)

        // Idempotent: the books now match the live balance, so nothing more is deducted.
        assertNull(service.clampBackingToLiveReceiver("11", receiverSats = 69_056, price = 100_000.0))
        assertEquals(69.0563, service.loadChannel("11")?.expectedUSD ?: -1.0, 0.0001)

        // A position that is merely below par is NOT an overflow — leave it alone.
        assertNull(service.clampBackingToLiveReceiver("11", receiverSats = 80_000, price = 100_000.0))
        assertEquals(69_056L, service.loadChannel("11")?.backingSats)
        service.close()
    }

    @Test
    fun uncorrelatedSyncAppliesWhenTheLspUsesItsOwnUserChannelId() {
        // Each side assigns its own user_channel_id; for a JIT channel they never match, and the
        // LSP signs the sync with ITS id. Keying the lookup on user_channel_id therefore found
        // no row and every uncorrelated sync was dropped, leaving the LSP unable to correct a
        // diverged wallet (#311). channel_id is what both sides agree on.
        val identifier = "ef".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "316138149017243335882538127405458540875",   // the app's own id
            expectedUSD = 19.7555,
            backingSats = 25_493,
            note = null,
            receiverSats = 25_493,
            latestPrice = 100_000.0
        )

        val sync = TradeControlMessage.Sync(
            channelId = identifier,
            userChannelId = "317806336254983028346801304467074316335",   // the LSP's own id
            expectedUsd = 14.6022,
            backingSats = 18_819,
            syncVersion = 3,
            correlation = null
        )
        assertEquals(
            TradeControlApplyStatus.APPLIED,
            service.applyUncorrelatedSyncIfNewer(sync, trustedPrice = 100_000.0).status
        )
        val healed = service.loadChannel("316138149017243335882538127405458540875")
        assertEquals(14.6022, healed?.expectedUSD ?: -1.0, 0.0001)
        assertEquals(20_340L, healed?.backingSats)   // delta applied against the app's own backing

        // Replaying the same version changes nothing.
        assertEquals(
            TradeControlApplyStatus.DUPLICATE,
            service.applyUncorrelatedSyncIfNewer(sync, trustedPrice = 100_000.0).status
        )
        service.close()
    }

    @Test
    fun outgoingReconcileIsIdempotentBelowPar() {
        // Same preserve-sats rule as StabilityService.reconcileOutgoing: a redelivered
        // PaymentSuccessful (the handler rethrows transient failures, so LDK re-runs it) must not
        // deduct a second time.
        val identifier = "ab".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier,
            userChannelId = "9",
            expectedUSD = 100.0,
            backingSats = 90_000,          // below par
            note = null,
            receiverSats = 90_000,
            latestPrice = 100_000.0
        )

        val first = service.reconcileOutgoingBacking(
            channelId = identifier, userChannelId = "9", note = null,
            receiverSats = 82_000, latestPrice = 100_000.0, price = 100_000.0
        )
        assertNotNull(first)
        assertEquals(8.0, first!!.usdDeducted, 0.0001)
        assertEquals(92.0, first.newExpectedUSD, 0.0001)
        assertEquals(82_000L, first.newBackingSats)

        val replay = service.reconcileOutgoingBacking(
            channelId = identifier, userChannelId = "9", note = null,
            receiverSats = 82_000, latestPrice = 100_000.0, price = 100_000.0
        )
        assertNull(replay)
        assertEquals(92.0, service.loadChannel("9")?.expectedUSD ?: -1.0, 0.0001)
        assertEquals(82_000L, service.loadChannel("9")?.backingSats)
        service.close()
    }

    @Test
    fun mostRecentTradeFailureFindsARejectionResolvedWhileTheAppWasAway() {
        // A rejection delivered while backgrounded is committed here, but the sheet that would
        // have shown it dies with the process. Startup resurfaces the newest failure so the
        // refusal is not silent (flow 15).
        val identifier = "ba".repeat(32)
        val service = DatabaseService(context)
        val now = System.currentTimeMillis() / 1000L
        val trade = TradeProtocol.prepare(
            spendableSats = 100_000,
            sc = StableChannel(
                channelId = identifier,
                userChannelId = "7",
                expectedUSD = USD(50.0),
                stableReceiverBTC = Bitcoin(100_000),
                backingSats = 55_000
            ),
            action = "sell",
            amountUsd = 10.0,
            amountBtc = 0.000099,
            feeUsd = 0.1,
            newExpectedUsd = 59.9,
            quotePrice = 100_000.0,
            now = now,
            tradeId = "cc".repeat(32)
        )!!
        val dbId = service.recordPreparedTrade(trade)
        val paymentId = "dd".repeat(32)
        assertTrue(service.attachTradePaymentId(dbId, paymentId))
        assertNull(service.mostRecentTradeFailure(3600))   // nothing terminal yet

        val rejection = TradeControlMessage.Rejected(
            channelId = identifier,
            correlation = TradeCorrelation(trade.tradeId, paymentId, trade.requestHash),
            reasonCode = "quote_deviation",
            decidedAt = now
        )
        assertEquals(TradeControlApplyStatus.APPLIED, service.applyTradeRejection(rejection).status)

        val failure = service.mostRecentTradeFailure(3600)
        assertNotNull(failure)
        assertEquals(paymentId, failure!!.paymentId)
        assertEquals(TradeProtocol.rejectionMessage("quote_deviation"), failure.outcome.message)

        // Outside the window it is stale news — History carries it instead.
        service.writableDatabase.execSQL("UPDATE trades SET resolved_at = resolved_at - 7200")
        assertNull(service.mostRecentTradeFailure(3600))
        service.close()
    }

    @Test
    fun outgoingReconcileClosesThePositionWhenTheSpendExhaustsTheTarget() {
        // Zero boundary, persisted side: nothing backs an exhausted target.
        val identifier = "1a".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier, userChannelId = "21", expectedUSD = 10.0,
            backingSats = 20_000, note = null, receiverSats = 20_000, latestPrice = 100_000.0
        )

        val result = service.reconcileOutgoingBacking(
            channelId = identifier, userChannelId = "21", note = null,
            receiverSats = 5_000, latestPrice = 100_000.0, price = 100_000.0
        )

        assertNotNull(result)
        assertEquals(0.0, result!!.newExpectedUSD, 0.0001)
        assertEquals(0L, result.newBackingSats)
        val row = service.loadChannel("21")
        assertEquals(0.0, row?.expectedUSD ?: -1.0, 0.0001)
        assertEquals(0L, row?.backingSats)          // the 5,000 sats are native, not stranded
        service.close()
    }

    @Test
    fun backingClampClosesThePositionWhenTheRepairExhaustsTheTarget() {
        // Same boundary on the startup repair path.
        val identifier = "2b".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier, userChannelId = "22", expectedUSD = 10.0,
            backingSats = 20_000, note = null, receiverSats = 20_000, latestPrice = 100_000.0
        )

        val repaired = service.clampBackingToLiveReceiver("22", receiverSats = 5_000, price = 100_000.0)

        assertNotNull(repaired)
        assertEquals(0.0, repaired!!.newExpectedUSD, 0.0001)
        assertEquals(0L, repaired.newBackingSats)
        assertEquals(0L, service.loadChannel("22")?.backingSats)
        service.close()
    }

    @Test
    fun confirmationPollerLeavesSpliceRowsToTheSpliceMonitor() {
        // The #311 race: the poller used to complete splice rows at 1 conf, which skipped the
        // stable-books deduction and left nothing for recovery to find. Splice rows must not be
        // offered to the poller at all — the monitor owns them.
        val service = DatabaseService(context)
        val txid = "3c".repeat(32)
        for (type in listOf("splice_out", "splice_in", "onchain")) {
            service.recordPayment(
                paymentId = "$type-pending",
                paymentType = type,
                direction = if (type == "onchain") "received" else "sent",
                amountMsat = 15_000_000,
                amountUSD = 15.0,
                btcPrice = 100_000.0,
                txid = txid,
                status = "pending"
            )
        }

        val offered = service.getPaymentsNeedingConfirmation().map { it.paymentType }.toSet()
        assertTrue(offered.contains("onchain"))
        assertFalse(offered.contains("splice_out"))
        assertFalse(offered.contains("splice_in"))
        service.close()
    }

    @Test
    fun repairDefersWhileAStabilitySendIsStillUnreconciled() {
        // (1) A stability keysend that SUCCEEDED but whose backing debit is not recorded yet
        // leaves the channel looking like an overflow. Repairing there would take the same sats
        // twice — once as a USD clamp, once as the debit recovery is about to apply.
        val identifier = "4d".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier, userChannelId = "31", expectedUSD = 20.0,
            backingSats = 20_000, note = null, receiverSats = 20_000, latestPrice = 100_000.0
        )
        assertTrue(service.claimPendingSend(amountMsat = 5_000_000, price = 100_000.0))
        service.setPendingSendPaymentId("55".repeat(32))

        // The sats have left the channel: backing 20,000 vs a live balance of 15,000.
        assertNull(service.clampBackingToLiveReceiver("31", receiverSats = 15_000, price = 100_000.0))
        assertEquals(20_000L, service.loadChannel("31")?.backingSats)

        // Recovery records the payment and debits backing once, then clears the marker.
        val persisted = service.recordPaymentAndMaybeUpdateBacking(
            paymentId = "55".repeat(32), paymentType = "stability", direction = "sent",
            amountMsat = 5_000_000, userChannelId = "31", backingDeltaSats = -5_000
        )
        assertTrue(persisted.isNewPayment)
        assertEquals(15_000L, persisted.backingSats)
        service.clearPendingSend()

        // Now the books match the live balance, so the repair has nothing left to take.
        assertNull(service.clampBackingToLiveReceiver("31", receiverSats = 15_000, price = 100_000.0))
        assertEquals(15_000L, service.loadChannel("31")?.backingSats)
        service.close()
    }

    @Test
    fun spliceAccountingResumesExactlyOnceAfterATerminationBeforeCompletion() {
        // (2) Books are written before the row is marked complete, so a crash in between leaves
        // the row pending and the resume path runs the deduction again — which must be a no-op
        // the second time.
        val identifier = "5e".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier, userChannelId = "32", expectedUSD = 20.0,
            backingSats = 20_000, note = null, receiverSats = 20_000, latestPrice = 100_000.0
        )
        val txid = "6f".repeat(32)
        service.recordPayment(
            paymentId = "splice-resume", paymentType = "splice_out", direction = "sent",
            amountMsat = 5_000_000, amountUSD = 5.0, btcPrice = 100_000.0,
            txid = txid, status = "pending"
        )

        // First attempt: deduction lands, then the process dies before completeSplice().
        val first = service.reconcileOutgoingBacking(
            channelId = identifier, userChannelId = "32", note = null,
            receiverSats = 15_000, latestPrice = 100_000.0, price = 100_000.0
        )
        assertNotNull(first)
        assertEquals(15.0, first!!.newExpectedUSD, 0.0001)
        assertTrue(service.hasPendingSpliceFor(txid))   // still resumable

        // Resume: the reconcile is idempotent, and only now is the row completed.
        assertNull(
            service.reconcileOutgoingBacking(
                channelId = identifier, userChannelId = "32", note = null,
                receiverSats = 15_000, latestPrice = 100_000.0, price = 100_000.0
            )
        )
        assertEquals(15.0, service.loadChannel("32")?.expectedUSD ?: -1.0, 0.0001)
        assertTrue(service.completeSplice(txid))
        assertFalse(service.hasPendingSpliceFor(txid))
        service.close()
    }

    @Test
    fun repairWorksWhenTheChannelBalanceIsZero() {
        // (3) A full splice-out empties the channel; the position must be allowed to close.
        val identifier = "7a".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier, userChannelId = "33", expectedUSD = 5.0,
            backingSats = 5_000, note = null, receiverSats = 5_000, latestPrice = 100_000.0
        )

        val repaired = service.clampBackingToLiveReceiver("33", receiverSats = 0, price = 100_000.0)

        assertNotNull(repaired)
        assertEquals(0L, repaired!!.newBackingSats)
        assertEquals(0.0, repaired.newExpectedUSD, 0.0001)
        assertEquals(0L, service.loadChannel("33")?.backingSats)
        service.close()
    }

    @Test
    fun repairRetriesOnceATrustedPriceIsAvailable() {
        // (4) Without a trusted price the spend cannot be valued, so the repair defers — and the
        // deferral must leave the books untouched so a later attempt still finds the work.
        val identifier = "8b".repeat(32)
        val service = DatabaseService(context)
        service.saveChannel(
            channelId = identifier, userChannelId = "34", expectedUSD = 20.0,
            backingSats = 20_000, note = null, receiverSats = 20_000, latestPrice = 100_000.0
        )

        assertNull(service.clampBackingToLiveReceiver("34", receiverSats = 15_000, price = 0.0))
        assertEquals(20_000L, service.loadChannel("34")?.backingSats)   // nothing lost

        val repaired = service.clampBackingToLiveReceiver("34", receiverSats = 15_000, price = 100_000.0)
        assertNotNull(repaired)
        assertEquals(15_000L, repaired!!.newBackingSats)
        assertEquals(15.0, repaired.newExpectedUSD, 0.0001)
        service.close()
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm"))
            .forEach { file -> if (file.exists()) assertTrue(file.delete()) }
        assertFalse(dbFile.exists())
    }
}
