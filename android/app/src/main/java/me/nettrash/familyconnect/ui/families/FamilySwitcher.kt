/*
 * FamilySwitcher.kt
 * Family Connect (Android)
 *
 * The switcher between families (#88; docs/protocol.md, "Several families"
 * — "Switching"): on the phone the family's name at the top of the chat
 * list opens a sheet listing every family with its unread count, the
 * pending join requests ("Waiting for approval"), the invitations to answer,
 * and the two doors to another family — "New Family…" and "Join Another
 * Family…". The invitation banner over the chat list is here too.
 *
 * Shown only when there is something to switch to or answer
 * ([FamiliesUi.showsSwitcher]): an account in ONE family with nothing
 * pending sees none of it.
 */

package me.nettrash.familyconnect.ui.families

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.GroupAdd
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material.icons.outlined.HourglassEmpty
import androidx.compose.material.icons.outlined.Mail
import androidx.compose.material3.Badge
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.InvitationDto
import me.nettrash.familyconnect.data.net.dto.PendingJoinRequestDto

/** What the switcher and the invitation banner draw — ChatListViewModel builds it from settings and the chats. */
data class FamiliesUi(
    val families: List<FamilyDto> = emptyList(),
    val currentFamilyId: Long? = null,
    /** Each family's unread: its listed chats', a direct chat counted in each family listing it. */
    val unread: Map<Long, Int> = emptyMap(),
    val pending: List<PendingJoinRequestDto> = emptyList(),
    val invitations: List<InvitationDto> = emptyList(),
    /** Two families or more, or something pending to show; otherwise nothing changes on screen. */
    val showsSwitcher: Boolean = false,
    val canStartAnother: Boolean = false,
    val canJoinAnother: Boolean = false,
) {
    /** The family shown, by name, for the title the switcher hangs off. */
    val currentName: String? get() = families.firstOrNull { it.id == currentFamilyId }?.name
}

/**
 * The chat list's title when there is a switcher: the current family's
 * name, which opens it. Its button label is "Switch Family".
 */
@Composable
fun FamilySwitcherTitle(name: String, onClick: () -> Unit) {
    val label = stringResource(R.string.s_switch_family)
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(8.dp))
            .clickable(onClickLabel = label, role = Role.Button, onClick = onClick)
            .padding(horizontal = 4.dp, vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = name,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f, fill = false),
        )
        Icon(Icons.Filled.ArrowDropDown, contentDescription = label)
    }
}

/**
 * The switcher itself: every family (name, unread, the current one ticked),
 * every pending join request, every invitation, and the doors to another
 * family when this account may open them.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FamilySwitcherSheet(
    state: FamiliesUi,
    /** The invitation whose answer is in flight, if any. */
    answeringInvitation: Long?,
    onPick: (Long) -> Unit,
    onAccept: (Long) -> Unit,
    onDecline: (Long) -> Unit,
    onNewFamily: () -> Unit,
    onJoinFamily: () -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Column(modifier = Modifier.verticalScroll(rememberScrollState())) {
            SectionTitle(stringResource(R.string.s_families))
            state.families.forEach { family ->
                FamilyRow(
                    family = family,
                    unread = state.unread[family.id] ?: 0,
                    current = family.id == state.currentFamilyId,
                    onClick = { onPick(family.id) },
                )
            }
            state.pending.forEach { request ->
                ListItem(
                    headlineContent = { Text(request.familyName) },
                    supportingContent = { Text(stringResource(R.string.s_waiting_for_approval)) },
                    leadingContent = {
                        Icon(Icons.Outlined.HourglassEmpty, contentDescription = null)
                    },
                    colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
                )
            }
            if (state.invitations.isNotEmpty()) {
                SectionTitle(stringResource(R.string.s_invitations))
                state.invitations.forEach { invitation ->
                    InvitationContent(
                        invitation = invitation,
                        busy = answeringInvitation != null,
                        onJoin = { onAccept(invitation.id) },
                        onDecline = { onDecline(invitation.id) },
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }
            if (state.canStartAnother || state.canJoinAnother) {
                HorizontalDivider(
                    modifier = Modifier.padding(vertical = 8.dp),
                    color = MaterialTheme.colorScheme.outlineVariant,
                )
            }
            if (state.canStartAnother) {
                ActionRow(
                    label = stringResource(R.string.s_new_family),
                    icon = { Icon(Icons.Outlined.Add, contentDescription = null) },
                    onClick = onNewFamily,
                )
            }
            if (state.canJoinAnother) {
                ActionRow(
                    label = stringResource(R.string.s_join_another_family),
                    icon = { Icon(Icons.Outlined.GroupAdd, contentDescription = null) },
                    onClick = onJoinFamily,
                )
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.titleMedium,
        modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 8.dp, bottom = 4.dp),
    )
}

/**
 * One family. Read aloud as "<name>, <n> unread" (the visible row is
 * cleared of its own semantics so it is not read twice), and marked
 * selected when it is the one shown.
 */
@Composable
private fun FamilyRow(family: FamilyDto, unread: Int, current: Boolean, onClick: () -> Unit) {
    val description = stringResource(R.string.s_family_unread, family.name, unread)
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(role = Role.Button, onClick = onClick)
            .semantics {
                contentDescription = description
                selected = current
            },
    ) {
        ListItem(
            modifier = Modifier.clearAndSetSemantics { },
            headlineContent = {
                Text(family.name, maxLines = 2, overflow = TextOverflow.Ellipsis)
            },
            leadingContent = { Icon(Icons.Outlined.Groups, contentDescription = null) },
            trailingContent = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (unread > 0) Badge { Text(unread.toString()) }
                    if (current) {
                        Spacer(Modifier.width(8.dp))
                        Icon(
                            Icons.Filled.Check,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.primary,
                        )
                    }
                }
            },
            colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
        )
    }
}

@Composable
private fun ActionRow(label: String, icon: @Composable () -> Unit, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(label) },
        leadingContent = icon,
        modifier = Modifier.clickable(role = Role.Button, onClick = onClick),
        colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
    )
}

/**
 * The banner at the top of the chat list while an invitation is waiting:
 * "<Inviter> invited you to “<Family>”", with Join and Decline.
 */
@Composable
fun InvitationBanner(
    invitation: InvitationDto,
    busy: Boolean,
    onJoin: () -> Unit,
    onDecline: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
    ) {
        InvitationContent(
            invitation = invitation,
            busy = busy,
            onJoin = onJoin,
            onDecline = onDecline,
            modifier = Modifier.padding(16.dp),
        )
    }
}

@Composable
private fun InvitationContent(
    invitation: InvitationDto,
    busy: Boolean,
    onJoin: () -> Unit,
    onDecline: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Outlined.Mail, contentDescription = null, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(12.dp))
            Text(
                text = stringResource(
                    R.string.s_invited_you_to,
                    invitation.invitedBy.displayName,
                    invitation.familyName,
                ),
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        Spacer(Modifier.height(8.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.End,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TextButton(onClick = onDecline, enabled = !busy) {
                Text(stringResource(R.string.s_decline))
            }
            Spacer(Modifier.width(8.dp))
            FilledTonalButton(onClick = onJoin, enabled = !busy) {
                Text(stringResource(R.string.s_join))
            }
        }
    }
}
