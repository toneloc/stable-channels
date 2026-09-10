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

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm"))
            .forEach { file -> if (file.exists()) assertTrue(file.delete()) }
        assertFalse(dbFile.exists())
    }
}
