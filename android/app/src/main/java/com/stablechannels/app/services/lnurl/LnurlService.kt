package com.stablechannels.app.services.lnurl

import com.stablechannels.app.domain.lnurl.LnurlError
import com.stablechannels.app.domain.lnurl.LnurlInvoiceQuote
import com.stablechannels.app.domain.lnurl.LnurlInvoiceValidator
import com.stablechannels.app.domain.lnurl.LnurlPayParams
import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.services.AuditService
import java.net.URI
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull

/** LNURL-pay (LUD-06): fetch the payee's parameters, then a checked invoice for an amount. */
class LnurlService(
    private val network: BitcoinNetwork,
    private val invoiceValidator: LnurlInvoiceValidator,
    private val transport: LnurlTransport = LnurlHttpClient(),
    private val nowEpochSecs: () -> Long = { System.currentTimeMillis() / 1000 },
    private val audit: (String, Map<String, Any?>) -> Unit = AuditService::log,
) {
    /** The payee's pay parameters; only `payRequest` with a text description and sane bounds. */
    suspend fun fetchPayParams(url: URI): LnurlPayParams {
        val response = transport.get(url)
        if (response.code == 404) throw LnurlError.RecipientNotFound
        if (response.code !in 200..299) throw LnurlError.InvalidResponse
        val json = parseObject(response.body)
        throwIfErrorResponse(json)

        val params =
            LnurlPayParams(
                tag = json?.string("tag") ?: throw LnurlError.InvalidResponse,
                callback = json.string("callback") ?: throw LnurlError.InvalidResponse,
                minSendable =
                    json.nonNegativeLong("minSendable") ?: throw LnurlError.InvalidResponse,
                maxSendable =
                    json.nonNegativeLong("maxSendable") ?: throw LnurlError.InvalidResponse,
                metadata = json.string("metadata") ?: throw LnurlError.InvalidResponse,
                commentAllowed = json.optionalInt("commentAllowed"),
            )
        if (!params.tag.equals("payRequest", ignoreCase = true))
            throw LnurlError.UnsupportedTag(params.tag)
        if (params.plainTextDescription == null) throw LnurlError.InvalidMetadata
        if (!params.hasValidBounds)
            throw LnurlError.AmountOutOfBounds(params.minSats, params.maxSats)
        return params
    }

    /**
     * The payee's invoice for [amountMsat], checked for network, exact amount and expiry. A
     * description hash that does not match the metadata is audit-logged, not rejected.
     */
    suspend fun fetchInvoice(
        params: LnurlPayParams,
        amountMsat: Long,
        comment: String? = null,
    ): LnurlInvoiceQuote {
        if (!params.isAmountValid(amountMsat))
            throw LnurlError.AmountOutOfBounds(params.minSats, params.maxSats)
        // A blank comment is not sent, so it never needs the payee to allow comments.
        if (!params.isCommentValid(LnurlInvoiceQuote.normalizedComment(comment))) {
            throw LnurlError.InvalidComment
        }

        val callback = buildInvoiceCallbackUrl(params.callback, amountMsat, comment)
        val response = transport.get(callback)
        if (response.code !in 200..299) throw LnurlError.InvalidResponse
        val json = parseObject(response.body)
        throwIfErrorResponse(json)
        val invoice = json?.string("pr") ?: throw LnurlError.InvalidResponse
        if (!isWellFormedSuccessAction(json["successAction"])) throw LnurlError.InvalidResponse

        val checked =
            invoiceValidator.validate(
                invoice,
                amountMsat,
                network,
                params.metadataHashHex,
                nowEpochSecs(),
            )
        val quote =
            LnurlInvoiceQuote(
                invoice = invoice,
                amountMsat = amountMsat,
                comment = LnurlInvoiceQuote.normalizedComment(comment),
                binding = checked.binding,
                expiresAtEpochSecs = checked.expiresAtEpochSecs,
            )
        if (quote.isDescriptionUnbound) {
            audit(
                "LNURL_INVOICE_DESCRIPTION_UNBOUND",
                mapOf(
                    "binding" to checked.binding.toString(),
                    "expected_hash" to params.metadataHashHex,
                    "callback_host" to callback.host.orEmpty(),
                ),
            )
        }
        return quote
    }

    companion object {
        /** [callback] with `amount` and, when not blank, the trimmed `comment` appended. */
        fun buildInvoiceCallbackUrl(callback: String, amountMsat: Long, comment: String?): URI {
            val builder = callback.toHttpUrlOrNull()?.newBuilder() ?: throw LnurlError.InvalidTarget
            builder.addQueryParameter("amount", amountMsat.toString())
            LnurlInvoiceQuote.normalizedComment(comment)?.let {
                builder.addQueryParameter("comment", it)
            }
            return URI(builder.build().toString())
        }

        private val json = Json { ignoreUnknownKeys = true }

        private fun parseObject(body: String): JsonObject? = runCatching {
            json.parseToJsonElement(body) as? JsonObject
        }
            .getOrNull()

        /** LUD-06 error object: `status` ERROR, or a bare `reason` with no `tag` or `pr`. */
        private fun throwIfErrorResponse(json: JsonObject?) {
            json ?: return
            val reason = json.string("reason")
            if (json.string("status").equals("ERROR", ignoreCase = true)) {
                throw LnurlError.ErrorResponse(reason ?: "The recipient service reported an error.")
            }
            if (!reason.isNullOrEmpty() && json["tag"] == null && json["pr"] == null) {
                throw LnurlError.ErrorResponse(reason)
            }
        }

        private fun JsonObject.string(key: String): String? =
            (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content

        /** Absent or null is null; any other non-integer value is [LnurlError.InvalidResponse]. */
        private fun JsonObject.optionalInt(key: String): Int? {
            val value = this[key] ?: return null
            if (value is JsonNull) return null
            return (value as? JsonPrimitive)?.takeUnless { it.isString }?.intOrNull
                ?: throw LnurlError.InvalidResponse
        }

        /**
         * A success action, when present, is an object with a string `tag` and string-or-null
         * fields.
         */
        private fun isWellFormedSuccessAction(element: JsonElement?): Boolean {
            if (element == null || element is JsonNull) return true
            val action = element as? JsonObject ?: return false
            if (action.string("tag") == null) return false
            return listOf("description", "url", "message", "ciphertext", "iv").all { key ->
                val field = action[key]
                field == null || field is JsonNull || (field as? JsonPrimitive)?.isString == true
            }
        }

        private fun JsonObject.nonNegativeLong(key: String): Long? =
            (this[key] as? JsonPrimitive)
                ?.takeUnless { it.isString }
                ?.longOrNull
                ?.takeIf { it >= 0 }
    }
}
