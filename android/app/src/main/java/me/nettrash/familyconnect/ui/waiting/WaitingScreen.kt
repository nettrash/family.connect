/*
 * WaitingScreen.kt
 * Family Connect (Android)
 *
 * "Ask the owner to let you in." Polls via the ViewModel (resume +
 * ticker + manual button). The protocol has no cancel-join-request
 * endpoint, so instead of a fake cancel button there's an honest note —
 * the owner approves or declines, that's the whole state machine.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Waiting/WaitingView.swift
 */

package me.nettrash.familyconnect.ui.waiting

import me.nettrash.familyconnect.ui.components.readableColumn
import me.nettrash.familyconnect.R
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.HourglassEmpty
import androidx.compose.material.icons.filled.SentimentDissatisfied
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle

@Composable
fun WaitingScreen(
    onApproved: () -> Unit,
    onBackToGate: () -> Unit,
    viewModel: WaitingViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()

    LaunchedEffect(state.approved) {
        if (state.approved) onApproved()
    }

    // Poll on every return to the foreground — the approval most likely
    // happened while we were away.
    LifecycleResumeEffect(Unit) {
        viewModel.refresh()
        onPauseOrDispose { }
    }

    Scaffold { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .readableColumn()
                .padding(32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            AnimatedContent(
                targetState = state.declined,
                // Fade-through: the old state clears before the new lands.
                transitionSpec = {
                    fadeIn(tween(210, delayMillis = 90)) togetherWith fadeOut(tween(120))
                },
                label = "waitingState",
            ) { declined ->
                if (declined) {
                    Column(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Icon(
                            imageVector = Icons.Filled.SentimentDissatisfied,
                            contentDescription = null,
                            modifier = Modifier.size(48.dp),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Spacer(Modifier.height(16.dp))
                        Text(
                            text = stringResource(R.string.s_request_declined),
                            style = MaterialTheme.typography.titleLarge,
                        )
                        Spacer(Modifier.height(8.dp))
                        Text(
                            text = stringResource(R.string.s_request_declined_explanation),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(Modifier.height(24.dp))
                        Button(onClick = onBackToGate) {
                            Text(stringResource(R.string.s_back))
                        }
                    }
                } else {
                    Column(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        // Gentle periodic flip: hold, then a 400ms half-turn.
                        // HourglassEmpty is 180°-symmetric, so the snap back
                        // to 0f between cycles is invisible.
                        val flip = rememberInfiniteTransition(label = "hourglassFlip")
                        val flipAngle by flip.animateFloat(
                            initialValue = 0f,
                            targetValue = 180f,
                            animationSpec = infiniteRepeatable(
                                keyframes {
                                    durationMillis = 2400
                                    0f at 0
                                    0f at 2000 using FastOutSlowInEasing
                                    180f at 2400
                                },
                            ),
                            label = "hourglassAngle",
                        )
                        Icon(
                            imageVector = Icons.Filled.HourglassEmpty,
                            contentDescription = null,
                            modifier = Modifier
                                .size(48.dp)
                                .graphicsLayer { rotationZ = flipAngle },
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Spacer(Modifier.height(16.dp))
                        Text(
                            text = stringResource(R.string.s_waiting_for_approval),
                            style = MaterialTheme.typography.titleLarge,
                        )
                        Spacer(Modifier.height(8.dp))
                        Text(
                            text = state.familyName
                                ?.let { stringResource(R.string.s_your_request_to_join_is_with_the_family_owner, it) }
                                ?: stringResource(R.string.s_your_request_is_with_the_family_owner),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(Modifier.height(8.dp))
                        Text(
                            text = stringResource(R.string.s_waiting_explanation),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                        Spacer(Modifier.height(24.dp))
                        // Fixed-height slot so button↔spinner never shifts
                        // the text above it.
                        Box(
                            modifier = Modifier.height(48.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            AnimatedContent(
                                targetState = state.refreshing,
                                transitionSpec = {
                                    fadeIn(tween(150)) togetherWith fadeOut(tween(150))
                                },
                                label = "checkAgain",
                            ) { refreshing ->
                                if (refreshing) {
                                    CircularProgressIndicator(modifier = Modifier.size(24.dp))
                                } else {
                                    TextButton(onClick = viewModel::refresh) {
                                        Text(stringResource(R.string.s_check_again))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
