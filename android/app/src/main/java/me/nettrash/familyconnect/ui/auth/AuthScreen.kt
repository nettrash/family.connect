/*
 * AuthScreen.kt
 * Family Connect (Android)
 *
 * Login / Register with a segmented toggle. Field errors render inline
 * under their field (409 username_taken lands on the username field);
 * transport errors get an ErrorCard.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Auth/AuthView.swift
 */

package me.nettrash.familyconnect.ui.auth

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.ui.components.BusyButtonContent
import me.nettrash.familyconnect.ui.components.ErrorCard

@Composable
fun AuthScreen(
    onAuthenticated: (FamilyStatus) -> Unit,
    onChangeServer: () -> Unit,
    viewModel: AuthViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val serverUrl by viewModel.serverUrl.collectAsStateWithLifecycle()
    var passwordVisible by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(state.authenticatedStatus) {
        state.authenticatedStatus?.let(onAuthenticated)
    }

    Scaffold { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .imePadding()
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Text(
                text = stringResource(R.string.s_welcome),
                style = MaterialTheme.typography.headlineMedium,
            )
            Spacer(Modifier.height(24.dp))

            SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                SegmentedButton(
                    selected = state.mode == AuthViewModel.Mode.LOGIN,
                    onClick = { viewModel.setMode(AuthViewModel.Mode.LOGIN) },
                    shape = SegmentedButtonDefaults.itemShape(index = 0, count = 2),
                ) {
                    Text(stringResource(R.string.s_log_in))
                }
                SegmentedButton(
                    selected = state.mode == AuthViewModel.Mode.REGISTER,
                    onClick = { viewModel.setMode(AuthViewModel.Mode.REGISTER) },
                    shape = SegmentedButtonDefaults.itemShape(index = 1, count = 2),
                ) {
                    Text(stringResource(R.string.s_register))
                }
            }
            Spacer(Modifier.height(24.dp))

            OutlinedTextField(
                value = state.username,
                onValueChange = viewModel::onUsernameChange,
                modifier = Modifier
                    .fillMaxWidth()
                    .semantics { contentType = ContentType.Username },
                label = { Text(stringResource(R.string.s_username)) },
                singleLine = true,
                isError = state.usernameError != null,
                supportingText = state.usernameError?.let { { Text(it) } },
            )

            AnimatedVisibility(
                visible = state.mode == AuthViewModel.Mode.REGISTER,
                enter = expandVertically(spring(stiffness = Spring.StiffnessMediumLow)) + fadeIn(),
                exit = shrinkVertically() + fadeOut(),
            ) {
                // Spacer lives inside so the collapsed state leaves no gap.
                Column {
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = state.displayName,
                        onValueChange = viewModel::onDisplayNameChange,
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text(stringResource(R.string.s_display_name)) },
                        singleLine = true,
                        isError = state.displayNameError != null,
                        supportingText = state.displayNameError?.let { { Text(it) } },
                    )
                }
            }

            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = state.password,
                onValueChange = viewModel::onPasswordChange,
                modifier = Modifier
                    .fillMaxWidth()
                    .semantics {
                        contentType =
                            if (state.mode == AuthViewModel.Mode.REGISTER) ContentType.NewPassword
                            else ContentType.Password
                    },
                label = { Text(stringResource(R.string.s_password)) },
                singleLine = true,
                visualTransformation =
                    if (passwordVisible) VisualTransformation.None
                    else PasswordVisualTransformation(),
                trailingIcon = {
                    IconButton(onClick = { passwordVisible = !passwordVisible }) {
                        Icon(
                            imageVector =
                                if (passwordVisible) Icons.Outlined.VisibilityOff
                                else Icons.Outlined.Visibility,
                            contentDescription =
                                if (passwordVisible) "Hide password" else "Show password",
                        )
                    }
                },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                isError = state.passwordError != null,
                supportingText = state.passwordError?.let { { Text(it) } },
            )

            state.generalError?.let {
                Spacer(Modifier.height(12.dp))
                ErrorCard(message = it)
            }

            Spacer(Modifier.height(24.dp))
            Button(
                onClick = viewModel::submit,
                enabled = !state.submitting,
                modifier = Modifier.fillMaxWidth(),
            ) {
                BusyButtonContent(
                    label = if (state.mode == AuthViewModel.Mode.LOGIN) stringResource(R.string.s_log_in) else "Create account",
                    busy = state.submitting,
                )
            }

            Spacer(Modifier.height(8.dp))
            TextButton(onClick = onChangeServer) {
                Text(stringResource(R.string.s_use_a_different_server))
            }
            Text(
                // Host only — the scheme is noise here. Rendered
                // unconditionally so the line height is reserved from the
                // first frame: the VM's serverUrl flow seeds null, and a
                // conditional caption would pop in a frame late and shift
                // the centered form. AUTH is unreachable without a
                // configured server, so it is never permanently blank.
                text = serverUrl.orEmpty()
                    .removePrefix("https://")
                    .removePrefix("http://")
                    .trimEnd('/'),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}
