package com.stablechannels.app.ui.transfer.send

import com.stablechannels.app.domain.send.NetworkFeeSpeedTier
import com.stablechannels.app.domain.send.RecommendedFees
import com.stablechannels.app.domain.send.SendAmountCalculator
import com.stablechannels.app.domain.send.SendAmountUnit

// Amount entry for the send flow: typed text, units, shares of the balance, Max and fee tiers.

fun SendFlowModel.setAmountText(text: String) {
    update { it.copy(amountInputText = text) }
    // Max holds only while the text still equals the Max amount.
    if (state.value.isMaxAmount && text != maxAmountText()) {
        update { it.copy(isMaxAmount = false) }
    }
}

fun SendFlowModel.normalizeAmountInput() = update {
    it.copy(
        amountInputText = SendAmountCalculator.normalizeInput(it.amountInputText, it.amountUnit)
    )
}

fun SendFlowModel.switchUnit(unit: SendAmountUnit) = update {
    if (unit == it.amountUnit) it
    else
        it.copy(
            amountUnit = unit,
            amountInputText =
                SendAmountCalculator.switchUnit(
                    it.amountUnit,
                    unit,
                    it.amountInputText,
                    wallet.accountingPrice(),
                ),
        )
}

/** 25% and 50% fill a share of the balance; 100% is Max. */
fun SendFlowModel.applyPercentage(percent: Int) {
    if (percent == 100) {
        update { it.copy(isMaxAmount = true) }
        return refreshMaxAmountText()
    }
    val s = state.value
    // What this destination can spend, as on iOS: Lightning capacity or the on-chain balance.
    val balance = quote()?.availableSats ?: 0
    // An LNURL payee's max bounds the principal, so a share never exceeds it.
    val base = s.lnurlParams?.let { minOf(balance, it.maxSats) } ?: balance
    update {
        it.copy(
            isMaxAmount = false,
            amountInputText =
                SendAmountCalculator.percentageOf(
                    percent,
                    base,
                    it.amountUnit,
                    wallet.accountingPrice(),
                ),
        )
    }
}

fun SendFlowModel.selectFeeTier(tier: NetworkFeeSpeedTier) {
    update { it.copy(selectedFeeTier = tier) }
    refreshMaxAmountText()
}

/** Live fee rates arrived; a Max amount is recomputed with them. */
fun SendFlowModel.applyRecommendedFees(fees: RecommendedFees) {
    update { it.copy(recommendedFees = fees) }
    refreshMaxAmountText()
}

private fun SendFlowModel.maxAmountText(): String {
    val s = state.value
    return SendAmountCalculator.formatSats(
        quote(s)?.amountSats ?: 0,
        s.amountUnit,
        wallet.accountingPrice(),
    )
}

private fun SendFlowModel.refreshMaxAmountText() {
    if (!state.value.isMaxAmount) return
    val text = maxAmountText()
    update { it.copy(amountInputText = text) }
}
