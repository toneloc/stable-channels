package com.stablechannels.app.domain

import android.content.Context
import com.stablechannels.app.services.EmailPromptStore
import com.stablechannels.app.services.EmailRegistration
import com.stablechannels.app.services.EmailSubmissionService
import com.stablechannels.app.services.EmailSubmitter
import com.stablechannels.app.ui.email.EmailSettingsModel
import kotlinx.coroutines.test.runTest
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

private const val NODE_ID = "02eec7245d6b7d2ccb30380bfbe2a3648cd7a942653f5aa340edcea1f283686619"
private const val TS = 1_762_790_000L

private fun email(raw: String) = EmailAddress.parse(raw)!!

private class FakeSigner(
    override val readyNodeId: String? = NODE_ID,
    private val error: Exception? = null,
) : NodeMessageSigner {
    val signed = mutableListOf<String>()

    override fun sign(message: ByteArray): String {
        error?.let { throw it }
        signed += String(message, Charsets.UTF_8)
        return "sig"
    }
}

private class InMemorySavedEmail(override var savedEmail: EmailAddress? = null) : SavedEmailStoring

private suspend fun failureOf(block: suspend () -> Unit): EmailSaveFailure? =
    try {
        block()
        null
    } catch (e: EmailSaveException) {
        e.failure
    }

class EmailAddressTest {
    @Test
    fun validAddress_isTrimmedAndLowercased() {
        assertEquals("satoshi@gmx.com", EmailAddress.parse("  Satoshi@GMX.com \n")?.value)
    }

    @Test
    fun invalidAddresses_areRejected() {
        val invalid =
            listOf(
                "",
                "plain",
                "@example.com",
                "a@",
                "a@b",
                "a@b.c",
                "a@@b.com",
                "a@b@c.com",
                "a b@c.com",
                "a@.com",
                "a@b..com",
                "a@b.com.",
                "a".repeat(65) + "@b.com",
                "a@" + "b".repeat(250) + ".com",
            )
        for (raw in invalid) assertNull(raw, EmailAddress.parse(raw))
    }

    // Same rule as the server: plain ASCII address characters only, so nothing the server refuses
    // gets sent.
    @Test
    fun addressesTheServerRefuses_areRejected() {
        val refused =
            listOf(
                "a<b@example.com",
                "a>b@example.com",
                "\"a\"@example.com",
                "a@exa<mple.com",
                "a@example.com/x",
                "a@exa_mple.com",
                "a@-example.com",
                "a@example-.com",
                "a\\b@example.com",
                "ä@example.com",
                "a@exämple.com",
                ".a@example.com",
                "a.@example.com",
                "a..b@example.com",
            )
        for (raw in refused) assertNull(raw, EmailAddress.parse(raw))
        for (raw in
            listOf(
                "o'brien@example.com",
                "a_b-c+d@sub-domain.example.co",
                "x@xn--bcher-kva.example",
            )) {
            assertNotNull(raw, EmailAddress.parse(raw))
        }
    }

    @Test
    fun longestAllowedAddress_isAccepted() {
        val raw = "a@" + "b".repeat(EmailAddress.MAX_LENGTH - 6) + ".com"
        assertEquals(EmailAddress.MAX_LENGTH, raw.length)
        assertNotNull(EmailAddress.parse(raw))
    }
}

class EmailRegistrationTest {
    @Test
    fun payload_hasSortedKeysAndNoWhitespace_matchingIos() {
        assertEquals(
            """{"email":"you@example.com","node_id":"$NODE_ID","ts":1762790000,"type":"REGISTER_EMAIL_V1"}""",
            EmailRegistration.payload(email("You@Example.com"), NODE_ID, TS),
        )
    }

    @Test
    fun payload_forRemoval_sendsEmptyEmail() {
        assertEquals(
            """{"email":"","node_id":"$NODE_ID","ts":1762790000,"type":"REGISTER_EMAIL_V1"}""",
            EmailRegistration.payload(null, NODE_ID, TS),
        )
    }

    @Test
    fun payload_doesNotEscapeSlashes() {
        val payload = EmailRegistration.payload(email("a/b@example.com"), NODE_ID, TS)
        assertTrue(payload, payload.startsWith("""{"email":"a/b@example.com","""))
    }
}

class EmailSubmissionServiceTest {
    private fun service(server: MockWebServer, signer: NodeMessageSigner) =
        EmailSubmissionService(
            signer = signer,
            endpoint = server.url("/api/register-email").toString(),
            nowSecs = { TS },
        )

    @Test
    fun submit_postsSignedEnvelopeOverExactPayload() = runTest {
        MockWebServer().use { server ->
            server.enqueue(MockResponse().setResponseCode(200).setBody("""{"ok":true}"""))
            val signer = FakeSigner()

            service(server, signer).submit(email("user@example.com"))

            val payload =
                """{"email":"user@example.com","node_id":"$NODE_ID","ts":1762790000,"type":"REGISTER_EMAIL_V1"}"""
            assertEquals(listOf(payload), signer.signed)
            val request = server.takeRequest()
            assertEquals("POST", request.method)
            assertEquals("/api/register-email", request.path)
            assertEquals(
                """{"payload":"${payload.replace("\"", "\\\"")}","signature":"sig"}""",
                request.body.readUtf8(),
            )
        }
    }

    @Test
    fun submit_whenServerRejects_failsAsRejected() = runTest {
        MockWebServer().use { server ->
            server.enqueue(MockResponse().setResponseCode(401))
            val failure = failureOf { service(server, FakeSigner()).submit(null) }
            assertEquals(EmailSaveFailure.REJECTED, failure)
        }
    }

    @Test
    fun submit_whenServerUnreachable_failsAsNetwork() = runTest {
        val server = MockWebServer()
        server.start()
        val service = service(server, FakeSigner())
        server.shutdown()

        assertEquals(EmailSaveFailure.NETWORK, failureOf { service.submit(email("a@b.co")) })
    }

    @Test
    fun submit_whenNodeNotRunning_failsWithoutNetworkCall() = runTest {
        MockWebServer().use { server ->
            val notReady = failureOf {
                service(server, FakeSigner(readyNodeId = null)).submit(email("a@b.co"))
            }
            val signFails = failureOf {
                service(server, FakeSigner(error = IllegalStateException())).submit(email("a@b.co"))
            }

            assertEquals(EmailSaveFailure.WALLET_NOT_READY, notReady)
            assertEquals(EmailSaveFailure.WALLET_NOT_READY, signFails)
            assertEquals(0, server.requestCount)
        }
    }
}

class EmailSettingsModelTest {
    private class StubSubmitter(private val failure: EmailSaveFailure? = null) : EmailSubmitter {
        val submitted = mutableListOf<EmailAddress?>()

        override suspend fun submit(email: EmailAddress?) {
            submitted += email
            failure?.let { throw EmailSaveException(it) }
        }
    }

    @Test
    fun save_submitsNormalizedEmailStoresItAndAnswersPrompt() = runTest {
        val stub = StubSubmitter()
        val store = InMemorySavedEmail()
        var savedCount = 0
        val model = EmailSettingsModel(stub, store, onSaved = { savedCount++ })
        model.onInputChange(" User@Example.com ")

        model.save()

        assertEquals(listOf("user@example.com"), stub.submitted.map { it?.value })
        assertEquals(EmailSettingsModel.Status.SAVED, model.status)
        assertEquals("user@example.com", store.savedEmail?.value)
        assertEquals(store.savedEmail, model.savedEmail)
        assertFalse(model.showsField)
        assertEquals(1, savedCount)
    }

    @Test
    fun save_withInvalidInput_doesNotSubmitAndShowsHint() = runTest {
        val stub = StubSubmitter()
        val model = EmailSettingsModel(stub, InMemorySavedEmail())
        model.onInputChange("not-an-email")
        assertFalse(model.canSave)
        assertFalse(model.showsInvalidHint)

        model.save()

        assertTrue(stub.submitted.isEmpty())
        assertTrue(model.showsInvalidHint)
        assertEquals(EmailSettingsModel.Status.IDLE, model.status)
    }

    @Test
    fun invalidHint_showsOnlyAfterFieldLosesFocus() {
        val model = EmailSettingsModel(StubSubmitter(), InMemorySavedEmail())
        model.onFocusChange(false)
        model.onInputChange("bad")
        assertFalse(model.showsInvalidHint)

        model.onFocusChange(true)
        assertFalse(model.showsInvalidHint)
        model.onFocusChange(false)
        assertTrue(model.showsInvalidHint)

        model.onInputChange("")
        assertFalse(model.showsInvalidHint)
    }

    @Test
    fun save_whenServerFails_keepsFailureAndStore() = runTest {
        val store = InMemorySavedEmail()
        var savedCount = 0
        val model =
            EmailSettingsModel(
                StubSubmitter(EmailSaveFailure.NETWORK),
                store,
                onSaved = { savedCount++ },
            )
        model.onInputChange("user@example.com")

        model.save()

        assertEquals(EmailSettingsModel.Status.FAILED, model.status)
        assertEquals(EmailSaveFailure.NETWORK, model.failure)
        assertNull(store.savedEmail)
        assertEquals(0, savedCount)

        model.onInputChange("user@example.co")
        assertNull(model.failure)
        assertEquals(EmailSettingsModel.Status.IDLE, model.status)
    }

    @Test
    fun startChange_prefillsSavedAddressAndShowsField() {
        val model =
            EmailSettingsModel(StubSubmitter(), InMemorySavedEmail(email("old@example.com")))
        assertFalse(model.showsField)

        model.startChange()

        assertTrue(model.showsField)
        assertEquals("old@example.com", model.input)
    }

    @Test
    fun remove_sendsNullAndClearsStore() = runTest {
        val stub = StubSubmitter()
        val store = InMemorySavedEmail(email("old@example.com"))
        val model = EmailSettingsModel(stub, store)

        model.remove()

        assertEquals(listOf<EmailAddress?>(null), stub.submitted)
        assertNull(store.savedEmail)
        assertNull(model.savedEmail)
        assertTrue(model.showsField)
    }

    @Test
    fun remove_whenServerRejects_keepsSavedAddress() = runTest {
        val store = InMemorySavedEmail(email("old@example.com"))
        val model = EmailSettingsModel(StubSubmitter(EmailSaveFailure.REJECTED), store)

        model.remove()

        assertEquals(EmailSaveFailure.REJECTED, model.failure)
        assertEquals("old@example.com", store.savedEmail?.value)
        assertFalse(model.showsField)
    }
}

class EmailPromptPolicyTest {
    private fun shows(
        launches: Int,
        shown: Int = 0,
        lastShown: Int = 0,
        responded: Boolean = false,
    ) = EmailPromptPolicy.shouldShow(launches, shown, lastShown, responded)

    @Test
    fun shows_onLaunches3_9_27_whenSwipedAway() {
        assertFalse(shows(2))
        assertTrue(shows(3))
        assertFalse(shows(8, shown = 1, lastShown = 3))
        assertTrue(shows(9, shown = 1, lastShown = 3))
        assertFalse(shows(26, shown = 2, lastShown = 9))
        assertTrue(shows(27, shown = 2, lastShown = 9))
        assertFalse(shows(1000, shown = 3, lastShown = 27))
    }

    @Test
    fun lateFirstShow_waitsThreeTimesAsLong() {
        assertFalse(shows(21, shown = 1, lastShown = 20))
        assertTrue(shows(60, shown = 1, lastShown = 20))
    }

    @Test
    fun neverShows_afterAnswer() {
        assertFalse(shows(3, responded = true))
        assertFalse(shows(9, shown = 1, lastShown = 3, responded = true))
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class EmailPromptStoreTest {
    private fun prefs() =
        RuntimeEnvironment.getApplication()
            .getSharedPreferences("email_prompt_test", Context.MODE_PRIVATE)
            .also { it.edit().clear().commit() }

    @Test
    fun recordLaunchOnce_countsOncePerProcess() {
        val prefs = prefs()
        val store = EmailPromptStore(prefs)
        store.recordLaunchOnce()
        store.recordLaunchOnce()
        assertEquals(1, store.launches)

        EmailPromptStore(prefs).recordLaunchOnce()
        assertEquals(2, EmailPromptStore(prefs).launches)
    }

    @Test
    fun recordShown_countsAndRemembersTheLaunch() {
        val prefs = prefs()
        val store = EmailPromptStore(prefs)
        store.recordLaunchOnce()
        store.recordShown()

        val reopened = EmailPromptStore(prefs)
        assertEquals(1, reopened.timesShown)
        assertEquals(1, reopened.lastShownLaunch)
    }

    // A restored wallet is a different node: the old address and answers must not carry over.
    @Test
    fun reset_clearsEverythingForANewWallet() {
        val prefs = prefs()
        val store = EmailPromptStore(prefs)
        store.recordLaunchOnce()
        store.recordShown()
        store.markResponded()
        store.savedEmail = email("user@example.com")

        store.reset()

        val reopened = EmailPromptStore(prefs)
        assertEquals(0, reopened.launches)
        assertEquals(0, reopened.timesShown)
        assertEquals(0, reopened.lastShownLaunch)
        assertFalse(reopened.hasResponded)
        assertNull(reopened.savedEmail)
    }

    @Test
    fun markResponded_persists() {
        val prefs = prefs()
        EmailPromptStore(prefs).markResponded()
        assertTrue(EmailPromptStore(prefs).hasResponded)
    }

    @Test
    fun savedEmail_persistsAndClears() {
        val prefs = prefs()
        EmailPromptStore(prefs).savedEmail = email("user@example.com")
        assertEquals("user@example.com", EmailPromptStore(prefs).savedEmail?.value)

        EmailPromptStore(prefs).savedEmail = null
        assertNull(EmailPromptStore(prefs).savedEmail)
    }
}
