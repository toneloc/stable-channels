package com.stablechannels.app

import android.content.Context
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
}
