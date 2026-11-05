package com.stablechannels.app.domain.send

import java.net.URI

/** A validated send destination. */
sealed class SendDestination {
    /** The destination as the user would recognise it. */
    abstract val raw: String

    /** True when the user must enter the amount. */
    abstract val requiresManualAmount: Boolean

    /** True when the payment travels over Lightning rather than on-chain. */
    val isLightning: Boolean
        get() = this !is Onchain

    data class Bolt11(override val raw: String, val amountMsat: Long?) : SendDestination() {
        override val requiresManualAmount: Boolean
            get() = amountMsat == null || amountMsat == 0L
    }

    data class Bolt12(override val raw: String) : SendDestination() {
        override val requiresManualAmount: Boolean
            get() = true
    }

    data class Onchain(val address: String, val amountSats: Long?) : SendDestination() {
        override val raw: String
            get() = address

        override val requiresManualAmount: Boolean
            get() = amountSats == null || amountSats == 0L
    }

    data class LightningAddress(val handle: String, val domain: String, val url: URI) :
        SendDestination() {
        override val raw: String
            get() = "$handle@$domain"

        override val requiresManualAmount: Boolean
            get() = true
    }

    data class LnurlPay(val url: URI) : SendDestination() {
        override val raw: String
            get() = url.toString()

        override val requiresManualAmount: Boolean
            get() = true
    }
}

/** Why an entered destination was rejected. The UI maps each reason to localized text. */
enum class DestinationRejection {
    WRONG_NETWORK,
    INVALID_BOLT11,
    INVALID_BOLT12,
    INVALID_LNURL,
    UNSUPPORTED_LIGHTNING_PREFIX,
    INVALID_LIGHTNING_ADDRESS,
    INVALID_ONCHAIN_ADDRESS,
    UNHANDLED_REQUIRED_BIP21_PARAMETER,
    DUPLICATE_BIP21_PARAMETER,
    UNRECOGNIZED,
}

/** Result of classifying user input as a payment destination. */
sealed class DestinationClassification {
    data object Empty : DestinationClassification()

    data class Valid(val destination: SendDestination) : DestinationClassification()

    data class Invalid(val reason: DestinationRejection) : DestinationClassification()
}

/** The amount the destination itself carries (a BIP-21 amount or a fixed invoice), or null. */
val SendDestination.presetSats: Long?
    get() =
        when (this) {
            is SendDestination.Onchain -> amountSats
            is SendDestination.Bolt11 -> amountMsat?.let { it / 1000 }
            else -> null
        }?.takeIf { it > 0 }
