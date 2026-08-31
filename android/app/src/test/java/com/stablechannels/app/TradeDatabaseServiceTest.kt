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
    fun failedPaymentRecoversPreparedTradeWhenPaymentIdAttachmentWasLost() {
        val channelId = "12".repeat(32)
        val paymentId = "34".repeat(32)
        val service = DatabaseService(context)
        val prepared = TradeProtocol.prepare(
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

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm"))
            .forEach { file -> if (file.exists()) assertTrue(file.delete()) }
        assertFalse(dbFile.exists())
    }
}
