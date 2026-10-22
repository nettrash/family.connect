/*
 * GroupComponents.kt
 * Family Connect (Android)
 *
 * The pieces of a group (#107; docs/protocol.md, "Groups") that more than
 * one screen draws: its picture in a list, the member picker the New Group
 * form and Add Members share, and the New Group form itself, which lives in
 * the chat list's "New chat" sheet.
 */

package me.nettrash.familyconnect.ui.group

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.util.resolvedDisplayName

/**
 * A group's picture: the "people" glyph in a disc. Nobody's colour and
 * nobody's face, like the family chat's mark — a group is several people —
 * and named "Group" to TalkBack, since the title beside it is the group's
 * own name and says nothing about what kind of chat it is.
 */
@Composable
fun GroupAvatarMark(size: Int = 48) {
    Box(
        modifier = Modifier
            .size(size.dp)
            .background(MaterialTheme.colorScheme.tertiaryContainer, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = Icons.Outlined.Groups,
            contentDescription = stringResource(R.string.s_group),
            // The glyph keeps its proportion of the disc at every size.
            modifier = Modifier.size((size * 26 / 48).dp),
            tint = MaterialTheme.colorScheme.onTertiaryContainer,
        )
    }
}

/**
 * Family members to tick, each a whole-row toggle — the New Group form's
 * list and Add Members' share it, so the two cannot drift.
 */
@Composable
fun GroupMemberPicker(
    candidates: List<MemberEntity>,
    selected: Set<Long>,
    onToggle: (Long) -> Unit,
    enabled: Boolean = true,
) {
    val context = LocalContext.current
    Column {
        candidates.forEach { member ->
            val checked = member.userId in selected
            ListItem(
                headlineContent = { Text(member.resolvedDisplayName(context)) },
                supportingContent = { Text("@${member.username}") },
                leadingContent = {
                    Avatar(
                        name = member.displayName,
                        userId = member.userId,
                        size = 40,
                        avatarVersion = member.avatarVersion,
                    )
                },
                // Null: the row's toggleable is the control, so TalkBack
                // reads one checkbox per member rather than two.
                trailingContent = { Checkbox(checked = checked, onCheckedChange = null, enabled = enabled) },
                // Whatever it sits on — the sheet, or the Add Members dialog.
                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                modifier = Modifier.toggleable(
                    value = checked,
                    enabled = enabled,
                    role = Role.Checkbox,
                    onValueChange = { onToggle(member.userId) },
                ),
            )
        }
    }
}

/**
 * New Group: a name and the members to add (docs/protocol.md, "Groups").
 *
 * Stateless — the sheet holds the name and the ticks, so a rotation keeps
 * them, and the ViewModel checks both before anything is sent.
 */
@Composable
fun NewGroupForm(
    name: String,
    onNameChange: (String) -> Unit,
    candidates: List<MemberEntity>,
    selected: Set<Long>,
    onToggle: (Long) -> Unit,
    error: String?,
    busy: Boolean,
    onCancel: () -> Unit,
    onCreate: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(
            text = stringResource(R.string.s_new_group),
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 16.dp),
        )
        OutlinedTextField(
            value = name,
            onValueChange = onNameChange,
            label = { Text(stringResource(R.string.s_group_name)) },
            singleLine = true,
            enabled = !busy,
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.Sentences,
                imeAction = ImeAction.Done,
            ),
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp),
        )
        Spacer(Modifier.height(8.dp))
        if (candidates.isEmpty()) {
            Text(
                text = stringResource(R.string.s_no_other_family_members_yet_share_the_invite_code_first),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 16.dp),
            )
        } else {
            GroupMemberPicker(candidates = candidates, selected = selected, onToggle = onToggle, enabled = !busy)
        }
        error?.let {
            Text(
                text = it,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.End,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TextButton(onClick = onCancel, enabled = !busy) {
                Text(stringResource(R.string.s_cancel))
            }
            Spacer(Modifier.size(8.dp))
            Button(onClick = onCreate, enabled = !busy && candidates.isNotEmpty()) {
                if (busy) {
                    CircularProgressIndicator(modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                } else {
                    Text(stringResource(R.string.s_create_group))
                }
            }
        }
    }
}
