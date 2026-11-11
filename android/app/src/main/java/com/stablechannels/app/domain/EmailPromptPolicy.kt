package com.stablechannels.app.domain

/**
 * Decides when Home offers the email prompt: to anyone who has opened the wallet a few times,
 * funded or not. Only Save ends it; after "Not now" or a swipe the next showing waits the next gap
 * in [SHOW_GAPS] (launches 3, 12, 39), and the gaps repeat, so a 39-launch stretch never asks more
 * than three times.
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
