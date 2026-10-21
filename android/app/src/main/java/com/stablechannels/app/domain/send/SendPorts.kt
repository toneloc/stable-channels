package com.stablechannels.app.domain.send

/** What the send flow reads from the wallet. */
interface SendWallet {
    /** The chain the node runs on, or null before it starts. */
    val network: BitcoinNetwork?

    val isOnline: Boolean

    /** Trusted price for converting amounts, or 0 when no price is trusted. */
    fun accountingPrice(): Double

    fun balances(): SendBalanceSnapshot

    /** Status of an earlier outbound payment for [paymentHash], or null when there is none. */
    fun outboundPaymentStatus(paymentHash: String): OutboundPaymentStatus?
}

/** Node calls and wallet bookkeeping a send makes. Implementations may block. */
interface SendNode {
    /** Pays [invoice]; a null [amountMsat] pays the amount the invoice carries. */
    fun payBolt11(invoice: String, amountMsat: Long?): String

    fun payBolt12(offer: String, amountMsat: Long): String

    /** Returns the txid. A null [feeRateSatVb] lets the node pick the rate. */
    fun sendOnchain(address: String, amountSats: Long, feeRateSatVb: Double?): String

    /** Empties the on-chain wallet; the fee comes out of the balance. Returns the txid. */
    fun sendAllOnchain(address: String, feeRateSatVb: Double?): String

    fun spliceOut(channel: SpliceChannelCandidate, address: String, amountSats: Long)

    /** Throws when the send would spend sats still owed to the LSP. */
    fun ensureNoUnsettledSurplus(amountMsat: Long)

    /** Records the pending splice-out before the node call, so it survives a restart. */
    fun beginSpliceOut(amountSats: Long, address: String, price: Double)

    fun cancelSpliceStart()

    fun onchainBroadcast(amountSats: Long, isSendAll: Boolean, txid: String)
}

/** Payment history writes, injected so tests can use a fake. */
interface SendPaymentRecorder {
    fun recordLightning(paymentId: String, paymentType: String, amountMsat: Long, price: Double)

    fun recordOnchain(txid: String, amountMsat: Long, price: Double, address: String)
}

/** Confirms the user may send, for example with biometrics. False blocks the send. */
fun interface SendAuthenticator {
    suspend fun authorize(destination: SendDestination): Boolean
}
