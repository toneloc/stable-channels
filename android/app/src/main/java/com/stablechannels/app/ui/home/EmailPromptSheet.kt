package com.stablechannels.app.ui.home

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.outlined.Email
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.stablechannels.app.R
import com.stablechannels.app.ui.components.InkButton
import com.stablechannels.app.ui.components.NeutralTextButton
import com.stablechannels.app.ui.components.inkColor
import com.stablechannels.app.ui.components.onInkColor
import com.stablechannels.app.ui.email.EmailField
import com.stablechannels.app.ui.email.EmailSaveError
import com.stablechannels.app.ui.email.EmailSettingsModel
import com.stablechannels.app.ui.theme.LocalDarkTheme
import com.stablechannels.app.ui.theme.LocalSemanticColors
import com.stablechannels.app.util.Constants
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Home prompt for the optional email. [onNotNow] answers it; [onClose] runs when it is swiped away
 * or closes after a save, so a swiped-away prompt can come back later.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EmailPromptSheet(model: EmailSettingsModel, onNotNow: () -> Unit, onClose: () -> Unit) {
    val saved = model.status == EmailSettingsModel.Status.SAVED

    LaunchedEffect(saved) {
        if (saved) {
            delay(Constants.EMAIL_PROMPT_SAVED_DISMISS_DELAY_MS)
            onClose()
        }
    }

    ModalBottomSheet(
        onDismissRequest = onClose,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = if (LocalDarkTheme.current) Color.Black else Color.White,
    ) {
        AnimatedContent(
            targetState = saved,
            transitionSpec = {
                (fadeIn() + scaleIn(initialScale = 0.92f)) togetherWith
                    (fadeOut() + scaleOut(targetScale = 0.92f))
            },
            label = "emailPromptContent",
        ) { isSaved ->
            Column(
                modifier =
                    Modifier.fillMaxWidth()
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 24.dp)
                        .padding(top = 8.dp, bottom = 24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                if (isSaved) SavedContent(model) else FormContent(model, onNotNow)
            }
        }
    }
}

@Composable
private fun FormContent(model: EmailSettingsModel, onNotNow: () -> Unit) {
    val scope = rememberCoroutineScope()
    val focusManager = LocalFocusManager.current
    val save: () -> Unit = {
        focusManager.clearFocus()
        scope.launch { model.save() }
    }
    val saving = model.status == EmailSettingsModel.Status.SAVING
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant

    HeroIcon(
        Icons.Outlined.Email,
        tint = MaterialTheme.colorScheme.primary,
        background =
            MaterialTheme.colorScheme.primary.copy(alpha = Constants.EMAIL_HERO_ICON_CIRCLE_ALPHA),
        iconSize = 28.dp,
    )
    HeroText(
        title = stringResource(R.string.email_prompt_title),
        body = stringResource(R.string.email_prompt_body),
    )
    EmailField(model, onDone = save, modifier = Modifier.fillMaxWidth())
    EmailSaveError(model.failure, modifier = Modifier.fillMaxWidth())
    InkButton(
        onClick = save,
        enabled = model.canSave,
        modifier =
            Modifier.fillMaxWidth()
                .alpha(if (model.isValid) 1f else Constants.EMAIL_DISABLED_ALPHA),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = inkColor(),
                contentColor = onInkColor(),
                disabledContainerColor = inkColor(),
                disabledContentColor = onInkColor(),
            ),
        minHeight = Constants.EMAIL_PRIMARY_BUTTON_HEIGHT_DP.dp,
        restingRadius = (Constants.EMAIL_PRIMARY_BUTTON_HEIGHT_DP / 2).dp,
    ) {
        if (saving) {
            CircularProgressIndicator(
                modifier = Modifier.size(20.dp),
                strokeWidth = 2.dp,
                color = onInkColor(),
            )
        } else {
            Text(stringResource(R.string.email_save), fontWeight = FontWeight.SemiBold)
        }
    }
    NeutralTextButton(
        onClick = onNotNow,
        modifier = Modifier.heightIn(min = Constants.MIN_TOUCH_TARGET_DP.dp),
    ) {
        Text(stringResource(R.string.email_prompt_not_now), color = secondary)
    }
    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Icon(
            Icons.Filled.Lock,
            contentDescription = null,
            tint = secondary,
            modifier = Modifier.padding(top = 2.dp).size(12.dp),
        )
        Text(
            stringResource(R.string.email_prompt_footnote),
            style = MaterialTheme.typography.bodySmall,
            color = secondary,
        )
    }
}

@Composable
private fun SavedContent(model: EmailSettingsModel) {
    val success = LocalSemanticColors.current.success
    HeroIcon(
        Icons.Filled.CheckCircle,
        tint = success,
        background = Color.Transparent,
        iconSize = Constants.EMAIL_HERO_ICON_CIRCLE_DP.dp,
    )
    HeroText(
        title = stringResource(R.string.email_prompt_saved_title),
        body = stringResource(R.string.email_prompt_saved_body, model.savedEmail?.value.orEmpty()),
    )
}

@Composable
private fun HeroIcon(icon: ImageVector, tint: Color, background: Color, iconSize: Dp) {
    Box(
        modifier =
            Modifier.size(Constants.EMAIL_HERO_ICON_CIRCLE_DP.dp)
                .background(background, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(iconSize))
    }
}

@Composable
private fun HeroText(title: String, body: String) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            title,
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center,
        )
        Text(
            body,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
