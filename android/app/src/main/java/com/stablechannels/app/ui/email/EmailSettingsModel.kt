package com.stablechannels.app.ui.email

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.stablechannels.app.domain.EmailAddress
import com.stablechannels.app.domain.EmailSaveException
import com.stablechannels.app.domain.EmailSaveFailure
import com.stablechannels.app.domain.SavedEmailStoring
import com.stablechannels.app.services.EmailSubmitter
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Form state for the optional email, shared by the Home prompt and Settings > Email. */
class EmailSettingsModel(
    private val service: EmailSubmitter,
    private val savedStore: SavedEmailStoring,
    private val onSaved: () -> Unit = {},
) {
    enum class Status {
        IDLE,
        SAVING,
        SAVED,
        FAILED,
    }

    var status by mutableStateOf(Status.IDLE)
        private set

    var failure by mutableStateOf<EmailSaveFailure?>(null)
        private set

    var input by mutableStateOf("")
        private set

    var savedEmail by mutableStateOf(savedStore.savedEmail)
        private set

    /** True while Settings shows the field to replace a saved address. */
    var isChanging by mutableStateOf(false)
        private set

    private var validationRequested by mutableStateOf(false)
    private var hadFocus = false

    val isValid: Boolean
        get() = EmailAddress.parse(input) != null

    val canSave: Boolean
        get() = status != Status.SAVING && isValid

    val showsInvalidHint: Boolean
        get() = validationRequested && input.isNotBlank() && !isValid

    val showsField: Boolean
        get() = savedEmail == null || isChanging

    fun onInputChange(value: String) {
        input = value
        if (status != Status.SAVING) clearResult()
    }

    fun onFocusChange(focused: Boolean) {
        if (focused) hadFocus = true else if (hadFocus) validationRequested = true
    }

    fun startChange() {
        input = savedEmail?.value.orEmpty()
        isChanging = true
        validationRequested = false
        clearResult()
    }

    suspend fun save() {
        validationRequested = true
        val email = EmailAddress.parse(input) ?: return
        if (status != Status.SAVING) submit(email)
    }

    suspend fun remove() {
        if (status != Status.SAVING) submit(null)
    }

    private suspend fun submit(email: EmailAddress?) {
        status = Status.SAVING
        failure = null
        // A request in flight still reaches the server, so finish it and record the result even if
        // the sheet closes; callTimeout bounds the wait.
        withContext(NonCancellable) {
            try {
                service.submit(email)
            } catch (e: EmailSaveException) {
                failure = e.failure
                status = Status.FAILED
                return@withContext
            }
            savedStore.savedEmail = email
            savedEmail = email
            isChanging = false
            if (email == null) {
                input = ""
                status = Status.IDLE
            } else {
                status = Status.SAVED
                onSaved()
            }
        }
    }

    private fun clearResult() {
        status = Status.IDLE
        failure = null
    }
}
