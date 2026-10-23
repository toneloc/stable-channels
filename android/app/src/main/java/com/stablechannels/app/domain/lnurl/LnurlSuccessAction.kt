package com.stablechannels.app.domain.lnurl

import java.net.URI
import java.util.Base64

/** LUD-09/LUD-10 success action returned with an LNURL invoice, as received. */
data class LnurlSuccessAction(
    val tag: String,
    val description: String? = null,
    val url: String? = null,
    val message: String? = null,
    val ciphertext: String? = null,
    val iv: String? = null,
) {
    sealed class Action {
        data class Message(val text: String) : Action()

        data class Url(val description: String, val url: URI) : Action()

        data class Aes(val description: String, val ciphertext: String, val iv: String) : Action()

        data class Unknown(val tag: String) : Action()
    }

    /**
     * The action if it is within LUD-09 limits. A URL action must stay on the callback host or a
     * subdomain of it and use HTTPS (HTTP only for `.onion`); anything else is [Action.Unknown].
     */
    fun validated(callbackUrl: URI): Action {
        val unknown = Action.Unknown(tag)
        return when (tag.lowercase()) {
            "message" ->
                message
                    ?.takeIf { it.isNotEmpty() && it.graphemeCount() <= MAX_TEXT }
                    ?.let(Action::Message)
            "url" -> {
                val target =
                    url?.takeIf { it.length <= MAX_URL }
                        ?.let { runCatching { URI(it) }.getOrNull() }
                val scheme = target?.scheme?.lowercase()
                val host = target?.host?.lowercase()
                val secure =
                    scheme == "https" || (scheme == "http" && host?.endsWith(".onion") == true)
                if (
                    description != null &&
                        description.graphemeCount() <= MAX_TEXT &&
                        secure &&
                        isSameHostOrSubdomain(callbackUrl)
                )
                    Action.Url(description, target!!)
                else null
            }
            "aes" -> {
                val ivBytes = iv?.let { runCatching { Base64.getDecoder().decode(it) }.getOrNull() }
                val cipherOk =
                    ciphertext != null &&
                        ciphertext.isNotEmpty() &&
                        ciphertext.length <= MAX_CIPHERTEXT &&
                        runCatching { Base64.getDecoder().decode(ciphertext) }.isSuccess
                if (
                    description != null &&
                        description.graphemeCount() <= MAX_TEXT &&
                        ivBytes?.size == 16 &&
                        cipherOk
                ) {
                    Action.Aes(description, ciphertext!!, iv!!)
                } else null
            }
            else -> null
        } ?: unknown
    }

    /** True when the action URL's host is the callback host or a subdomain of it. */
    fun isSameHostOrSubdomain(callbackUrl: URI): Boolean {
        val actionHost =
            url?.let { runCatching { URI(it).host }.getOrNull() }?.lowercase() ?: return false
        val callbackHost = callbackUrl.host?.lowercase() ?: return false
        return actionHost == callbackHost || actionHost.endsWith(".$callbackHost")
    }

    private companion object {
        const val MAX_TEXT = 144
        const val MAX_URL = 2048
        const val MAX_CIPHERTEXT = 4096
    }
}
