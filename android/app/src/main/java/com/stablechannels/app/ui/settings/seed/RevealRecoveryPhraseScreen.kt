package com.stablechannels.app.ui.settings.seed

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedContentTransitionScope.SlideDirection
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBackIosNew
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.domain.seed.QuizNavigationDirection
import com.stablechannels.app.domain.seed.RevealQuizQuestion as QuizQuestion
import com.stablechannels.app.domain.seed.RevealQuizStep
import com.stablechannels.app.domain.seed.RevealRecoveryQuiz

private val StepAnimation = tween<Float>(250, easing = EaseInOut)

/** Forward steps enter from the trailing edge, back steps from the leading edge. */
internal fun quizSlideDirection(direction: QuizNavigationDirection): SlideDirection =
    when (direction) {
        QuizNavigationDirection.FORWARD -> SlideDirection.Start
        QuizNavigationDirection.BACKWARD -> SlideDirection.End
    }

/**
 * The reveal flow: intro, two questions with feedback, then the phrase. State lives only in
 * composition (never saved), so every open starts at the intro.
 */
@Composable
fun RevealRecoveryPhraseScreen(
    mnemonicProvider: () -> String?,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var quiz by remember { mutableStateOf(RevealRecoveryQuiz()) }
    var showLearnMore by remember { mutableStateOf(false) }
    val back: () -> Unit = { if (quiz.canGoBack) quiz = quiz.goBack() else onClose() }
    val learnMore = { showLearnMore = true }
    BackHandler(onBack = back)

    Column(modifier.fillMaxSize()) {
        RevealTopBar(canGoBack = quiz.canGoBack, onBack = back)
        AnimatedContent(
            targetState = quiz,
            modifier = Modifier.weight(1f),
            transitionSpec = {
                // Removal stays a fade, as on iOS.
                (fadeIn(StepAnimation) +
                    slideIntoContainer(
                        quizSlideDirection(targetState.direction),
                        tween(250, easing = EaseInOut),
                    )) togetherWith fadeOut(StepAnimation)
            },
            contentKey = { it.step },
            label = "revealQuizStep",
        ) { state ->
            when (val step = state.step) {
                RevealQuizStep.Intro ->
                    RevealQuizIntro(
                        onGetStarted = { quiz = quiz.startQuiz() },
                        onLearnMore = learnMore,
                    )
                is RevealQuizStep.Question ->
                    QuizQuestion.all.getOrNull(step.index)?.let {
                        RevealQuizQuestion(
                            question = it,
                            progress = state.progressText(step.index),
                            onSelectOption = { option -> quiz = quiz.selectOption(option) },
                            onLearnMore = learnMore,
                        )
                    }
                is RevealQuizStep.Feedback ->
                    QuizQuestion.all.getOrNull(step.questionIndex)?.let {
                        RevealQuizFeedback(
                            question = it,
                            progress = state.progressText(step.questionIndex),
                            isCorrect = step.isCorrect,
                            onContinue = { quiz = quiz.continueFromFeedback() },
                            onLearnMore = learnMore,
                        )
                    }
                RevealQuizStep.Revealed -> {
                    val phrase = remember { state.mnemonic(mnemonicProvider) }
                    if (phrase != null) {
                        RevealQuizSecretPhrase(mnemonic = phrase, onDone = onClose)
                    } else {
                        // The phrase was removed during the quiz: close rather than show an empty
                        // screen.
                        LaunchedEffect(Unit) { onClose() }
                    }
                }
            }
        }
    }

    if (showLearnMore) RevealQuizLearnMoreSheet(onDismiss = { showLearnMore = false })
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RevealTopBar(canGoBack: Boolean, onBack: () -> Unit) {
    CenterAlignedTopAppBar(
        title = {
            Text(
                "Reveal Secret Recovery Phrase",
                fontSize = 17.sp,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        navigationIcon = {
            val isRtl = LocalLayoutDirection.current == LayoutDirection.Rtl
            IconButton(onClick = onBack) {
                Icon(
                    Icons.Filled.ArrowBackIosNew,
                    contentDescription = if (canGoBack) "Back" else "Close",
                    // No auto-mirrored ArrowBackIosNew in this icon set, so flip it for RTL here.
                    modifier =
                        Modifier.size(18.dp).scale(scaleX = if (isRtl) -1f else 1f, scaleY = 1f),
                )
            }
        },
        // The dialog already pads for the system bars.
        windowInsets = WindowInsets(0),
        colors = TopAppBarDefaults.centerAlignedTopAppBarColors(containerColor = Color.Transparent),
    )
}
