package com.stablechannels.app.ui.settings

/** Text rules for the emergency force close dialog (#363). */
internal object ForceCloseConfirmation {
    const val PHRASE = "FORCE CLOSE"

    fun isConfirmed(input: String): Boolean = input.trim() == PHRASE
}
