package com.stablechannels.app.ui.settings.seed

import android.text.InputType
import android.view.inputmethod.EditorInfo
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.platform.InterceptPlatformTextInput
import androidx.compose.ui.platform.PlatformTextInputInterceptor
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.platform.PlatformTextInputSession
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType

/** No capitals or autocorrect; [NoLearningTextInput] then makes the variation visible-password. */
internal val SeedKeyboardOptions =
    KeyboardOptions(
        capitalization = KeyboardCapitalization.None,
        autoCorrectEnabled = false,
        keyboardType = KeyboardType.Password,
    )

/**
 * Keeps the seed out of IME suggestions and learned words: Compose 1.7 KeyboardOptions cannot set
 * IME_FLAG_NO_PERSONALIZED_LEARNING, so text fields inside [content] get it from here.
 */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
internal fun NoLearningTextInput(content: @Composable () -> Unit) {
    InterceptPlatformTextInput(NoLearningInterceptor, content)
}

@OptIn(ExperimentalComposeUiApi::class)
private object NoLearningInterceptor : PlatformTextInputInterceptor {
    override suspend fun interceptStartInputMethod(
        request: PlatformTextInputMethodRequest,
        nextHandler: PlatformTextInputSession,
    ): Nothing =
        nextHandler.startInputMethod(
            object : PlatformTextInputMethodRequest {
                override fun createInputConnection(outAttributes: EditorInfo) =
                    request.createInputConnection(outAttributes).also {
                        outAttributes.inputType =
                            (outAttributes.inputType and InputType.TYPE_MASK_VARIATION.inv()) or
                                InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                        outAttributes.imeOptions =
                            outAttributes.imeOptions or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
                    }
            }
        )
}
