package com.stablechannels.app.domain.send

/** An address split into short groups so the user can check it against the one they expect. */
data class ChunkedAddress(val chunks: List<Chunk>, val raw: String) {
    data class Chunk(val index: Int, val text: String, val isHighlighted: Boolean)

    val formatted: String
        get() = chunks.joinToString("  ") { it.text }
}

/** A long invoice or offer shortened to its start and end. */
data class InvoicePreview(
    val prefix: String,
    val middle: String,
    val suffix: String,
    val raw: String,
)

/** How a destination is shown for the user to check before paying. */
sealed class DestinationVisual {
    data class Onchain(val address: ChunkedAddress) : DestinationVisual()

    data class Invoice(val preview: InvoicePreview) : DestinationVisual()

    data class LightningAddress(val handle: String, val domain: String) : DestinationVisual()

    data class Lnurl(val host: String, val rawUrl: String) : DestinationVisual()
}

/**
 * Splits addresses into 4-character groups and marks the first and last groups, which a
 * clipboard-swapping attack is least able to imitate.
 */
object AddressVisualChunker {
    private const val INVOICE_PREVIEW_MIN_LENGTH = 28
    private const val INVOICE_PREFIX_LENGTH = 14
    private const val INVOICE_SUFFIX_LENGTH = 10
    private const val INVOICE_ELLIPSIS = "········"

    fun format(destination: SendDestination): DestinationVisual =
        when (destination) {
            is SendDestination.Onchain -> DestinationVisual.Onchain(chunk(destination.address))
            is SendDestination.Bolt11,
            is SendDestination.Bolt12 -> DestinationVisual.Invoice(preview(destination.raw))
            is SendDestination.LightningAddress ->
                DestinationVisual.LightningAddress(destination.handle, destination.domain)
            is SendDestination.LnurlPay ->
                DestinationVisual.Lnurl(destination.url.host ?: destination.raw, destination.raw)
        }

    /** Short addresses (4 groups or fewer) highlight one group at each end; longer ones two. */
    fun chunk(raw: String, chunkSize: Int = 4): ChunkedAddress {
        val trimmed = raw.trim()
        if (chunkSize <= 0 || trimmed.isEmpty()) return ChunkedAddress(emptyList(), trimmed)
        val texts = trimmed.chunked(chunkSize)
        val edge = if (texts.size <= 4) 1 else 2
        val chunks = texts.mapIndexed { i, text ->
            ChunkedAddress.Chunk(i, text, isHighlighted = i < edge || i >= texts.size - edge)
        }
        return ChunkedAddress(chunks, trimmed)
    }

    private fun preview(raw: String): InvoicePreview {
        val trimmed = raw.trim()
        if (trimmed.length <= INVOICE_PREVIEW_MIN_LENGTH)
            return InvoicePreview(trimmed, "", "", trimmed)
        return InvoicePreview(
            prefix = trimmed.take(INVOICE_PREFIX_LENGTH),
            middle = INVOICE_ELLIPSIS,
            suffix = trimmed.takeLast(INVOICE_SUFFIX_LENGTH),
            raw = trimmed,
        )
    }
}
