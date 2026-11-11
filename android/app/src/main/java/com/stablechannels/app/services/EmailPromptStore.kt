package com.stablechannels.app.services

import android.content.Context
import android.content.SharedPreferences
import com.stablechannels.app.domain.EmailAddress
import com.stablechannels.app.domain.EmailPromptStoring
import com.stablechannels.app.domain.SavedEmailStoring

/**
 * Remembers how often the app launched, when the email prompt was shown, whether the user answered
 * it, and the last email address the server saved.
 */
class EmailPromptStore(private val prefs: SharedPreferences) :
    EmailPromptStoring, SavedEmailStoring {
    private var recordedThisProcess = false

    override val launches: Int
        get() = prefs.getInt(KEY_LAUNCHES, 0)

    override val hasResponded: Boolean
        get() = prefs.getBoolean(KEY_RESPONDED, false)

    override val timesShown: Int
        get() = prefs.getInt(KEY_TIMES_SHOWN, 0)

    override val lastShownLaunch: Int
        get() = prefs.getInt(KEY_LAST_SHOWN_LAUNCH, 0)

    override var savedEmail: EmailAddress?
        get() = prefs.getString(KEY_SAVED_EMAIL, null)?.let(EmailAddress::parse)
        set(value) {
            prefs
                .edit()
                .apply {
                    if (value == null) remove(KEY_SAVED_EMAIL)
                    else putString(KEY_SAVED_EMAIL, value.value)
                }
                .apply()
        }

    /** Counts this process once, however many times Home appears. */
    override fun recordLaunchOnce() {
        if (recordedThisProcess) return
        recordedThisProcess = true
        prefs.edit().putInt(KEY_LAUNCHES, launches + 1).apply()
    }

    override fun recordShown() {
        prefs
            .edit()
            .putInt(KEY_TIMES_SHOWN, timesShown + 1)
            .putInt(KEY_LAST_SHOWN_LAUNCH, launches)
            .apply()
    }

    /**
     * Forgets the prompt state and the saved address, for a restore that brings in a different
     * wallet.
     */
    fun reset() {
        prefs
            .edit()
            .remove(KEY_LAUNCHES)
            .remove(KEY_RESPONDED)
            .remove(KEY_TIMES_SHOWN)
            .remove(KEY_LAST_SHOWN_LAUNCH)
            .remove(KEY_SAVED_EMAIL)
            .apply()
    }

    override fun markResponded() {
        prefs.edit().putBoolean(KEY_RESPONDED, true).apply()
    }

    companion object {
        private const val PREFS_NAME = "email_prompt_prefs"
        private const val KEY_LAUNCHES = "launch_count"
        private const val KEY_RESPONDED = "responded"
        private const val KEY_TIMES_SHOWN = "times_shown"
        private const val KEY_LAST_SHOWN_LAUNCH = "last_shown_launch"
        private const val KEY_SAVED_EMAIL = "saved_email"

        @Volatile private var shared: EmailPromptStore? = null

        fun get(context: Context): EmailPromptStore =
            shared
                ?: synchronized(this) {
                    shared
                        ?: EmailPromptStore(
                                context.applicationContext.getSharedPreferences(
                                    PREFS_NAME,
                                    Context.MODE_PRIVATE,
                                )
                            )
                            .also { shared = it }
                }
    }
}
