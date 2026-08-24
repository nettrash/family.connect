/*
 * SettingsScreen.kt
 * Family Connect (Android)
 *
 * Profile block (picture + name), family block (invite code + share sheet
 * + manage entry for owners), leave family (confirmed), logout
 * (confirmed). The share action goes through a plain ACTION_SEND chooser
 * — the invite code is short text, every messenger can carry it.
 *
 * The photo picker is PickVisualMedia: the system picker, so the app
 * never asks for a storage permission and only ever sees the one image
 * the user chose. Everything after the pick — read, downscale, crop,
 * upload — belongs to the ViewModel (see AvatarSource for why the read
 * must not stay here).
 *
 * iOS counterpart: ios/FamilyConnect/UI/Settings/SettingsView.swift
 */

package me.nettrash.familyconnect.ui.settings

import android.content.ClipData
import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.material.icons.outlined.BarChart
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.AddAPhoto
import androidx.compose.material.icons.outlined.ManageAccounts
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material.icons.filled.Key
import androidx.compose.material3.ListItem
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.ui.familyadmin.SetPasswordDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
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
    onOpenStatistics: () -> Unit,
    onLoggedOut: () -> Unit,
    viewModel: SettingsViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val shareTitle = stringResource(R.string.s_share_invite_code)
    val inviteLabel = stringResource(R.string.s_invite_code)
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }
    val scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior()
    var confirmLeave by remember { mutableStateOf(false) }
    var confirmLogout by remember { mutableStateOf(false) }
    var changingPassword by remember { mutableStateOf(false) }
    /// Held here rather than inside the dialog: the dialog is rebuilt on
    /// every keystroke of the other fields, and the current password must
    /// survive that.
    var currentPassword by remember { mutableStateOf("") }
    // The Uri goes straight to the ViewModel: reading a cloud-backed
    // photo can take seconds, and a read owned by this composable would
    // die — with its busy flag stuck on — the moment the user backs out.
    val pickPhoto = rememberLauncherForActivityResult(
        ActivityResultContracts.PickVisualMedia(),
    ) { uri -> uri?.let(viewModel::setAvatar) }

    LaunchedEffect(state.loggedOut) {
        if (state.loggedOut) onLoggedOut()
    }

    // A failed photo is transient and self-explanatory — a snackbar, not
    // the persistent ErrorCard the family actions use.
    LaunchedEffect(state.avatarError) {
        state.avatarError?.let {
            snackbarHostState.showSnackbar(it)
            viewModel.dismissAvatarError()
        }
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.s_settings)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.s_back))
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
                    // Keyed to the session user id so the avatar hue (and
                    // now the picture) matches this user everywhere else.
                    Avatar(
                        name = state.displayName ?: "?",
                        userId = state.userId ?: 0L,
                        size = 56,
                        avatarVersion = state.avatarVersion,
                    )
                },
                trailingContent = {
                    if (state.uploadingAvatar) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(20.dp),
                            strokeWidth = 2.dp,
                        )
                    }
                },
            )
            ListItem(
                headlineContent = {
                    Text(if (state.avatarVersion > 0) "Change photo" else "Add photo")
                },
                leadingContent = {
                    Icon(
                        Icons.Outlined.AddAPhoto,
                        contentDescription = null,
                        modifier = Modifier.size(24.dp),
                    )
                },
                modifier = Modifier.clickable(enabled = !state.uploadingAvatar) {
                    pickPhoto.launch(
                        PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly),
                    )
                },
            )
            if (state.avatarVersion > 0) {
                ListItem(
                    headlineContent = {
                        Text(stringResource(R.string.s_remove_photo), color = MaterialTheme.colorScheme.error)
                    },
                    leadingContent = {
                        Icon(
                            Icons.Outlined.Delete,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.error,
                            modifier = Modifier.size(24.dp),
                        )
                    },
                    modifier = Modifier.clickable(enabled = !state.uploadingAvatar) {
                        viewModel.removeAvatar()
                    },
                )
            }
            HorizontalDivider(
                modifier = Modifier.padding(start = 16.dp),
                color = MaterialTheme.colorScheme.outlineVariant,
            )

            // -- Family -----------------------------------------------------
            state.familyName?.let { familyName ->
                Text(
                    text = stringResource(R.string.s_family),
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
                        context.startActivity(Intent.createChooser(send, shareTitle))
                    }
                    ListItem(
                        overlineContent = { Text(stringResource(R.string.s_invite_code)) },
                        headlineContent = {
                            Text(
                                text = code,
                                fontFamily = FontFamily.Monospace,
                                letterSpacing = 1.sp,
                            )
                        },
                        supportingContent = { Text(stringResource(R.string.s_share_it_to_invite_family_members)) },
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
                                            ClipData.newPlainText(inviteLabel, code)
                                                .toClipEntry(),
                                        )
                                        snackbarHostState.showSnackbar("Copied")
                                    }
                                }) {
                                    Icon(
                                        Icons.Outlined.ContentCopy,
                                        contentDescription = stringResource(R.string.s_copy_invite_code),
                                    )
                                }
                                IconButton(onClick = shareCode) {
                                    Icon(
                                        Icons.Outlined.Share,
                                        contentDescription = stringResource(R.string.s_share_invite_code),
                                    )
                                }
                            }
                        },
                        modifier = Modifier.clickable(onClick = shareCode),
                    )
                }
                // Everyone gets in: the roster is not owner-gated on the
                // server, only the invite code, policy, requests and
                // removal are — and the screen hides those for members.
                run {
                    ListItem(
                        headlineContent = {
                            Text(if (state.isOwner) "Manage family" else "Family members")
                        },
                        supportingContent = {
                            Text(
                                if (state.isOwner) {
                                    "Join requests, members, invite code"
                                } else {
                                    "See who is in the family"
                                },
                            )
                        },
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
                // Every member, not just the owner (protocol.md, "Family
                // statistics") — so it sits outside the owner-only block.
                ListItem(
                    headlineContent = { Text(stringResource(R.string.s_statistics)) },
                    leadingContent = {
                        Icon(
                            Icons.Outlined.BarChart,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    },
                    trailingContent = {
                        Icon(
                            Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    },
                    modifier = Modifier.clickable(onClick = onOpenStatistics),
                )
                ListItem(
                    headlineContent = {
                        Text(stringResource(R.string.s_leave_family), color = MaterialTheme.colorScheme.error)
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

            // -- Privacy ----------------------------------------------------
            // The one setting that changes who the app talks to, so it
            // says so plainly rather than hiding behind a label.
            Text(
                text = stringResource(R.string.s_privacy),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
            )
            ListItem(
                headlineContent = { Text(stringResource(R.string.s_link_previews)) },
                supportingContent = {
                    Text(
                        stringResource(R.string.s_link_previews_explanation),
                    )
                },
                trailingContent = {
                    Switch(
                        checked = state.linkPreviewsEnabled,
                        onCheckedChange = viewModel::setLinkPreviewsEnabled,
                    )
                },
                modifier = Modifier.clickable {
                    viewModel.setLinkPreviewsEnabled(!state.linkPreviewsEnabled)
                },
            )
            ListItem(
                headlineContent = { Text(stringResource(R.string.s_map_previews)) },
                supportingContent = {
                    Text(
                        stringResource(R.string.s_map_previews_explanation),
                    )
                },
                trailingContent = {
                    Switch(
                        checked = state.mapPreviewsEnabled,
                        onCheckedChange = viewModel::setMapPreviewsEnabled,
                    )
                },
                modifier = Modifier.clickable {
                    viewModel.setMapPreviewsEnabled(!state.mapPreviewsEnabled)
                },
            )
            HorizontalDivider(
                modifier = Modifier.padding(start = 16.dp),
                color = MaterialTheme.colorScheme.outlineVariant,
            )

            // -- Session ----------------------------------------------------
            Spacer(Modifier.height(8.dp))
            ListItem(
                headlineContent = { Text(stringResource(R.string.s_change_password)) },
                supportingContent = {
                    Text(stringResource(R.string.s_your_other_devices_will_be_signed_out_this_one_stays_signe))
                },
                leadingContent = { Icon(Icons.Filled.Key, contentDescription = null) },
                modifier = Modifier.clickable { changingPassword = true },
            )
            HorizontalDivider(
                modifier = Modifier.padding(start = 16.dp),
                color = MaterialTheme.colorScheme.outlineVariant,
            )
            ListItem(
                headlineContent = {
                    Text(stringResource(R.string.s_log_out), color = MaterialTheme.colorScheme.error)
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
                // The app's own display name, not a second copy of it —
                // the launcher says "Family" and this said "Family Connect".
                text = "${stringResource(R.string.app_name)} " +
                    "${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})",
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
            title = { Text(stringResource(R.string.s_leave_the_family)) },
            text = {
                Text(
                    stringResource(R.string.s_leave_family_explanation),
                )
            },
            confirmButton = {
                DestructiveTextButton(
                    label = stringResource(R.string.s_leave),
                    onClick = {
                        confirmLeave = false
                        viewModel.leaveFamily()
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmLeave = false }) {
                    Text(stringResource(R.string.s_cancel))
                }
            },
        )
    }

    if (changingPassword) {
        SetPasswordDialog(
            title = stringResource(R.string.s_change_password),
            explanation = stringResource(R.string.s_your_other_devices_will_be_signed_out_this_one_stays_signe),
            confirmLabel = stringResource(R.string.s_save),
            busy = state.busy,
            currentPassword = currentPassword,
            onCurrentPasswordChange = { currentPassword = it },
            onDismiss = {
                changingPassword = false
                currentPassword = ""
            },
            onConfirm = { password ->
                viewModel.changePassword(currentPassword, password) {
                    scope.launch { snackbarHostState.showSnackbar("Password changed") }
                }
                changingPassword = false
                currentPassword = ""
            },
        )
    }

    if (confirmLogout) {
        AlertDialog(
            onDismissRequest = { confirmLogout = false },
            title = { Text(stringResource(R.string.s_log_out_2)) },
            text = { Text(stringResource(R.string.s_local_messages_are_removed_from_this_device)) },
            confirmButton = {
                DestructiveTextButton(
                    label = stringResource(R.string.s_log_out),
                    onClick = {
                        confirmLogout = false
                        viewModel.logout()
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmLogout = false }) {
                    Text(stringResource(R.string.s_cancel))
                }
            },
        )
    }
}
