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

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
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
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.ui.components.BusyButtonContent
import me.nettrash.familyconnect.ui.components.ErrorCard

/** Which card's action is in flight — the VM has one shared busy flag. */
private enum class GateAction { CREATE, JOIN }

@Composable
fun FamilyGateScreen(
    onJoined: () -> Unit,
    onPending: () -> Unit,
    viewModel: FamilyGateViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    // Pure UI state: remember which button was tapped so only that one
    // shows its spinner while the shared busy flag is up.
    var busyAction by remember { mutableStateOf<GateAction?>(null) }

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
                text = stringResource(R.string.s_your_family),
                style = MaterialTheme.typography.headlineMedium,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.s_start_a_new_family_space_or_join_one_you_were_invited_to),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            // Last message is remembered so the exit animation still has
            // content to draw while shrinking (same trick as OfflineBanner).
            val lastError = remember { mutableStateOf("") }
            state.generalError?.let { lastError.value = it }
            AnimatedVisibility(
                visible = state.generalError != null,
                enter = expandVertically() + fadeIn(),
                exit = shrinkVertically() + fadeOut(),
            ) {
                Column {
                    Spacer(Modifier.height(12.dp))
                    ErrorCard(message = lastError.value)
                }
            }

            Spacer(Modifier.height(24.dp))
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(stringResource(R.string.s_create_a_family), style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(12.dp))
                    OutlinedTextField(
                        value = state.familyName,
                        onValueChange = viewModel::onFamilyNameChange,
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text(stringResource(R.string.s_family_name)) },
                        singleLine = true,
                        isError = state.nameError != null,
                        supportingText = state.nameError?.let { { Text(it) } },
                    )
                    Spacer(Modifier.height(12.dp))
                    Button(
                        onClick = {
                            busyAction = GateAction.CREATE
                            viewModel.createFamily()
                        },
                        enabled = !state.busy,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        BusyButtonContent(
                            label = stringResource(R.string.s_create),
                            busy = state.busy && busyAction == GateAction.CREATE,
                        )
                    }
                }
            }

            Spacer(Modifier.height(16.dp))
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(stringResource(R.string.s_join_with_an_invite_code), style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(12.dp))
                    OutlinedTextField(
                        value = state.inviteCode,
                        onValueChange = viewModel::onInviteCodeChange,
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text(stringResource(R.string.s_invite_code)) },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Characters,
                        ),
                        isError = state.codeError != null,
                        supportingText = state.codeError?.let { { Text(it) } },
                    )
                    Spacer(Modifier.height(12.dp))
                    // Tonal: creating is the primary path, joining the
                    // secondary one — same actions, quieter emphasis.
                    FilledTonalButton(
                        onClick = {
                            busyAction = GateAction.JOIN
                            viewModel.join()
                        },
                        enabled = !state.busy,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        BusyButtonContent(
                            label = stringResource(R.string.s_join),
                            busy = state.busy && busyAction == GateAction.JOIN,
                        )
                    }
                }
            }
        }
    }
}
