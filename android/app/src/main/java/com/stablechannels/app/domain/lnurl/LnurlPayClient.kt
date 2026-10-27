package com.stablechannels.app.domain.lnurl

import java.net.URI

/** Resolves LNURL-pay endpoints. The send flow depends on this port so tests can use a fake. */
interface LnurlPayClient {
    suspend fun fetchPayParams(url: URI): LnurlPayParams

    /** The payee's invoice for [amountMsat], already checked for network, amount and expiry. */
    suspend fun fetchInvoice(
        params: LnurlPayParams,
        amountMsat: Long,
        comment: String? = null,
    ): LnurlInvoiceQuote
}
