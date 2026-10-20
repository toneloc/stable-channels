package com.stablechannels.app.services

import com.stablechannels.app.domain.send.BitcoinNetwork
import com.stablechannels.app.domain.send.DecodedBolt11
import com.stablechannels.app.domain.send.DecodedBolt12
import com.stablechannels.app.domain.send.LightningInvoiceDecoder
import org.lightningdevkit.ldknode.Bolt11Invoice
import org.lightningdevkit.ldknode.Bolt11InvoiceDescription
import org.lightningdevkit.ldknode.Network
import org.lightningdevkit.ldknode.Offer

/** Parses invoices and offers with LDK Node. */
object LdkLightningInvoiceDecoder : LightningInvoiceDecoder {
    override fun decodeBolt11(raw: String): DecodedBolt11? = runCatching {
        val invoice = Bolt11Invoice.fromStr(raw)
        DecodedBolt11(
            network = invoice.network().toDomain(),
            amountMsat = invoice.amountMilliSatoshis()?.toLong(),
            descriptionHash =
                (invoice.invoiceDescription() as? Bolt11InvoiceDescription.Hash)?.hash,
            expiresAtEpochSecs =
                saturatedSum(invoice.secondsSinceEpoch(), invoice.expiryTimeSeconds()),
        )
    }
        .getOrNull()

    override fun decodeBolt12(raw: String): DecodedBolt12? = runCatching {
        DecodedBolt12(Offer.fromStr(raw).chains().map { it.toDomain() }.toSet())
    }
        .getOrNull()
}

fun Network.toDomain(): BitcoinNetwork =
    when (this) {
        Network.BITCOIN -> BitcoinNetwork.BITCOIN
        Network.TESTNET -> BitcoinNetwork.TESTNET
        Network.SIGNET -> BitcoinNetwork.SIGNET
        Network.REGTEST -> BitcoinNetwork.REGTEST
    }

/** [a] + [b] as a Long, capped at Long.MAX_VALUE instead of wrapping. */
private fun saturatedSum(a: ULong, b: ULong): Long {
    val max = Long.MAX_VALUE.toULong()
    return if (a > max || b > max - a) Long.MAX_VALUE else (a + b).toLong()
}
