package com.stablechannels.app.domain

/**
 * Decides when Home offers the email prompt: to anyone who has opened the wallet a few times,
 * funded or not. "Not now" or a swipe only dismisses it: it returns after gaps of 3, 9 and 27
 * launches ([SHOW_GAPS]), and the cycle repeats. Only Save ends it.
 */
object EmailPromptPolicy {
    val SHOW_GAPS = listOf(3, 9, 27)

    fun shouldShow(
        launches: Int,
        timesShown: Int,
        lastShownLaunch: Int,
        hasResponded: Boolean,
    ): Boolean {
        if (hasResponded) return false
        return launches >= lastShownLaunch + SHOW_GAPS[timesShown % SHOW_GAPS.size]
    }

    /**
     * Whether Home opens the prompt now. Home asks right before opening it, so it never covers
     * another Home sheet.
     */
    fun shouldShowOnHome(store: EmailPromptStoring, otherSheetShowing: Boolean): Boolean =
        !otherSheetShowing &&
            shouldShow(
                launches = store.launches,
                timesShown = store.timesShown,
                lastShownLaunch = store.lastShownLaunch,
                hasResponded = store.hasResponded,
            )
}

/**
 * Where the prompt keeps its launch count, when it was shown and the answer; the app stores it in
 * SharedPreferences.
 */
interface EmailPromptStoring {
    val launches: Int
    val hasResponded: Boolean
    val timesShown: Int
    val lastShownLaunch: Int

    fun recordLaunchOnce()

    fun recordShown()

    fun markResponded()
}
