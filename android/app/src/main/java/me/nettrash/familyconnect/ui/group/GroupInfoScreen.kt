/*
 * GroupInfoScreen.kt
 * Family Connect (Android)
 *
 * A group's info (#107; docs/protocol.md, "Groups"): the name, "Members:
 * N", the members with "Admin" beside the admin, and the actions —
 * Rename Group, Add Members, Remove from Group and Delete Group for the
 * admin, Leave Group for everybody. Opened from the group's top bar, on a
 * phone and inside the tablet's detail pane alike.
 *
 * When the group goes — left or deleted here, or a `chat_removed` from
 * elsewhere — [onGroupGone] takes the reader out past the chat itself:
 * there is no conversation left to go back to.
 */

package me.nettrash.familyconnect.ui.group

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ExitToApp
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.DestructiveTextButton
import me.nettrash.familyconnect.ui.components.ErrorCard
import me.nettrash.familyconnect.util.GroupRules

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GroupInfoScreen(
    onBack: () -> Unit,
    /** The group left this device: leave past the chat, which is gone too. */
    onGroupGone: () -> Unit,
    viewModel: GroupInfoViewModel = hiltViewModel(),
) {
    val chat by viewModel.chat.collectAsStateWithLifecycle()
    val me by viewModel.myUserId.collectAsStateWithLifecycle()
    val members by viewModel.members.collectAsStateWithLifecycle()
    val addable by viewModel.addable.collectAsStateWithLifecycle()
    val busy by viewModel.busy.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()

    var renaming by rememberSaveable { mutableStateOf(false) }
    var adding by rememberSaveable { mutableStateOf(false) }
    var confirmLeave by rememberSaveable { mutableStateOf(false) }
    var confirmDelete by rememberSaveable { mutableStateOf(false) }

    // Non-null → null, never plain null: the flow starts with null while
    // Room answers (ChatScreen's rule for the same thing).
    val currentOnGone by rememberUpdatedState(onGroupGone)
    var chatWasSeen by remember { mutableStateOf(false) }
    LaunchedEffect(chat != null) {
        if (chat != null) chatWasSeen = true else if (chatWasSeen) currentOnGone()
    }

    val group = chat
    val kind = group?.kind
    val memberIds = group?.memberIds.orEmpty()
    val adminId = group?.adminUserId
    fun may(action: GroupRules.Action, target: Long? = null) =
        GroupRules.may(action, kind, memberIds, adminId, me, target)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.s_group_info)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.s_back))
                    }
                },
            )
        },
    ) { padding ->
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
            contentPadding = PaddingValues(bottom = 24.dp),
        ) {
            item(key = "header") {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 24.dp, vertical = 20.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    GroupAvatarMark(size = 96)
                    Spacer(Modifier.height(12.dp))
                    Text(
                        text = group?.title.orEmpty(),
                        style = MaterialTheme.typography.headlineSmall,
                        textAlign = TextAlign.Center,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = stringResource(R.string.s_group_members_count, memberIds.size),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            item(key = "error") {
                AnimatedVisibility(visible = error != null) {
                    ErrorCard(
                        message = error.orEmpty(),
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                        onRetry = viewModel::dismissError,
                    )
                }
            }
            if (may(GroupRules.Action.RENAME)) {
                item(key = "rename") {
                    ActionRow(
                        label = stringResource(R.string.s_rename_group),
                        icon = { Icon(Icons.Outlined.Edit, contentDescription = null) },
                        enabled = !busy,
                        onClick = { renaming = true },
                    )
                }
            }
            if (may(GroupRules.Action.ADD_MEMBERS)) {
                item(key = "add") {
                    ActionRow(
                        label = stringResource(R.string.s_add_members),
                        icon = { Icon(Icons.Outlined.PersonAdd, contentDescription = null) },
                        enabled = !busy,
                        onClick = { adding = true },
                    )
                }
            }
            item(key = "members-divider") { HorizontalDivider(Modifier.padding(vertical = 8.dp)) }
            items(members, key = { "member-${it.userId}" }) { member ->
                MemberRow(
                    member = member,
                    removable = may(GroupRules.Action.REMOVE_MEMBER, member.userId),
                    enabled = !busy,
                    onRemove = { viewModel.removeMember(member.userId) },
                )
            }
            item(key = "actions-divider") { HorizontalDivider(Modifier.padding(vertical = 8.dp)) }
            if (may(GroupRules.Action.LEAVE)) {
                item(key = "leave") {
                    ActionRow(
                        label = stringResource(R.string.s_leave_group),
                        icon = { Icon(Icons.AutoMirrored.Filled.ExitToApp, contentDescription = null) },
                        enabled = !busy,
                        destructive = true,
                        onClick = { confirmLeave = true },
                    )
                }
            }
            if (may(GroupRules.Action.DELETE)) {
                item(key = "delete") {
                    ActionRow(
                        label = stringResource(R.string.s_delete_group),
                        icon = { Icon(Icons.Outlined.Delete, contentDescription = null) },
                        enabled = !busy,
                        destructive = true,
                        onClick = { confirmDelete = true },
                    )
                }
            }
        }
    }

    if (renaming && group != null) {
        RenameDialog(
            current = group.title,
            busy = busy,
            onDismiss = { renaming = false },
            onSave = { name -> viewModel.rename(name) { renaming = false } },
        )
    }
    if (adding) {
        AddMembersDialog(
            candidates = addable,
            busy = busy,
            onDismiss = { adding = false },
            onAdd = { ids -> viewModel.addMembers(ids) { adding = false } },
        )
    }
    if (confirmLeave && group != null) {
        AlertDialog(
            onDismissRequest = { confirmLeave = false },
            title = { Text(stringResource(R.string.s_leave_group)) },
            text = { Text(stringResource(R.string.s_leave_group_confirm, group.title)) },
            confirmButton = {
                DestructiveTextButton(
                    label = stringResource(R.string.s_leave),
                    onClick = {
                        confirmLeave = false
                        viewModel.leave()
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmLeave = false }) { Text(stringResource(R.string.s_cancel)) }
            },
        )
    }
    if (confirmDelete && group != null) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(stringResource(R.string.s_delete_group)) },
            text = { Text(stringResource(R.string.s_delete_group_confirm, group.title)) },
            confirmButton = {
                DestructiveTextButton(
                    label = stringResource(R.string.s_delete),
                    onClick = {
                        confirmDelete = false
                        viewModel.delete()
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) { Text(stringResource(R.string.s_cancel)) }
            },
        )
    }
}

@Composable
private fun ActionRow(
    label: String,
    icon: @Composable () -> Unit,
    enabled: Boolean,
    onClick: () -> Unit,
    destructive: Boolean = false,
) {
    val color = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
    ListItem(
        headlineContent = { Text(label) },
        leadingContent = icon,
        colors = ListItemDefaults.colors(
            headlineColor = color,
            leadingIconColor = if (destructive) color else MaterialTheme.colorScheme.onSurfaceVariant,
        ),
        // Not clickable mid-change: a second tap is not a second request.
        modifier = Modifier.clickable(enabled = enabled, onClick = onClick),
    )
}

@Composable
private fun MemberRow(
    member: GroupMemberRow,
    removable: Boolean,
    enabled: Boolean,
    onRemove: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    ListItem(
        headlineContent = {
            Text(
                text = if (member.isMe) "${member.name} (${stringResource(R.string.s_you)})" else member.name,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        supportingContent = member.username?.let { { Text("@$it") } },
        leadingContent = {
            Avatar(
                name = member.name,
                userId = member.userId,
                size = 40,
                avatarVersion = member.avatarVersion,
            )
        },
        trailingContent = {
            Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.Center) {
                if (member.isAdmin) {
                    Text(
                        text = stringResource(R.string.s_group_admin),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
                if (removable) {
                    Box {
                        IconButton(onClick = { menuOpen = true }, enabled = enabled) {
                            Icon(Icons.Filled.MoreVert, contentDescription = stringResource(R.string.s_remove_from_group))
                        }
                        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.s_remove_from_group)) },
                                onClick = {
                                    menuOpen = false
                                    onRemove()
                                },
                            )
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun RenameDialog(
    current: String,
    busy: Boolean,
    onDismiss: () -> Unit,
    onSave: (String) -> Unit,
) {
    var name by rememberSaveable { mutableStateOf(current) }
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text(stringResource(R.string.s_rename_group)) },
        text = {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                label = { Text(stringResource(R.string.s_group_name)) },
                singleLine = true,
                enabled = !busy,
                isError = GroupRules.validName(name) == null,
                supportingText = if (GroupRules.validName(name) == null) {
                    { Text(stringResource(R.string.e_group_name_length)) }
                } else {
                    null
                },
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                modifier = Modifier.fillMaxWidth(),
            )
        },
        confirmButton = {
            TextButton(
                onClick = { onSave(name) },
                enabled = !busy && GroupRules.validName(name) != null,
            ) { Text(stringResource(R.string.s_save)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, enabled = !busy) { Text(stringResource(R.string.s_cancel)) }
        },
    )
}

@Composable
private fun AddMembersDialog(
    candidates: List<MemberEntity>,
    busy: Boolean,
    onDismiss: () -> Unit,
    onAdd: (Set<Long>) -> Unit,
) {
    var selected by rememberSaveable { mutableStateOf(emptyList<Long>()) }
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text(stringResource(R.string.s_add_members)) },
        text = {
            if (candidates.isEmpty()) {
                Text(stringResource(R.string.s_everyone_already_in_group))
            } else {
                Column(modifier = Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState())) {
                    GroupMemberPicker(
                        candidates = candidates,
                        selected = selected.toSet(),
                        onToggle = { id -> selected = if (id in selected) selected - id else selected + id },
                        enabled = !busy,
                    )
                }
            }
        },
        confirmButton = {
            if (candidates.isNotEmpty()) {
                TextButton(
                    onClick = { onAdd(selected.toSet()) },
                    enabled = !busy && selected.isNotEmpty(),
                ) { Text(stringResource(R.string.s_add_members)) }
            } else {
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_done)) }
            }
        },
        dismissButton = if (candidates.isNotEmpty()) {
            { TextButton(onClick = onDismiss, enabled = !busy) { Text(stringResource(R.string.s_cancel)) } }
        } else {
            null
        },
    )
}
