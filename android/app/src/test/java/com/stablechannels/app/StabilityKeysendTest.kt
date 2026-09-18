package com.stablechannels.app

import android.content.Context
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.services.DatabaseService
import com.stablechannels.app.services.StabilityKeysend
import com.stablechannels.app.util.Constants
import java.io.File
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.lightningdevkit.ldknode.NodeException
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class StabilityKeysendTest {
    private lateinit var context: Context
    private lateinit var db: DatabaseService

    private val auditFile
        get() = File(context.cacheDir, "keysend-audit.log")

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
        auditFile.delete()
        AuditService.setLogPath(auditFile.path)
        db = DatabaseService(context)
        db.saveChannel("channel", "7", 10.0, 11_000, null, 20_000, 100_000.0)
        assertTrue(db.claimPendingSend(1_000_000, 100_000.0, "7"))
    }

    @After
    fun tearDown() {
        db.close()
        auditFile.delete()
        context.deleteDatabase(
            File(Constants.userDataDir(context), "stablechannels.db").absolutePath
        )
    }

    @Test
    fun paymentIdIsTheSha256OfThePreimageAsLdkDerivesIt() {
        assertEquals(
            "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925",
            StabilityKeysend.paymentId("00".repeat(32)),
        )
        assertEquals(64, StabilityKeysend.newPreimage().length)
        assertNotEquals(StabilityKeysend.newPreimage(), StabilityKeysend.newPreimage())
    }

    @Test
    fun theClaimCarriesThePaymentIdBeforeLdkIsAskedToSend() {
        var idAtSend: String? = null
        val outcome =
            StabilityKeysend.send(db) { preimage ->
                idAtSend = db.loadPendingSend()!!.paymentId
                StabilityKeysend.paymentId(preimage)
            }
        assertTrue(outcome is StabilityKeysend.Outcome.Sent)
        assertEquals(idAtSend, (outcome as StabilityKeysend.Outcome.Sent).paymentId)
        assertEquals(64, idAtSend!!.length)
        assertEquals(idAtSend, db.loadPendingSend()!!.paymentId)
    }

    @Test
    fun aPersistenceFailureAfterDispatchKeepsTheClaimAndItsId() {
        val outcome = StabilityKeysend.send(db) { throw NodeException.PersistenceFailed("store") }
        assertTrue(outcome is StabilityKeysend.Outcome.OutcomeUnknown)
        assertEquals(64, db.loadPendingSend()!!.paymentId.length)
        assertTrue(auditFile.readText().contains("STABILITY_PAYMENT_OUTCOME_UNKNOWN"))
    }

    @Test
    fun anyOtherSendErrorMeansNothingLeft() {
        val outcome =
            StabilityKeysend.send(db) { throw NodeException.PaymentSendingFailed("route") }
        assertTrue(outcome is StabilityKeysend.Outcome.NotSent)
    }

    @Test
    fun anIdLdkDidNotDeriveOurWayRekeysTheClaim() {
        val outcome = StabilityKeysend.send(db) { "ldk-chose-this" }
        assertEquals("ldk-chose-this", (outcome as StabilityKeysend.Outcome.Sent).paymentId)
        assertEquals("ldk-chose-this", db.loadPendingSend()!!.paymentId)
        assertTrue(auditFile.readText().contains("STABILITY_PAYMENT_ID_MISMATCH"))
    }
}
