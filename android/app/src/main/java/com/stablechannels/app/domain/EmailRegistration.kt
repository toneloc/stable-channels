package com.stablechannels.app.domain

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** Signs messages with the wallet's Lightning node key. */
interface NodeMessageSigner {
    /** The running node's id, or null while the node is not running. */
    val readyNodeId: String?

    fun sign(message: ByteArray): String
}

/**
 * The last timestamp signed for this wallet. The server refuses a change whose timestamp is not
 * newer than the last one it saw, so each signature must come strictly after this one.
 */
interface SigningTimestampStoring {
    var lastSignedTs: Long
}

/** The signed REGISTER_EMAIL_V1 request; a null email asks the server to remove the saved one. */
object EmailRegistration {
    const val MESSAGE_TYPE = "REGISTER_EMAIL_V1"

    /** How far the server lets a signed timestamp be from its own clock. */
    const val SERVER_WINDOW_SECS = 300L

    /**
     * Now, or one second past the last signature when that is not yet in the past. A last signature
     * beyond the server window came from a clock that was ahead; following it would get every save
     * refused until real time caught up, so it is ignored.
     */
    fun nextTimestamp(nowSecs: Long, lastSent: Long): Long {
        val afterLast = lastSent + 1
        return if (afterLast - nowSecs > SERVER_WINDOW_SECS) nowSecs else maxOf(nowSecs, afterLast)
    }

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

/** The last email address the server accepted, kept on the device so Settings can show it. */
interface SavedEmailStoring {
    var savedEmail: EmailAddress?
}

/** Why saving or removing the email failed; the UI shows one line of copy for each. */
enum class EmailSaveFailure {
    WALLET_NOT_READY,
    NETWORK,
    REJECTED,
    /** Bad signature or a timestamp outside the server's window, most often a wrong clock. */
    SIGNATURE_REFUSED,
    /** The server is not taking emails right now. */
    UNAVAILABLE,
}

/** Carries only the failure kind, so the address never reaches a log or crash report. */
class EmailSaveException(val failure: EmailSaveFailure, cause: Throwable? = null) :
    Exception(failure.name, cause)
