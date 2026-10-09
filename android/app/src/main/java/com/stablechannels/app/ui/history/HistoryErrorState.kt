package com.stablechannels.app.ui.history

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.stablechannels.app.services.ConfirmationPollResult
import com.stablechannels.app.services.refreshErrorMessage

/**
 * Error banner state for History. Network (confirmation) and database (load) errors are tracked
 * separately so a successful database reload never hides a failed confirmation check. The setters
 * are private so every change goes through the rules below.
 */
internal class HistoryErrorState {
    var refreshError by mutableStateOf<String?>(null)
        private set

    var loadError by mutableStateOf<String?>(null)
        private set

    val visibleErrors: List<String>
        get() = listOfNotNull(refreshError, loadError).distinct()

    /** Every confirmation pass result replaces the banner; only a clean pass clears it. */
    fun onConfirmationResult(result: ConfirmationPollResult) {
        refreshError = result.refreshErrorMessage()
    }

    fun onRefreshFailed() {
        refreshError = "Couldn't refresh history. Pull to try again."
    }

    fun onLoadFailed(message: String) {
        loadError = message
    }

    fun onLoadSucceeded() {
        loadError = null
    }

    fun clear() {
        refreshError = null
        loadError = null
    }
}
