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

import android.content.ClipData
import android.content.Intent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.ExitToApp
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.ManageAccounts
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.toClipEntry
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.BuildConfig
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.DestructiveTextButton
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
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }
    val scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior()
    var confirmLeave by remember { mutableStateOf(false) }
    var confirmLogout by remember { mutableStateOf(false) }

    LaunchedEffect(state.loggedOut) {
        if (state.loggedOut) onLoggedOut()
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                scrollBehavior = scrollBehavior,
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState()),
        ) {
            // The exit animation still needs text to draw while the card
            // shrinks, so keep the last non-null message around.
            var lastError by remember { mutableStateOf("") }
            state.error?.let { lastError = it }
            AnimatedVisibility(
                visible = state.error != null,
                enter = expandVertically() + fadeIn(),
                exit = shrinkVertically() + fadeOut(),
            ) {
                ErrorCard(
                    message = lastError,
                    modifier = Modifier.padding(16.dp),
                    onRetry = viewModel::dismissError,
                )
            }

            // -- Profile ----------------------------------------------------
            ListItem(
                headlineContent = { Text(state.displayName ?: "") },
                supportingContent = {
                    Text(
                        text = "@${state.username ?: ""} · ${state.serverUrl ?: ""}",
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                leadingContent = {
                    // Keyed to the session user id so the avatar hue matches
                    // this user's color everywhere else in the app.
                    Avatar(name = state.displayName ?: "?", userId = state.userId ?: 0L)
                },
            )
            HorizontalDivider(
                modifier = Modifier.padding(start = 16.dp),
                color = MaterialTheme.colorScheme.outlineVariant,
            )

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
                    leadingContent = {
                        Icon(
                            Icons.Outlined.Groups,
                            contentDescription = null,
                            modifier = Modifier.size(24.dp),
                        )
                    },
                )
                state.inviteCode?.let { code ->
                    val shareCode = {
                        val send = Intent(Intent.ACTION_SEND).apply {
                            type = "text/plain"
                            putExtra(
                                Intent.EXTRA_TEXT,
                                "Join our family on Family Connect! " +
                                    "Server: ${state.serverUrl} — invite code: $code",
                            )
                        }
                        context.startActivity(Intent.createChooser(send, "Share invite code"))
                    }
                    ListItem(
                        overlineContent = { Text("Invite code") },
                        headlineContent = {
                            Text(
                                text = code,
                                fontFamily = FontFamily.Monospace,
                                letterSpacing = 1.sp,
                            )
                        },
                        supportingContent = { Text("Share it to invite family members") },
                        leadingContent = {
                            Icon(
                                Icons.Outlined.Key,
                                contentDescription = null,
                                modifier = Modifier.size(24.dp),
                            )
                        },
                        trailingContent = {
                            Row {
                                IconButton(onClick = {
                                    // setClipEntry is suspend, hence the scope.
                                    scope.launch {
                                        clipboard.setClipEntry(
                                            ClipData.newPlainText("Invite code", code)
                                                .toClipEntry(),
                                        )
                                        snackbarHostState.showSnackbar("Copied")
                                    }
                                }) {
                                    Icon(
                                        Icons.Outlined.ContentCopy,
                                        contentDescription = "Copy invite code",
                                    )
                                }
                                IconButton(onClick = shareCode) {
                                    Icon(
                                        Icons.Outlined.Share,
                                        contentDescription = "Share invite code",
                                    )
                                }
                            }
                        },
                        modifier = Modifier.clickable(onClick = shareCode),
                    )
                }
                if (state.isOwner) {
                    ListItem(
                        headlineContent = { Text("Manage family") },
                        supportingContent = { Text("Join requests, members, invite code") },
                        leadingContent = {
                            Icon(
                                Icons.Outlined.ManageAccounts,
                                contentDescription = null,
                                modifier = Modifier.size(24.dp),
                            )
                        },
                        trailingContent = {
                            // No AutoMirrored ChevronRight exists in the icon
                            // set; KeyboardArrowRight is its mirrored twin.
                            Icon(
                                Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        modifier = Modifier.clickable(onClick = onManageFamily),
                    )
                }
                ListItem(
                    headlineContent = {
                        Text("Leave family", color = MaterialTheme.colorScheme.error)
                    },
                    leadingContent = {
                        Icon(
                            Icons.AutoMirrored.Outlined.ExitToApp,
                            contentDescription = null,
                            modifier = Modifier.size(24.dp),
                            tint = MaterialTheme.colorScheme.error,
                        )
                    },
                    modifier = Modifier.clickable { confirmLeave = true },
                )
                HorizontalDivider(
                    modifier = Modifier.padding(start = 16.dp),
                    color = MaterialTheme.colorScheme.outlineVariant,
                )
            }

            // -- Session ----------------------------------------------------
            Spacer(Modifier.height(8.dp))
            ListItem(
                headlineContent = {
                    Text("Log out", color = MaterialTheme.colorScheme.error)
                },
                leadingContent = {
                    Icon(
                        Icons.AutoMirrored.Outlined.ExitToApp,
                        contentDescription = null,
                        modifier = Modifier.size(24.dp),
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
                DestructiveTextButton(
                    label = "Leave",
                    onClick = {
                        confirmLeave = false
                        viewModel.leaveFamily()
                    },
                )
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
                DestructiveTextButton(
                    label = "Log out",
                    onClick = {
                        confirmLogout = false
                        viewModel.logout()
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmLogout = false }) {
                    Text("Cancel")
                }
            },
        )
    }
}
