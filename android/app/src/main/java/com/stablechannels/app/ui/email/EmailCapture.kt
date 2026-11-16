package com.stablechannels.app.ui.email

import androidx.annotation.StringRes
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.outlined.Email
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.stablechannels.app.R
import com.stablechannels.app.domain.EmailSaveFailure
import com.stablechannels.app.services.EmailPromptStore
import com.stablechannels.app.services.EmailSubmissionService
import com.stablechannels.app.services.NodeService
import com.stablechannels.app.services.NodeServiceSigner
import com.stablechannels.app.ui.theme.LocalSemanticColors
import com.stablechannels.app.util.Constants

/** Builds the model with the running node as signer; saving also answers the Home prompt. */
@Composable
fun rememberEmailSettingsModel(nodeService: NodeService): EmailSettingsModel {
    val store = EmailPromptStore.get(LocalContext.current)
    return remember(nodeService, store) {
        EmailSettingsModel(
            service = EmailSubmissionService(NodeServiceSigner(nodeService), store),
            savedStore = store,
            onSaved = store::markResponded,
        )
    }
}

/** The email input with its envelope, valid checkmark, focus border and inline validation. */
@Composable
fun EmailField(model: EmailSettingsModel, onDone: () -> Unit, modifier: Modifier = Modifier) {
    var focused by remember { mutableStateOf(false) }
    val shape = RoundedCornerShape(Constants.EMAIL_FIELD_CORNER_RADIUS_DP.dp)
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant

    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        BasicTextField(
            value = model.input,
            onValueChange = model::onInputChange,
            modifier =
                Modifier.fillMaxWidth().onFocusChanged {
                    focused = it.isFocused
                    model.onFocusChange(it.isFocused)
                },
            singleLine = true,
            textStyle =
                MaterialTheme.typography.bodyLarge.copy(
                    color = MaterialTheme.colorScheme.onSurface
                ),
            cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
            keyboardOptions =
                KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Email,
                    imeAction = ImeAction.Done,
                ),
            keyboardActions = KeyboardActions(onDone = { onDone() }),
            decorationBox = { innerTextField ->
                Row(
                    modifier =
                        Modifier.fillMaxWidth()
                            .height(Constants.EMAIL_FIELD_HEIGHT_DP.dp)
                            .background(MaterialTheme.colorScheme.surfaceVariant, shape)
                            .then(
                                if (focused) {
                                    Modifier.border(
                                        Constants.EMAIL_FIELD_FOCUS_BORDER_DP.dp,
                                        MaterialTheme.colorScheme.primary,
                                        shape,
                                    )
                                } else {
                                    Modifier
                                }
                            )
                            .padding(horizontal = 14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Icon(
                        Icons.Outlined.Email,
                        contentDescription = null,
                        tint = secondary,
                        modifier = Modifier.size(20.dp),
                    )
                    Box(Modifier.weight(1f)) {
                        if (model.input.isEmpty()) {
                            Text(
                                stringResource(R.string.email_field_placeholder),
                                style = MaterialTheme.typography.bodyLarge,
                                color = secondary.copy(alpha = 0.6f),
                            )
                        }
                        innerTextField()
                    }
                    if (model.isValid) {
                        Icon(
                            Icons.Filled.CheckCircle,
                            contentDescription = null,
                            tint = LocalSemanticColors.current.success,
                            modifier = Modifier.size(20.dp),
                        )
                    }
                }
            },
        )
        if (model.showsInvalidHint) {
            Text(
                stringResource(R.string.email_field_invalid),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
}

/** The line of copy shown for this failure. */
val EmailSaveFailure.message: Int
    @StringRes
    get() =
        when (this) {
            EmailSaveFailure.WALLET_NOT_READY -> R.string.email_error_wallet_not_ready
            EmailSaveFailure.NETWORK -> R.string.email_error_network
            EmailSaveFailure.REJECTED -> R.string.email_error_rejected
            EmailSaveFailure.SIGNATURE_REFUSED -> R.string.email_error_check_clock
            EmailSaveFailure.UNAVAILABLE -> R.string.email_error_unavailable
        }

/** One red line explaining why the last save failed; nothing when it did not. */
@Composable
fun EmailSaveError(failure: EmailSaveFailure?, modifier: Modifier = Modifier) {
    if (failure == null) return
    Text(
        stringResource(failure.message),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.error,
        modifier = modifier,
    )
}
