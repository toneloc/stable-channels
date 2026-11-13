package com.stablechannels.app.services

/** How the app lets go of the node when it goes to the background. */
internal data class BackgroundStopPlan(
    val delayMs: Long,
    /**
     * The foreground service reason that keeps the process alive for the delay; null for a plain
     * quick-switch grace.
     */
    val foregroundServiceReason: String?,
)

/** A quick app-switch reconnects instantly instead of restarting LDK. */
internal const val QUICK_SWITCH_GRACE_MS = 10_000L

/** Long enough for a payment wait, the app's own move, or an LSP splice to finish. */
internal const val LONG_BACKGROUND_GRACE_MS = 60_000L

/**
 * A payment wait, an open in-app picker, the app's own negotiating move, or an LSP splice the push
 * announced all keep the node for the long grace behind a foreground service. Anything else gets
 * the quick-switch grace, so a stuck flag degrades to "stop after a minute", never "never stop".
 */
internal fun backgroundStopPlan(
    isWaitingForPayment: Boolean,
    isPickingMedia: Boolean,
    negotiatingSplice: Boolean,
    holdingForLspSplice: Boolean,
): BackgroundStopPlan =
    when {
        negotiatingSplice ->
            BackgroundStopPlan(LONG_BACKGROUND_GRACE_MS, LdkBackgroundService.REASON_SPLICE)
        holdingForLspSplice ->
            BackgroundStopPlan(LONG_BACKGROUND_GRACE_MS, LdkBackgroundService.REASON_LSP_SPLICE)
        isWaitingForPayment || isPickingMedia ->
            BackgroundStopPlan(LONG_BACKGROUND_GRACE_MS, LdkBackgroundService.REASON_PAYMENT)
        else -> BackgroundStopPlan(QUICK_SWITCH_GRACE_MS, null)
    }
