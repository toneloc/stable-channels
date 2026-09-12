package com.stablechannels.app.services

/**
 * Whether a trade failure committed while the app was away should be shown on launch.
 *
 * A rejection is only ever displayed at the moment it is processed (the trade sheet, and a status
 * message set alongside it) — both live in process memory, so a rejection that arrives while the
 * app is backgrounded is silent after the cold start that follows. Startup resurfaces the most
 * recent one, subject to two rules kept here so they can be tested without a ViewModel.
 */
object TradeFailureNotice {
    fun shouldShow(
        failurePaymentId: String,
        lastShownPaymentId: String?,
        capsuleOccupied: Boolean
    ): Boolean {
        // Already told them — including when the live trade flow showed it before the relaunch.
        if (failurePaymentId == lastShownPaymentId) return false
        // Never displace a live message. Not marked seen either, so a later launch can show it.
        if (capsuleOccupied) return false
        return true
    }
}
