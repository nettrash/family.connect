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
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.ui.components.ErrorCard

@Composable
fun AuthScreen(
    onAuthenticated: (FamilyStatus) -> Unit,
    onChangeServer: () -> Unit,
    viewModel: AuthViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()

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
                text = "Welcome",
                style = MaterialTheme.typography.headlineMedium,
            )
            Spacer(Modifier.height(24.dp))

            SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                SegmentedButton(
                    selected = state.mode == AuthViewModel.Mode.LOGIN,
                    onClick = { viewModel.setMode(AuthViewModel.Mode.LOGIN) },
                    shape = SegmentedButtonDefaults.itemShape(index = 0, count = 2),
                ) {
                    Text("Log in")
                }
                SegmentedButton(
                    selected = state.mode == AuthViewModel.Mode.REGISTER,
                    onClick = { viewModel.setMode(AuthViewModel.Mode.REGISTER) },
                    shape = SegmentedButtonDefaults.itemShape(index = 1, count = 2),
                ) {
                    Text("Register")
                }
            }
            Spacer(Modifier.height(24.dp))

            OutlinedTextField(
                value = state.username,
                onValueChange = viewModel::onUsernameChange,
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Username") },
                singleLine = true,
                isError = state.usernameError != null,
                supportingText = state.usernameError?.let { { Text(it) } },
            )

            if (state.mode == AuthViewModel.Mode.REGISTER) {
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = state.displayName,
                    onValueChange = viewModel::onDisplayNameChange,
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text("Display name") },
                    singleLine = true,
                    isError = state.displayNameError != null,
                    supportingText = state.displayNameError?.let { { Text(it) } },
                )
            }

            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = state.password,
                onValueChange = viewModel::onPasswordChange,
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Password") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
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
                if (state.submitting) {
                    CircularProgressIndicator(modifier = Modifier.height(20.dp))
                } else {
                    Text(if (state.mode == AuthViewModel.Mode.LOGIN) "Log in" else "Create account")
                }
            }

            Spacer(Modifier.height(8.dp))
            TextButton(onClick = onChangeServer) {
                Text("Use a different server")
            }
        }
    }
}
