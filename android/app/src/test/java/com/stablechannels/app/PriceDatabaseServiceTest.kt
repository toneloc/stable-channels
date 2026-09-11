package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.models.DailyPriceRecord
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class PriceDatabaseServiceTest {
    private lateinit var context: Context
    private lateinit var dbFile: File
    private lateinit var service: DatabaseService

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        dbFile = File(Constants.userDataDir(context), "stablechannels.db")
        deleteDatabaseFiles()
        service = DatabaseService(context)
    }

    @After
    fun tearDown() {
        if (::service.isInitialized) service.close()
        deleteDatabaseFiles()
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm"))
            .forEach { file -> if (file.exists()) file.delete() }
    }

    @Test
    fun testGetLatestPriceHistoryTimestamp() {
        assertNull(service.getLatestPriceHistoryTimestamp())

        val now = System.currentTimeMillis() / 1000
        val baseTs = now - 48 * 3600
        service.backfillHourlyPrices(listOf(baseTs to 50000.0, baseTs + 3600 to 51000.0))

        val latest = service.getLatestPriceHistoryTimestamp()
        assertNotNull(latest)
        assertEquals(baseTs + 3600, latest)
    }

    @Test
    fun testBackfillHourlyPricesDeduplicationAndRowCount() {
        val now = System.currentTimeMillis() / 1000
        val baseTs = now - 24 * 3600
        val candles = listOf(
            baseTs to 50000.0,
            baseTs + 3600 to 51000.0,
            baseTs + 7200 to 52000.0
        )

        val inserted = service.backfillHourlyPrices(candles)
        assertEquals(3, inserted)

        // Re-inserting identical candles should insert 0 rows
        val reinserted = service.backfillHourlyPrices(candles)
        assertEquals(0, reinserted)

        // Inserting a candle within ±30 minutes (+10 minutes) should be ignored
        val nearDuplicate = listOf((baseTs + 600) to 50500.0)
        val nearCount = service.backfillHourlyPrices(nearDuplicate)
        assertEquals(0, nearCount)

        // Inserting a new candle 3 hours later should be inserted
        val newCandle = listOf((baseTs + 10800) to 53000.0)
        val newCount = service.backfillHourlyPrices(newCandle)
        assertEquals(1, newCount)

        val totalRecords = service.getPriceHistory(24 * 30)
        assertEquals(4, totalRecords.size)
    }

    @Test
    fun testBackfillDailyPricesNewDateAccounting() {
        val candles = listOf(
            DailyPriceRecord("2026-09-08", 55000.0, 56000.0, 54000.0, 55500.0, 100.0),
            DailyPriceRecord("2026-09-09", 55500.0, 57000.0, 55000.0, 56500.0, 150.0),
            DailyPriceRecord("2026-09-10", 56500.0, 58000.0, 56000.0, 57500.0, 200.0)
        )

        val inserted = service.backfillDailyPrices(candles)
        assertEquals(3, inserted)

        // Re-inserting existing dates with updated close price returns 0 newly inserted dates
        val updatedCandles = listOf(
            DailyPriceRecord("2026-09-10", 56500.0, 58500.0, 56000.0, 58200.0, 250.0)
        )
        val updatedCount = service.backfillDailyPrices(updatedCandles)
        assertEquals(0, updatedCount)

        // Verify the existing row's close price was refreshed
        val daily = service.getDailyPrices(10)
        val sep10 = daily.find { it.date == "2026-09-10" }
        assertNotNull(sep10)
        assertEquals(58200.0, sep10!!.close, 0.001)

        // Inserting a new date returns 1
        val newDay = listOf(
            DailyPriceRecord("2026-09-11", 58200.0, 59000.0, 58000.0, 58800.0, 180.0)
        )
        val newDayCount = service.backfillDailyPrices(newDay)
        assertEquals(1, newDayCount)
    }
}
