/*
 * FamilyGateScreen.kt
 * Family Connect (Android)
 *
 * Two cards: create a family (become owner) or join one by invite code.
 * The code field uppercases as you type — codes are displayed uppercase
 * everywhere.
 *
 * iOS counterpart: ios/FamilyConnect/UI/FamilyGate/FamilyGateView.swift
 */

package me.nettrash.familyconnect.ui.familygate

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.ui.components.ErrorCard

@Composable
fun FamilyGateScreen(
    onJoined: () -> Unit,
    onPending: () -> Unit,
    viewModel: FamilyGateViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()

    LaunchedEffect(state.outcome) {
        when (state.outcome) {
            FamilyGateViewModel.Outcome.JOINED -> onJoined()
            FamilyGateViewModel.Outcome.PENDING -> onPending()
            null -> Unit
        }
    }

    Scaffold { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .imePadding()
                .padding(24.dp),
        ) {
            Text(
                text = "Your family",
                style = MaterialTheme.typography.headlineMedium,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = "Start a new family space, or join one you were invited to.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            state.generalError?.let {
                Spacer(Modifier.height(12.dp))
                ErrorCard(message = it)
            }

            Spacer(Modifier.height(24.dp))
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text("Create a family", style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(12.dp))
                    OutlinedTextField(
                        value = state.familyName,
                        onValueChange = viewModel::onFamilyNameChange,
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("Family name") },
                        singleLine = true,
                        isError = state.nameError != null,
                        supportingText = state.nameError?.let { { Text(it) } },
                    )
                    Spacer(Modifier.height(12.dp))
                    Button(
                        onClick = viewModel::createFamily,
                        enabled = !state.busy,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Create")
                    }
                }
            }

            Spacer(Modifier.height(16.dp))
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text("Join with an invite code", style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(12.dp))
                    OutlinedTextField(
                        value = state.inviteCode,
                        onValueChange = viewModel::onInviteCodeChange,
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("Invite code") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Characters,
                        ),
                        isError = state.codeError != null,
                        supportingText = state.codeError?.let { { Text(it) } },
                    )
                    Spacer(Modifier.height(12.dp))
                    Button(
                        onClick = viewModel::join,
                        enabled = !state.busy,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Join")
                    }
                }
            }
        }
    }
}
