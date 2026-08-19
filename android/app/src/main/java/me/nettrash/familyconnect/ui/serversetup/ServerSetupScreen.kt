/*
 * ServerSetupScreen.kt
 * Family Connect (Android)
 *
 * First-run screen: type the family's server address. The probe is an
 * unauthenticated GET /me — a live Family Connect server answers 401
 * WITH the protocol error body, so that exact response is the success
 * signal (2xx would actually be suspicious). A failed probe still
 * offers "Save anyway" — the server might simply be off right now.
 *
 * http:// URLs work (self-hosted LAN boxes) but show a persistent
 * warning card: the network-security-config allows cleartext, the UX
 * makes sure it's a choice, not an accident.
 *
 * iOS counterpart: ios/FamilyConnect/UI/ServerSetup/ServerSetupView.swift
 */

package me.nettrash.familyconnect.ui.serversetup

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
import androidx.compose.material.icons.filled.Home
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle

@Composable
fun ServerSetupScreen(
    onSaved: () -> Unit,
    viewModel: ServerSetupViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()

    LaunchedEffect(state.saved) {
        if (state.saved) onSaved()
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
            Icon(
                imageVector = Icons.Filled.Home,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(bottom = 8.dp),
            )
            Text(
                text = "Family Connect",
                style = MaterialTheme.typography.headlineMedium,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = "Where does your family's server live?",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(24.dp))

            OutlinedTextField(
                value = state.url,
                onValueChange = viewModel::onUrlChange,
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Server address") },
                placeholder = { Text("chat.example.com or http://192.168.1.10:8080") },
                singleLine = true,
                isError = state.error != null,
                supportingText = state.error?.let { { Text(it) } },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            )

            if (state.isCleartext) {
                Spacer(Modifier.height(12.dp))
                Card(
                    modifier = Modifier.fillMaxWidth(),
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.secondaryContainer,
                    ),
                ) {
                    Text(
                        text = "This address uses plain http — messages travel " +
                            "unencrypted on the network. Fine on a trusted home " +
                            "LAN, risky anywhere else.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                        modifier = Modifier.padding(12.dp),
                    )
                }
            }

            Spacer(Modifier.height(24.dp))
            Button(
                onClick = viewModel::probeAndSave,
                enabled = !state.probing && state.url.isNotBlank(),
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (state.probing) {
                    CircularProgressIndicator(modifier = Modifier.height(20.dp))
                } else {
                    Text("Connect")
                }
            }

            if (state.showSaveAnyway) {
                Spacer(Modifier.height(8.dp))
                TextButton(onClick = viewModel::saveAnyway) {
                    Text("Save anyway — the server may be offline right now")
                }
            }
        }
    }
}
