/*
 * SettingsScreen.kt
 * Family Connect (Android)
 *
 * Profile block, family block (invite code + share sheet + manage entry
 * for owners), leave family (confirmed), logout (confirmed). The share
 * action goes through a plain ACTION_SEND chooser — the invite code is
 * short text, every messenger can carry it.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Settings/SettingsView.swift
 */

package me.nettrash.familyconnect.ui.settings

import android.content.Intent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Logout
import androidx.compose.material.icons.filled.Groups
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.BuildConfig
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.ErrorCard

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    onManageFamily: () -> Unit,
    onLoggedOut: () -> Unit,
    viewModel: SettingsViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var confirmLeave by remember { mutableStateOf(false) }
    var confirmLogout by remember { mutableStateOf(false) }

    LaunchedEffect(state.loggedOut) {
        if (state.loggedOut) onLoggedOut()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState()),
        ) {
            state.error?.let {
                ErrorCard(
                    message = it,
                    modifier = Modifier.padding(16.dp),
                    onRetry = viewModel::dismissError,
                )
            }

            // -- Profile ----------------------------------------------------
            ListItem(
                headlineContent = { Text(state.displayName ?: "") },
                supportingContent = { Text("@${state.username ?: ""} · ${state.serverUrl ?: ""}") },
                leadingContent = {
                    Avatar(name = state.displayName ?: "?", userId = 0L)
                },
            )
            HorizontalDivider()

            // -- Family -----------------------------------------------------
            state.familyName?.let { familyName ->
                Text(
                    text = "Family",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
                )
                ListItem(
                    headlineContent = { Text(familyName) },
                    leadingContent = { Icon(Icons.Filled.Groups, contentDescription = null) },
                )
                state.inviteCode?.let { code ->
                    ListItem(
                        headlineContent = { Text("Invite code: $code") },
                        supportingContent = { Text("Share it to invite family members") },
                        trailingContent = {
                            Icon(Icons.Filled.Share, contentDescription = "Share invite code")
                        },
                        modifier = Modifier.clickable {
                            val send = Intent(Intent.ACTION_SEND).apply {
                                type = "text/plain"
                                putExtra(
                                    Intent.EXTRA_TEXT,
                                    "Join our family on Family Connect! " +
                                        "Server: ${state.serverUrl} — invite code: $code",
                                )
                            }
                            context.startActivity(Intent.createChooser(send, "Share invite code"))
                        },
                    )
                }
                if (state.isOwner) {
                    ListItem(
                        headlineContent = { Text("Manage family") },
                        supportingContent = { Text("Join requests, members, invite code") },
                        modifier = Modifier.clickable(onClick = onManageFamily),
                    )
                }
                ListItem(
                    headlineContent = {
                        Text("Leave family", color = MaterialTheme.colorScheme.error)
                    },
                    modifier = Modifier.clickable { confirmLeave = true },
                )
                HorizontalDivider()
            }

            // -- Session ----------------------------------------------------
            Spacer(Modifier.height(8.dp))
            ListItem(
                headlineContent = {
                    Text("Log out", color = MaterialTheme.colorScheme.error)
                },
                leadingContent = {
                    Icon(
                        Icons.AutoMirrored.Filled.Logout,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                    )
                },
                modifier = Modifier.clickable { confirmLogout = true },
            )

            Spacer(Modifier.height(24.dp))
            Text(
                text = "Family Connect ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
            Spacer(Modifier.height(24.dp))
        }
    }

    if (confirmLeave) {
        AlertDialog(
            onDismissRequest = { confirmLeave = false },
            title = { Text("Leave the family?") },
            text = {
                Text(
                    "You'll lose access to the family chat and your direct " +
                        "chats. History stays on the server and comes back if " +
                        "you rejoin.",
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmLeave = false
                    viewModel.leaveFamily()
                }) {
                    Text("Leave")
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmLeave = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    if (confirmLogout) {
        AlertDialog(
            onDismissRequest = { confirmLogout = false },
            title = { Text("Log out?") },
            text = { Text("Local messages are removed from this device.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmLogout = false
                    viewModel.logout()
                }) {
                    Text("Log out")
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmLogout = false }) {
                    Text("Cancel")
                }
            },
        )
    }
}
