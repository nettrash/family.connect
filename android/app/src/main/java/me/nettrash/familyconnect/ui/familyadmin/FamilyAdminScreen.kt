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

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Autorenew
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.PersonRemove
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.ErrorCard

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FamilyAdminScreen(
    onBack: () -> Unit,
    viewModel: FamilyAdminViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val members by viewModel.members.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    var confirmRotate by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf<MemberEntity?>(null) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Manage family") },
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
                ErrorCard(message = it, modifier = Modifier.padding(16.dp))
            }

            // -- Pending requests ---------------------------------------------
            Text(
                text = "Join requests",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
            )
            if (state.requests.isEmpty()) {
                Text(
                    text = "No pending requests.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            } else {
                state.requests.forEach { request ->
                    ListItem(
                        headlineContent = { Text(request.user.displayName) },
                        supportingContent = { Text("@${request.user.username}") },
                        leadingContent = {
                            Avatar(name = request.user.displayName, userId = request.user.id)
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                TextButton(
                                    onClick = { viewModel.approve(request.id) },
                                    enabled = !state.busy,
                                ) {
                                    Text("Approve")
                                }
                                IconButton(
                                    onClick = { viewModel.reject(request.id) },
                                    enabled = !state.busy,
                                ) {
                                    Icon(
                                        Icons.Filled.Close,
                                        contentDescription = "Reject",
                                        tint = MaterialTheme.colorScheme.error,
                                    )
                                }
                            }
                        },
                    )
                }
            }
            HorizontalDivider(modifier = Modifier.padding(top = 8.dp))

            // -- Invite code -----------------------------------------------------
            Text(
                text = "Invite code",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
            )
            ListItem(
                headlineContent = { Text(state.inviteCode ?: "…") },
                supportingContent = { Text("Rotating invalidates the current code immediately") },
                trailingContent = {
                    IconButton(onClick = { confirmRotate = true }, enabled = !state.busy) {
                        Icon(Icons.Filled.Autorenew, contentDescription = "Rotate invite code")
                    }
                },
            )

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
            HorizontalDivider(modifier = Modifier.padding(top = 8.dp))

            // -- Members ------------------------------------------------------------
            Text(
                text = "Members",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
            )
            members.forEach { member ->
                ListItem(
                    headlineContent = { Text(member.displayName) },
                    supportingContent = {
                        Text("@${member.username}" + if (member.role == "owner") " · owner" else "")
                    },
                    leadingContent = {
                        Avatar(name = member.displayName, userId = member.userId)
                    },
                    trailingContent = {
                        // The owner can't be removed (protocol:
                        // cannot_remove_owner) — and that's also me here.
                        if (member.role != "owner" && member.userId != myUserId) {
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
            Spacer(Modifier.height(24.dp))
        }
    }

    if (confirmRotate) {
        AlertDialog(
            onDismissRequest = { confirmRotate = false },
            title = { Text("Rotate the invite code?") },
            text = { Text("The current code stops working immediately. Pending requests survive.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmRotate = false
                    viewModel.rotateInviteCode()
                }) {
                    Text("Rotate")
                }
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
                TextButton(onClick = {
                    viewModel.removeMember(member.userId)
                    confirmRemove = null
                }) {
                    Text("Remove")
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmRemove = null }) {
                    Text("Cancel")
                }
            },
        )
    }
}
