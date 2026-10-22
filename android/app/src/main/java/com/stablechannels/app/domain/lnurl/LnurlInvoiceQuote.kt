package com.stablechannels.app.domain.lnurl

/** How an LNURL invoice's description relates to the payee metadata. */
sealed class LnurlDescriptionBinding {
    data object MatchesMetadata : LnurlDescriptionBinding()

    data class MismatchedHash(val invoiceHash: String) : LnurlDescriptionBinding()

    /** The invoice has a plain description instead of a hash. */
    data object Direct : LnurlDescriptionBinding()
}

/**
 * A checked LNURL invoice for the confirm step. It fixes the amount the confirm step shows, checks
 * and pays; the slide pays exactly this invoice, and only while it is unexpired.
 */
data class LnurlInvoiceQuote(
    val invoice: String,
    val amountMsat: Long,
    /** The comment sent to the payee, normalized by [normalizedComment]. */
    val comment: String?,
    val binding: LnurlDescriptionBinding,
    val expiresAtEpochSecs: Long,
) {
    /**
     * Not bound to the payee metadata. Custom-domain addresses that forward to a provider cause
     * this legitimately, so it is audit-logged rather than shown.
     */
    val isDescriptionUnbound: Boolean
        get() = binding != LnurlDescriptionBinding.MatchesMetadata

    /**
     * Counts as expired [EXPIRY_MARGIN_SECS] early: authentication can take a while before
     * dispatch.
     */
    fun isExpired(nowEpochSecs: Long): Boolean =
        nowEpochSecs + EXPIRY_MARGIN_SECS >= expiresAtEpochSecs

    companion object {
        const val EXPIRY_MARGIN_SECS = 30L

        /** Trimmed comment, or null when blank: what is sent to the payee. */
        fun normalizedComment(comment: String?): String? =
            comment?.trim()?.takeIf { it.isNotEmpty() }
    }
}
