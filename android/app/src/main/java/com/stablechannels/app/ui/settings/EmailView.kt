package com.stablechannels.app.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.stablechannels.app.R
import com.stablechannels.app.ui.email.EmailField
import com.stablechannels.app.ui.email.EmailSaveError
import com.stablechannels.app.ui.email.EmailSettingsModel
import com.stablechannels.app.ui.theme.LocalSemanticColors
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.launch

@Composable
fun EmailView(model: EmailSettingsModel) {
    val scope = rememberCoroutineScope()
    var confirmRemove by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (model.showsField) {
            EntrySections(model)
        } else {
            SavedSections(model, onRemove = { confirmRemove = true })
        }
    }

    if (confirmRemove) {
        RemoveEmailDialog(
            onConfirm = {
                confirmRemove = false
                scope.launch { model.remove() }
            },
            onDismiss = { confirmRemove = false },
        )
    }
}

@Composable
private fun EntrySections(model: EmailSettingsModel) {
    val scope = rememberCoroutineScope()
    val focusManager = LocalFocusManager.current
    val save: () -> Unit = {
        focusManager.clearFocus()
        scope.launch { model.save() }
    }

    EmailField(model, onDone = save, modifier = Modifier.fillMaxWidth())
    SectionFooter(stringResource(R.string.email_settings_footer))
    SectionCard {
        ActionRow(
            label = stringResource(R.string.email_save),
            color = MaterialTheme.colorScheme.primary,
            enabled = model.canSave,
            busy = model.status == EmailSettingsModel.Status.SAVING,
            onClick = save,
        )
    }
    EmailSaveError(model.failure, modifier = Modifier.padding(horizontal = 16.dp))
}

@Composable
private fun SavedSections(model: EmailSettingsModel, onRemove: () -> Unit) {
    val success = LocalSemanticColors.current.success

    SectionHeader(stringResource(R.string.email_settings_saved_header))
    SectionCard {
        Row(
            modifier =
                Modifier.fillMaxWidth()
                    .heightIn(min = Constants.MIN_TOUCH_TARGET_DP.dp)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                model.savedEmail?.value.orEmpty(),
                style = MaterialTheme.typography.bodyLarge,
                modifier = Modifier.weight(1f),
            )
            Icon(
                Icons.Filled.CheckCircle,
                contentDescription = null,
                tint = success,
                modifier = Modifier.size(16.dp),
            )
            Text(
                stringResource(R.string.email_settings_saved_label),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
                color = success,
            )
        }
    }
    SectionCard(modifier = Modifier.padding(top = 16.dp)) {
        val enabled = model.status != EmailSettingsModel.Status.SAVING
        ActionRow(
            label = stringResource(R.string.email_settings_change),
            color = MaterialTheme.colorScheme.primary,
            enabled = enabled,
            busy = false,
            onClick = model::startChange,
        )
        HorizontalDivider(
            modifier = Modifier.padding(start = 16.dp),
            color = MaterialTheme.colorScheme.outlineVariant,
        )
        ActionRow(
            label = stringResource(R.string.email_settings_remove),
            color = MaterialTheme.colorScheme.error,
            enabled = enabled,
            busy = !enabled,
            onClick = onRemove,
        )
    }
    EmailSaveError(model.failure, modifier = Modifier.padding(horizontal = 16.dp))
    SectionFooter(stringResource(R.string.email_settings_footer))
}

@Composable
private fun SectionCard(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Surface(
        shape = MaterialTheme.shapes.medium,
        tonalElevation = 1.dp,
        modifier = modifier.fillMaxWidth(),
    ) {
        Column { content() }
    }
}

@Composable
private fun SectionHeader(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 16.dp),
    )
}

@Composable
private fun SectionFooter(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 16.dp).padding(bottom = 16.dp),
    )
}

@Composable
private fun ActionRow(
    label: String,
    color: Color,
    enabled: Boolean,
    busy: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier.fillMaxWidth()
                .heightIn(min = Constants.MIN_TOUCH_TARGET_DP.dp)
                .clickable(enabled = enabled, onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            label,
            style = MaterialTheme.typography.bodyLarge,
            color =
                if (enabled || busy) color
                else
                    MaterialTheme.colorScheme.onSurface.copy(
                        alpha = Constants.EMAIL_DISABLED_ALPHA
                    ),
            modifier = Modifier.weight(1f),
        )
        if (busy) {
            CircularProgressIndicator(
                modifier = Modifier.size(18.dp),
                strokeWidth = 2.dp,
                color = color,
            )
        }
    }
}

@Composable
private fun RemoveEmailDialog(onConfirm: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        shape = RoundedCornerShape(20.dp),
        title = {
            Text(
                stringResource(R.string.email_remove_confirm_title),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
        },
        text = {
            Text(
                stringResource(R.string.email_remove_confirm_message),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(
                    stringResource(R.string.email_remove_confirm_remove),
                    color = MaterialTheme.colorScheme.error,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(
                    stringResource(R.string.email_remove_confirm_cancel),
                    color = MaterialTheme.colorScheme.onSurface,
                )
            }
        },
    )
}
