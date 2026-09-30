package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class DatabaseServiceSweepTest {
    private lateinit var context: Context
    private lateinit var dbFile: File
    private lateinit var dbService: DatabaseService

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        dbFile = File(Constants.userDataDir(context), "stablechannels.db")
        deleteDatabaseFiles()
        dbService = DatabaseService(context)
    }

    @After
    fun tearDown() {
        deleteDatabaseFiles()
    }

    private fun deleteDatabaseFiles() {
        dbFile.delete()
        File(dbFile.absolutePath + "-journal").delete()
        File(dbFile.absolutePath + "-wal").delete()
        File(dbFile.absolutePath + "-shm").delete()
    }

    @Test
    fun reconcileChannels_survivesOnMatchedChannelId() {
        dbService.saveChannel(
            channelId = "live_chan_1",
            userChannelId = "stale_user_id_1",
            expectedUSD = 100.0,
            backingSats = 100000L,
            nativeSats = 0L,
            note = "",
            receiverSats = 100000L,
            latestPrice = 60000.0,
        )
        // Simulate sweep with live list that has a DIFFERENT userChannelId for the same channelId
        dbService.reconcileChannels(listOf("new_user_id_1"), listOf("live_chan_1"))

        // Should survive because channelId matched
        val row = dbService.loadChannel("stale_user_id_1")
        assertNotNull(row)
    }

    @Test
    fun reconcileChannels_deletesOnNeitherMatch() {
        dbService.saveChannel(
            channelId = "dead_chan_1",
            userChannelId = "dead_user_id_1",
            expectedUSD = 100.0,
            backingSats = 100000L,
            nativeSats = 0L,
            note = "",
            receiverSats = 100000L,
            latestPrice = 60000.0,
        )
        dbService.reconcileChannels(listOf("live_user_id"), listOf("live_chan_id"))

        // Should be deleted
        val row = dbService.loadChannel("dead_user_id_1")
        assertNull(row)
    }

    @Test
    fun reconcileChannels_truncatesOnZeroChannelBranch() {
        dbService.saveChannel(
            channelId = "dead_chan_1",
            userChannelId = "dead_user_id_1",
            expectedUSD = 100.0,
            backingSats = 100000L,
            nativeSats = 0L,
            note = "",
            receiverSats = 100000L,
            latestPrice = 60000.0,
        )
        dbService.reconcileChannels(emptyList(), emptyList())

        // Should be truncated
        val row = dbService.loadChannel("dead_user_id_1")
        assertNull(row)
        assertFalse(dbService.hasAnyChannel())
    }
}
