package com.stablechannels.app.domain.send

/** Why a send cannot go ahead. The UI maps each case to localized text. */
sealed class SendError {
    data object Offline : SendError()

    data object FeeRateNotReady : SendError()

    data object InvalidAmount : SendError()

    data class OutsidePayeeLimits(val minSats: Long, val maxSats: Long) : SendError()

    /** The amount alone is more than the wallet can spend. */
    data class AmountExceedsBalance(val availableSats: Long) : SendError()

    /** The amount fits but amount plus fee does not, or nothing is spendable. */
    data class InsufficientBalance(val availableSats: Long) : SendError()
}
