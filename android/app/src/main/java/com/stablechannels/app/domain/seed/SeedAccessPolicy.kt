package com.stablechannels.app.domain.seed

/**
 * The app lifecycle states seed screens react to, kept free of Android types for testing. Map
 * Lifecycle.Event.ON_PAUSE to [INACTIVE] and ON_STOP to [BACKGROUND]: the biometric prompt pauses
 * the activity without stopping it.
 */
enum class AppLifecyclePhase {
    ACTIVE,
    INACTIVE,
    BACKGROUND,
}

/**
 * When screens showing the recovery phrase close or may open. Whether opening one needs auth stays
 * in AppAccessPreferencesManager.seedPhraseRequiresAuth.
 */
object SeedAccessPolicy {
    /**
     * Leaving the app closes every seed screen. Not [AppLifecyclePhase.INACTIVE]: the biometric
     * prompt itself raises it.
     */
    fun shouldCloseSeedScreens(phase: AppLifecyclePhase): Boolean =
        phase == AppLifecyclePhase.BACKGROUND

    /** An auth reply that lands after the app left must not open a seed screen. */
    fun canOpenSeedScreen(after: AppLifecyclePhase): Boolean = !shouldCloseSeedScreens(after)
}
