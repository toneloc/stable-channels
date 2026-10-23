package com.stablechannels.app.domain.lnurl

import java.security.MessageDigest
import java.util.Base64
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive

/** LUD-06 pay parameters from the first LNURL-pay request. Amounts are millisatoshis. */
data class LnurlPayParams(
    val tag: String,
    val callback: String,
    val minSendable: Long,
    val maxSendable: Long,
    val metadata: String,
    val commentAllowed: Int? = null,
) {
    /** Smallest amount in whole sats, rounded up so it still meets [minSendable]. */
    val minSats: Long
        get() = minSendable / 1000 + if (minSendable % 1000 > 0) 1 else 0

    /** Largest amount in whole sats, rounded down so it still meets [maxSendable]. */
    val maxSats: Long
        get() = maxSendable / 1000

    val hasCustomSendBounds: Boolean
        get() = minSendable > 1000 || maxSendable < MAX_SUPPLY_MSAT

    val hasValidBounds: Boolean
        get() = minSendable > 0 && minSendable <= maxSendable && maxSendable <= MAX_SUPPLY_MSAT

    fun isAmountValid(msat: Long): Boolean = msat in minSendable..maxSendable

    /** LUD-12: no comment is always fine; otherwise the payee must allow one this long. */
    fun isCommentValid(comment: String?): Boolean {
        if (comment.isNullOrEmpty()) return true
        val allowed = commentAllowed ?: return false
        return allowed > 0 && comment.graphemeCount() <= allowed
    }

    /** Hex SHA-256 of the metadata; LUD-06 asks the invoice description hash to equal it. */
    val metadataHashHex: String
        get() =
            MessageDigest.getInstance("SHA-256").digest(metadata.toByteArray()).joinToString("") {
                "%02x".format(it)
            }

    /** The `text/plain` entry of the metadata, required by LUD-06. */
    val plainTextDescription: String?
        get() = metadataEntries().firstOrNull { it.first == "text/plain" }?.second

    /** The PNG or JPEG avatar bytes from the metadata, if any. */
    val avatarBytes: ByteArray?
        get() {
            val base64 =
                metadataEntries().firstOrNull { it.first in AVATAR_MIME_TYPES }?.second
                    ?: return null
            return runCatching { Base64.getMimeDecoder().decode(base64) }.getOrNull()
        }

    /**
     * `[["mime", "value"], ...]` pairs from entries with at least two items. LUD-06 metadata is an
     * array of string arrays; anything else makes the whole metadata unusable.
     */
    private fun metadataEntries(): List<Pair<String, String>> {
        val array = runCatching { Json.parseToJsonElement(metadata) as? JsonArray }.getOrNull()
        val rows =
            array?.map { item ->
                (item as? JsonArray)?.map {
                    (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.content
                }
            } ?: return emptyList()
        if (rows.any { row -> row == null || row.any { it == null } }) return emptyList()
        return rows.mapNotNull { row -> row!!.takeIf { it.size >= 2 }?.let { it[0]!! to it[1]!! } }
    }

    companion object {
        const val MAX_SUPPLY_MSAT = 21_000_000L * 100_000_000L * 1000L
        private val AVATAR_MIME_TYPES = setOf("image/png;base64", "image/jpeg;base64")
    }
}
