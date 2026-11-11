package com.stablechannels.app.domain

/**
 * Decides when Home offers the email prompt: to anyone who has opened the wallet a few times,
 * funded or not. Only Save ends it; after "Not now" or a swipe it comes back after three times as
 * many launches (3, 9, 27 for a new user), at most [MAX_SHOWS] times.
 */
object EmailPromptPolicy {
    const val FIRST_LAUNCH = 3
    const val LAUNCH_GROWTH = 3
    const val MAX_SHOWS = 3

    fun shouldShow(
        launches: Int,
        timesShown: Int,
        lastShownLaunch: Int,
        hasResponded: Boolean,
    ): Boolean {
        if (hasResponded || timesShown >= MAX_SHOWS) return false
        return launches >= maxOf(FIRST_LAUNCH, lastShownLaunch * LAUNCH_GROWTH)
    }
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
