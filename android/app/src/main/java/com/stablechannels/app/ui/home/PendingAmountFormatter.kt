package com.stablechannels.app.ui.home

import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.btcSpacedFormatted
import com.stablechannels.app.util.usdFormatted

/**
 * Pure formatting for pending on-chain amounts, extracted so it's directly unit-testable (Compose
 * UI has no unit-test harness here). Falls back to a BTC figure instead of dropping the amount when
 * the price feed is momentarily unavailable.
 */
object PendingAmountFormatter {
    fun amountText(amountSats: Long?, btcPrice: Double): String? {
        if (amountSats == null) return null
        val amountUSD =
            if (btcPrice > 0) (amountSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice else null
        return amountUSD?.usdFormatted() ?: "${amountSats.btcSpacedFormatted()} BTC"
    }

    fun moveToLightningLabel(spendableSats: Long, btcPrice: Double): String {
        return if (btcPrice > 0) {
            val spendableUSD = (spendableSats.toDouble() / Constants.SATS_IN_BTC) * btcPrice
            "Move ${spendableUSD.usdFormatted()} to Lightning"
        } else {
            "Move ${spendableSats.btcSpacedFormatted()} BTC to Lightning"
        }
    }
}
