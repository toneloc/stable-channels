package com.stablechannels.app.services.send

import com.stablechannels.app.domain.lnurl.LnurlInvoiceQuote
import com.stablechannels.app.domain.send.InvoicePayability
import com.stablechannels.app.domain.send.LightningInvoiceDecoder
import com.stablechannels.app.domain.send.LightningPaymentKind
import com.stablechannels.app.domain.send.SendChannelSpendPolicy
import com.stablechannels.app.domain.send.SendDestination
import com.stablechannels.app.domain.send.SendError
import com.stablechannels.app.domain.send.SendFailure
import com.stablechannels.app.domain.send.SendLedger
import com.stablechannels.app.domain.send.SendNode
import com.stablechannels.app.domain.send.SendPaymentDispatcher
import com.stablechannels.app.domain.send.SendPaymentOutcome
import com.stablechannels.app.domain.send.SendPaymentRecorder
import com.stablechannels.app.domain.send.SendPaymentResult
import com.stablechannels.app.domain.send.SendWallet
import com.stablechannels.app.services.AuditService
import com.stablechannels.app.util.Constants
import com.stablechannels.app.util.epochSecondsNow

/**
 * Dispatches a send to Lightning (BOLT11, BOLT12, LNURL) or on-chain (splice-out or wallet) and
 * records it. Calls block on the node, so run them off the main thread. Throws [SendFailure] for
 * known rejections and passes node errors through.
 */
class SendPaymentExecutor(
    private val node: SendNode,
    private val ledger: SendLedger,
    private val wallet: SendWallet,
    private val recorder: SendPaymentRecorder,
    private val decoder: LightningInvoiceDecoder,
    private val nowEpochSecs: () -> Long = ::epochSecondsNow,
    private val audit: (String, Map<String, Any?>) -> Unit = AuditService::log,
) : SendPaymentDispatcher {
    override fun execute(
        destination: SendDestination,
        amountSats: Long,
        feeRateSatVb: Double?,
        lnurlQuote: LnurlInvoiceQuote?,
    ): SendPaymentResult {
        val price = wallet.accountingPrice()
        return when (destination) {
            is SendDestination.Bolt11 -> sendBolt11(destination, amountSats, price)
            is SendDestination.Bolt12 -> sendBolt12(destination, amountSats, price)
            is SendDestination.LightningAddress,
            is SendDestination.LnurlPay -> sendLnurl(lnurlQuote, price)
            is SendDestination.Onchain ->
                sendOnchain(destination.address, amountSats, feeRateSatVb, price)
        }
    }

    /** Empties the on-chain wallet in one transaction, never as a splice-out. */
    override fun sendAllOnchain(address: String, feeRateSatVb: Double?): SendPaymentResult {
        val balances = wallet.balances()
        if (balances.isSweeping) throw SendFailure(SendError.SpliceInProgress)
        val price = wallet.accountingPrice()
        val txid = node.sendAllOnchain(address, feeRateSatVb)
        recorder.recordOnchain(txid, balances.onchainSats * 1000, price, address)
        ledger.onchainBroadcast(balances.onchainSats, isSendAll = true, txid = txid)
        return SendPaymentResult(balances.onchainSats, SendPaymentOutcome.Onchain(txid))
    }

    private fun sendBolt11(
        destination: SendDestination.Bolt11,
        amountSats: Long,
        price: Double,
    ): SendPaymentResult {
        if (decoder.decodeBolt11(destination.raw)?.isExpired(nowEpochSecs()) == true) {
            throw SendFailure(SendError.InvoiceNotPayable(InvoicePayability.EXPIRED))
        }
        val fixedMsat = destination.amountMsat?.takeIf { it > 0 }
        val msat = fixedMsat ?: positiveMsat(amountSats)
        ledger.ensureNoUnsettledSurplus(msat)
        // A fixed-amount invoice is paid as issued; a zero-amount one at the entered amount.
        val paymentId = node.payBolt11(destination.raw, if (fixedMsat != null) null else msat)
        recorder.recordLightning(paymentId, LightningPaymentKind.BOLT11, msat, price)
        return SendPaymentResult(msat / 1000, SendPaymentOutcome.Lightning(paymentId))
    }

    private fun sendBolt12(
        destination: SendDestination.Bolt12,
        amountSats: Long,
        price: Double,
    ): SendPaymentResult {
        val msat = positiveMsat(amountSats)
        ledger.ensureNoUnsettledSurplus(msat)
        val paymentId = node.payBolt12(destination.raw, msat)
        recorder.recordLightning(paymentId, LightningPaymentKind.BOLT12, msat, price)
        return SendPaymentResult(amountSats, SendPaymentOutcome.Lightning(paymentId))
    }

    /** Pays exactly the invoice the confirm step showed; it was checked when quoted. */
    private fun sendLnurl(quote: LnurlInvoiceQuote?, price: Double): SendPaymentResult {
        quote ?: throw SendFailure(SendError.MissingLnurlInvoice)
        ledger.ensureNoUnsettledSurplus(quote.amountMsat)
        val paymentId = node.payBolt11(quote.invoice, quote.amountMsat)
        if (quote.isDescriptionUnbound) {
            audit(
                "LNURL_UNBOUND_INVOICE_PAID",
                mapOf("binding" to quote.binding.toString(), "payment_id" to paymentId),
            )
        }
        recorder.recordLightning(paymentId, LightningPaymentKind.LNURL, quote.amountMsat, price)
        return SendPaymentResult(quote.amountMsat / 1000, SendPaymentOutcome.Lightning(paymentId))
    }

    private fun sendOnchain(
        address: String,
        amountSats: Long,
        feeRateSatVb: Double?,
        price: Double,
    ): SendPaymentResult {
        if (amountSats <= 0) throw SendFailure(SendError.InvalidAmount)
        val balances = wallet.balances()
        // A sweep has committed the on-chain balance to a splice-in. Checked before the branch
        // because isSpliceOut is false while sweeping, which would route around this guard.
        if (balances.isSweeping) throw SendFailure(SendError.SpliceInProgress)
        if (!SendChannelSpendPolicy.isSpliceOut(balances.channels, balances.isSweeping)) {
            val txid = node.sendOnchain(address, amountSats, feeRateSatVb)
            ledger.onchainBroadcast(amountSats, isSendAll = false, txid = txid)
            recorder.recordOnchain(txid, amountSats * 1000, price, address)
            return SendPaymentResult(amountSats, SendPaymentOutcome.Onchain(txid))
        }
        val channel =
            SendChannelSpendPolicy.selectSpliceChannel(
                balances.channels,
                amountSats,
                feeRateSatVb ?: Constants.PLACEHOLDER_FEE_RATE_SAT_VB,
            ) ?: throw SendFailure(SendError.NoSpliceChannel)
        ledger.beginSpliceOut(channel, amountSats, address, price)
        try {
            node.spliceOut(channel, address, amountSats)
        } catch (e: Exception) {
            ledger.cancelSpliceStart()
            throw e
        }
        return SendPaymentResult(amountSats, SendPaymentOutcome.SpliceOut)
    }

    private fun positiveMsat(amountSats: Long): Long {
        if (amountSats <= 0) throw SendFailure(SendError.InvalidAmount)
        return amountSats * 1000
    }
}
