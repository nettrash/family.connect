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

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
                .padding(32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            if (state.declined) {
                Icon(
                    imageVector = Icons.Filled.SentimentDissatisfied,
                    contentDescription = null,
                    modifier = Modifier.size(48.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(16.dp))
                Text(
                    text = "Request declined",
                    style = MaterialTheme.typography.titleLarge,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = "The family owner declined your request. " +
                        "You can try a different invite code or start your own family.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(24.dp))
                Button(onClick = onBackToGate) {
                    Text("Back")
                }
            } else {
                Icon(
                    imageVector = Icons.Filled.HourglassEmpty,
                    contentDescription = null,
                    modifier = Modifier.size(48.dp),
                    tint = MaterialTheme.colorScheme.primary,
                )
                Spacer(Modifier.height(16.dp))
                Text(
                    text = "Waiting for approval",
                    style = MaterialTheme.typography.titleLarge,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = state.familyName
                        ?.let { "Your request to join “$it” is with the family owner." }
                        ?: "Your request is with the family owner.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = "There's nothing to cancel from here — the owner " +
                        "either lets you in or declines the request.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(24.dp))
                if (state.refreshing) {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp))
                } else {
                    TextButton(onClick = viewModel::refresh) {
                        Text("Check again")
                    }
                }
            }
        }
    }
}
