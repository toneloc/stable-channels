package com.stablechannels.app.ui.history

import android.content.Context
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class HistoryLoadTest {
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
    fun `missing database is reported instead of an empty history`() {
        assertEquals(HistoryLoad.Failed(HISTORY_UNAVAILABLE_MESSAGE), readHistory(null))
    }

    @Test
    fun `rows are loaded from the database`() {
        val db = DatabaseService(context)
        db.recordPayment(
            paymentId = "p1",
            paymentType = "onchain",
            direction = "sent",
            amountMsat = 100_000,
        )

        val result = readHistory(db)

        assertTrue(result is HistoryLoad.Loaded)
        assertEquals(listOf("p1"), (result as HistoryLoad.Loaded).payments.map { it.paymentId })
        db.close()
    }

    @Test
    fun `read failure is reported instead of crashing`() {
        val db = DatabaseService(context)
        db.writableDatabase.execSQL("DROP TABLE payments")

        assertEquals(HistoryLoad.Failed(HISTORY_LOAD_FAILED_MESSAGE), readHistory(db))
        db.close()
    }

    private fun deleteDatabaseFiles() {
        listOf(dbFile, File("${dbFile.path}-wal"), File("${dbFile.path}-shm")).forEach { file ->
            if (file.exists()) assertTrue(file.delete())
        }
        assertFalse(dbFile.exists())
    }
}
