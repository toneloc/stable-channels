package com.stablechannels.app.services

import com.stablechannels.app.domain.EmailAddress
import com.stablechannels.app.domain.EmailSaveException
import com.stablechannels.app.domain.EmailSaveFailure
import com.stablechannels.app.domain.NodeMessageSigner
import com.stablechannels.app.util.Constants
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

/** Saves the email the user chose to share on the Stable Channels server; null removes it. */
fun interface EmailSubmitter {
    /** Throws [EmailSaveException] when the server did not save the change. */
    suspend fun submit(email: EmailAddress?)
}

/** The signed REGISTER_EMAIL_V1 request; a null email asks the server to remove the saved one. */
object EmailRegistration {
    const val MESSAGE_TYPE = "REGISTER_EMAIL_V1"

    // Keys in sorted order so the bytes match the iOS client for the same inputs.
    fun payload(email: EmailAddress?, nodeId: String, timestampSecs: Long): String =
        buildJsonObject {
            put("email", email?.value.orEmpty())
            put("node_id", nodeId)
            put("ts", timestampSecs)
            put("type", MESSAGE_TYPE)
        }
        .toString()

    fun envelope(payload: String, signature: String): String = buildJsonObject {
        put("payload", payload)
        put("signature", signature)
    }
        .toString()
}

/** Signs with the running node; not ready until the node has started. */
class NodeServiceSigner(private val nodeService: NodeService) : NodeMessageSigner {
    override val readyNodeId: String?
        get() = nodeService.nodeId.takeIf { nodeService.isRunning && it.isNotEmpty() }

    override fun sign(message: ByteArray): String = nodeService.signMessage(message)
}

class EmailSubmissionService(
    private val signer: NodeMessageSigner,
    private val endpoint: String = Constants.EMAIL_SUBMIT_URL,
    private val client: OkHttpClient =
        OkHttpClient.Builder()
            .callTimeout(Constants.EMAIL_SUBMIT_TIMEOUT_SECS, TimeUnit.SECONDS)
            .build(),
    private val nowSecs: () -> Long = { System.currentTimeMillis() / 1_000 },
) : EmailSubmitter {
    override suspend fun submit(email: EmailAddress?) =
        withContext(Dispatchers.IO) {
            val body = signedEnvelope(email).toRequestBody("application/json".toMediaType())
            val request = Request.Builder().url(endpoint).post(body).build()
            val accepted =
                try {
                    client.newCall(request).execute().use { it.isSuccessful }
                } catch (e: IOException) {
                    throw EmailSaveException(EmailSaveFailure.NETWORK, e)
                }
            if (!accepted) throw EmailSaveException(EmailSaveFailure.REJECTED)
        }

    private fun signedEnvelope(email: EmailAddress?): String {
        val nodeId =
            signer.readyNodeId ?: throw EmailSaveException(EmailSaveFailure.WALLET_NOT_READY)
        val payload = EmailRegistration.payload(email, nodeId, nowSecs())
        val signature =
            try {
                signer.sign(payload.toByteArray(Charsets.UTF_8))
            } catch (e: Exception) {
                throw EmailSaveException(EmailSaveFailure.WALLET_NOT_READY, e)
            }
        return EmailRegistration.envelope(payload, signature)
    }
}
