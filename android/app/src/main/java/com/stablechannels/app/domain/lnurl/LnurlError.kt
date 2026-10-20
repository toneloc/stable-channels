package com.stablechannels.app.domain.lnurl

/** Why an LNURL-pay step failed. The UI maps each case to localized text. */
sealed class LnurlError : Exception() {
    /** The address, LNURL or URL cannot be used. */
    data object InvalidTarget : LnurlError()

    /** The endpoint, or a redirect, points somewhere the wallet must not connect to. */
    data object InsecureEndpoint : LnurlError()

    data object InvalidResponse : LnurlError()

    data object InvalidComment : LnurlError()

    data object InvalidMetadata : LnurlError()

    data object RecipientNotFound : LnurlError()

    /** `.onion` endpoints need a Tor transport, which the app does not have. */
    data object OnionUnsupported : LnurlError()

    data object TooManyRedirects : LnurlError()

    /** The payee reported an error; [reason] is its text, shown as is. */
    data class ErrorResponse(val reason: String) : LnurlError()

    data class UnsupportedTag(val tag: String) : LnurlError()

    data class AmountOutOfBounds(val minSats: Long, val maxSats: Long) : LnurlError()

    data class InvoiceAmountMismatch(val expectedMsat: Long, val actualMsat: Long) : LnurlError()

    data object InvoiceWrongNetwork : LnurlError()

    data object InvoiceWithoutAmount : LnurlError()

    data object InvoiceExpired : LnurlError()

    data class Network(val detail: String) : LnurlError()
}
