package com.stablechannels.app.domain

/** Signs messages with the wallet's Lightning node key. */
interface NodeMessageSigner {
    /** The running node's id, or null while the node is not running. */
    val readyNodeId: String?

    fun sign(message: ByteArray): String
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
}

/** Carries only the failure kind, so the address never reaches a log or crash report. */
class EmailSaveException(val failure: EmailSaveFailure, cause: Throwable? = null) :
    Exception(failure.name, cause)
