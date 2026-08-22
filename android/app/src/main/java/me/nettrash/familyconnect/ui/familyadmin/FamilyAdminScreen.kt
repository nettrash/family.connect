/*
 * FamilyAdminScreen.kt
 * Family Connect (Android)
 *
 * Owner console: pending requests with approve/reject, invite-code
 * rotation (confirmed — the old code dies immediately), join-policy
 * toggle, member list with remove (confirmed; the owner row has no
 * remove — the protocol forbids removing the owner).
 *
 * iOS counterpart: ios/FamilyConnect/UI/FamilyAdmin/FamilyAdminView.swift
 */

package me.nettrash.familyconnect.ui.familyadmin

import android.content.ClipData
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Autorenew
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.PersonRemove
import androidx.compose.material.icons.outlined.Inbox
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.toClipEntry
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.DestructiveTextButton
import me.nettrash.familyconnect.ui.components.EmptyState
import me.nettrash.familyconnect.ui.components.ErrorCard

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FamilyAdminScreen(
    onBack: () -> Unit,
    viewModel: FamilyAdminViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val members by viewModel.members.collectAsStateWithLifecycle()
    val isOwner by viewModel.isOwner.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }
    val scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior()
    var confirmRotate by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf<MemberEntity?>(null) }

    // Rows hide-and-shrink the moment an approve/reject/remove is fired,
    // so the departure animates instead of snapping when the server's
    // reload drops the row from the list.
    val departingRequests = remember { mutableStateListOf<Long>() }
    val departingMembers = remember { mutableStateListOf<Long>() }
    // A failed mutation leaves its row in the list; once the mutation
    // settles, un-hide everything so nothing stays invisibly shrunk.
    LaunchedEffect(state.busy) {
        if (!state.busy) {
            departingRequests.clear()
            departingMembers.clear()
        }
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            TopAppBar(
                title = { Text(if (isOwner) "Manage family" else "Family members") },
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
            state.error?.let {
                ErrorCard(message = it, modifier = Modifier.padding(16.dp))
            }

            // Owner-only: join requests, the invite code and the join
            // policy are all owner endpoints on the server. A plain
            // member opens this screen for the roster below.
            if (isOwner) {
                // -- Pending requests ---------------------------------------------
                Text(
                    text = "Join requests",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
                )
                if (state.requests.isEmpty()) {
                    EmptyState(
                        icon = Icons.Outlined.Inbox,
                        title = "No pending requests",
                        subtitle = "Share the invite code below to add someone.",
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    state.requests.forEach { request ->
                        key(request.id) {
                            AnimatedVisibility(
                                visible = request.id !in departingRequests,
                                enter = expandVertically(tween(200)) + fadeIn(tween(200)),
                                exit = shrinkVertically(tween(200)) + fadeOut(tween(200)),
                            ) {
                                ListItem(
                                    headlineContent = { Text(request.user.displayName) },
                                    supportingContent = { Text("@${request.user.username}") },
                                    leadingContent = {
                                        // Initials on purpose: a pending
                                        // requester is not in the family
                                        // yet, so the server answers 404
                                        // for their picture — asking
                                        // would only cache them as
                                        // pictureless past approval.
                                        Avatar(
                                            name = request.user.displayName,
                                            userId = request.user.id,
                                        )
                                    },
                                    trailingContent = {
                                        Row(
                                            verticalAlignment = Alignment.CenterVertically,
                                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                                        ) {
                                            FilledTonalIconButton(
                                                onClick = {
                                                    departingRequests += request.id
                                                    viewModel.approve(request.id) {
                                                        scope.launch {
                                                            snackbarHostState.showSnackbar(
                                                                "Approved ${request.user.displayName}",
                                                            )
                                                        }
                                                    }
                                                },
                                                enabled = !state.busy,
                                                modifier = Modifier.size(40.dp),
                                                colors = IconButtonDefaults.filledTonalIconButtonColors(
                                                    containerColor = MaterialTheme.colorScheme.primaryContainer,
                                                    contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                                                ),
                                            ) {
                                                Icon(
                                                    Icons.Filled.Check,
                                                    contentDescription = "Approve ${request.user.displayName}",
                                                )
                                            }
                                            IconButton(
                                                onClick = {
                                                    departingRequests += request.id
                                                    viewModel.reject(request.id) {
                                                        scope.launch {
                                                            snackbarHostState.showSnackbar(
                                                                "Rejected ${request.user.displayName}",
                                                            )
                                                        }
                                                    }
                                                },
                                                enabled = !state.busy,
                                                modifier = Modifier.size(40.dp),
                                            ) {
                                                Icon(
                                                    Icons.Filled.Close,
                                                    contentDescription = "Reject ${request.user.displayName}",
                                                    tint = MaterialTheme.colorScheme.error,
                                                )
                                            }
                                        }
                                    },
                                )
                            }
                        }
                    }
                }
                SectionDivider()

                // -- Invite code -----------------------------------------------------
                Text(
                    text = "Invite code",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
                )
                Surface(
                    shape = RoundedCornerShape(12.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp),
                ) {
                    Row(
                        modifier = Modifier.padding(start = 16.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = state.inviteCode ?: "…",
                            style = MaterialTheme.typography.titleLarge.copy(
                                fontFamily = FontFamily.Monospace,
                                letterSpacing = 2.sp,
                            ),
                            modifier = Modifier.weight(1f),
                        )
                        IconButton(
                            onClick = {
                                val code = state.inviteCode ?: return@IconButton
                                scope.launch {
                                    clipboard.setClipEntry(
                                        ClipData.newPlainText("Invite code", code).toClipEntry(),
                                    )
                                    snackbarHostState.showSnackbar("Copied")
                                }
                            },
                            enabled = state.inviteCode != null,
                        ) {
                            Icon(Icons.Filled.ContentCopy, contentDescription = "Copy invite code")
                        }
                        IconButton(onClick = { confirmRotate = true }, enabled = !state.busy) {
                            Icon(Icons.Filled.Autorenew, contentDescription = "Rotate invite code")
                        }
                    }
                }
                Text(
                    text = "Rotating invalidates the current code immediately",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
                SectionDivider()

                // -- Join policy ------------------------------------------------------
                Text(
                    text = "Join policy",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 8.dp),
                )
                SingleChoiceSegmentedButtonRow(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp),
                ) {
                    SegmentedButton(
                        selected = state.joinPolicy == "open",
                        onClick = { viewModel.setJoinPolicy("open") },
                        shape = SegmentedButtonDefaults.itemShape(index = 0, count = 2),
                        enabled = !state.busy,
                    ) {
                        Text("Open")
                    }
                    SegmentedButton(
                        selected = state.joinPolicy == "approval",
                        onClick = { viewModel.setJoinPolicy("approval") },
                        shape = SegmentedButtonDefaults.itemShape(index = 1, count = 2),
                        enabled = !state.busy,
                    ) {
                        Text("Approval")
                    }
                }
                Text(
                    text = if (state.joinPolicy == "open") {
                        "Anyone with the invite code joins instantly."
                    } else {
                        "Joins with the code wait for your approval."
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
                SectionDivider()
            }

            // -- Members ------------------------------------------------------------
            Text(
                text = "Members",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
            )
            members.forEach { member ->
                key(member.userId) {
                    AnimatedVisibility(
                        visible = member.userId !in departingMembers,
                        enter = expandVertically(tween(200)) + fadeIn(tween(200)),
                        exit = shrinkVertically(tween(200)) + fadeOut(tween(200)),
                    ) {
                        ListItem(
                            headlineContent = { Text(member.displayName) },
                            supportingContent = {
                                Text("@${member.username}" + if (member.role == "owner") " · owner" else "")
                            },
                            leadingContent = {
                                Avatar(
                                    name = member.displayName,
                                    userId = member.userId,
                                    avatarVersion = member.avatarVersion,
                                )
                            },
                            trailingContent = {
                                // Removing is an owner action. The owner
                                // can't be removed (protocol:
                                // cannot_remove_owner) — and that's also me here.
                                if (isOwner && member.role != "owner" && member.userId != myUserId) {
                                    IconButton(
                                        onClick = { confirmRemove = member },
                                        enabled = !state.busy,
                                    ) {
                                        Icon(
                                            Icons.Filled.PersonRemove,
                                            contentDescription = "Remove ${member.displayName}",
                                            tint = MaterialTheme.colorScheme.error,
                                        )
                                    }
                                }
                            },
                        )
                    }
                }
            }
            SectionDivider()
            Spacer(Modifier.height(24.dp))
        }
    }

    if (confirmRotate) {
        AlertDialog(
            onDismissRequest = { confirmRotate = false },
            title = { Text("Rotate the invite code?") },
            text = { Text("The current code stops working immediately. Pending requests survive.") },
            confirmButton = {
                DestructiveTextButton(
                    label = "Rotate",
                    onClick = {
                        confirmRotate = false
                        viewModel.rotateInviteCode()
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmRotate = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    confirmRemove?.let { member ->
        AlertDialog(
            onDismissRequest = { confirmRemove = null },
            title = { Text("Remove ${member.displayName}?") },
            text = { Text("They lose access to the family chats. History stays and returns if they rejoin.") },
            confirmButton = {
                DestructiveTextButton(
                    label = "Remove",
                    onClick = {
                        departingMembers += member.userId
                        viewModel.removeMember(member.userId) {
                            scope.launch {
                                snackbarHostState.showSnackbar("Removed ${member.displayName}")
                            }
                        }
                        confirmRemove = null
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmRemove = null }) {
                    Text("Cancel")
                }
            },
        )
    }
}

@Composable
private fun SectionDivider() {
    HorizontalDivider(
        modifier = Modifier.padding(start = 16.dp),
        color = MaterialTheme.colorScheme.outlineVariant,
    )
}
