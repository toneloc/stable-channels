package com.stablechannels.app.services

import android.database.sqlite.SQLiteException
import org.lightningdevkit.ldknode.NodeException
import org.lightningdevkit.ldknode.PaymentFailureReason

/** Local copy only. Keep the original reason/exception in diagnostics, not in the UI. */
object WalletErrorMessages {
    fun paymentFailure(reason: PaymentFailureReason?): String = when (reason) {
        PaymentFailureReason.RETRIES_EXHAUSTED ->
            "The payment could not complete after several attempts. Try a smaller amount or try again later."
        PaymentFailureReason.ROUTE_NOT_FOUND ->
            "No available Lightning route could carry this payment. Try a smaller amount or try again later."
        PaymentFailureReason.RECIPIENT_REJECTED ->
            "The recipient rejected the payment. Ask them for a new invoice."
        PaymentFailureReason.USER_ABANDONED -> "The payment was cancelled."
        PaymentFailureReason.PAYMENT_EXPIRED ->
            "The payment expired before it completed. Ask the recipient for a new invoice."
        PaymentFailureReason.UNKNOWN_REQUIRED_FEATURES ->
            "This wallet does not support a feature required by the invoice. Update the app or ask for a different invoice."
        PaymentFailureReason.INVOICE_REQUEST_EXPIRED ->
            "The recipient did not return an invoice in time. Check your connection and try again later."
        PaymentFailureReason.INVOICE_REQUEST_REJECTED ->
            "The recipient rejected the invoice request. Check the offer with them before trying again."
        PaymentFailureReason.BLINDED_PATH_CREATION_FAILED ->
            "The wallet could not establish a reply path for this payment. Check your connection and try again later."
        PaymentFailureReason.UNEXPECTED_ERROR ->
            "An unexpected error stopped the payment. Try again later. If it continues, share your logs with support."
        null -> "The payment did not complete. Check its status in History before trying again."
    }

    fun paymentFailureCode(code: String?): String = paymentFailure(
        PaymentFailureReason.entries.firstOrNull { it.name == code }
    )

    fun operation(error: Exception, fallback: String): String = when (error) {
        is NodeException.InsufficientFunds ->
            "Your available balance cannot cover the amount and fees. Reduce the amount."
        is NodeException.DuplicatePayment ->
            "This payment has already been started. Check its status in History before trying again."
        is NodeException.ConnectionFailed, is NodeException.LiquiditySourceUnavailable ->
            "The Lightning provider is unavailable. Check your connection and try again later."
        is NodeException.LiquidityRequestFailed ->
            "The provider could not arrange receiving capacity. Try a smaller amount or try again later."
        is NodeException.LiquidityFeeTooHigh ->
            "The provider's channel opening fee exceeds the allowed limit. Try again later."
        is NodeException.InvalidInvoice -> "This invoice is invalid or expired. Ask the recipient for a new one."
        is NodeException.InvalidOffer -> "This offer is invalid. Ask the recipient for a new one."
        is NodeException.InvalidAddress, is NodeException.InvalidNetwork ->
            "Check that the address or invoice is valid for this wallet's Bitcoin network."
        is NodeException.InvalidAmount -> "Enter a valid amount and try again."
        is NodeException.InvalidFeeRate -> "The fee rate is invalid. Refresh the fee estimate and try again."
        is NodeException.InvalidCustomTlvs ->
            "The wallet could not prepare the provider message. Update the app and try again."
        is NodeException.PaymentSendingFailed ->
            "The wallet could not send the payment. Check your connection and available balance, then try again."
        is NodeException.InvoiceCreationFailed, is NodeException.InvoiceRequestCreationFailed,
        is NodeException.OfferCreationFailed ->
            "The wallet could not create an invoice or offer. Check your connection and try again."
        is NodeException.ChannelCreationFailed, is NodeException.ChannelSplicingFailed ->
            "The channel operation could not start. Check your connection and available balance, then try again."
        is NodeException.ChannelClosingFailed ->
            "The channel could not close. Check your connection to the provider and try again later."
        is NodeException.NotRunning, is NodeService.NodeServiceError -> "The wallet is still starting. Wait for it to connect and try again."
        is NodeException.FeerateEstimationUpdateFailed, is NodeException.FeerateEstimationUpdateTimeout,
        is NodeException.TxSyncFailed, is NodeException.TxSyncTimeout,
        is NodeException.GossipUpdateFailed, is NodeException.GossipUpdateTimeout ->
            "The wallet could not update network information. Check your connection and wait for it to sync."
        is NodeException.PersistenceFailed, is SQLiteException ->
            "The wallet could not save the result. Check History before trying again. If it continues, share your logs with support."
        is NodeException -> fallback
        // These callers also throw local validation exceptions with actionable copy.
        else -> error.message?.takeIf { it.isNotBlank() } ?: fallback
    }
}
