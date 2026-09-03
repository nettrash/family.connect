/*
 * FamilyAdminScreen.kt
 * Family Connect (Android)
 *
 * Owner console: pending requests with approve/reject, invite-code
 * rotation (confirmed — the old code dies immediately), join-policy
 * toggle, the family's language, whether a mention of the assistant sees
 * the chat's recent history, member list with remove (confirmed; the
 * owner row has no remove — the protocol forbids removing the owner) and
 * a birthday editor on every row, the owner's own included.
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
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Autorenew
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Cake
import androidx.compose.material.icons.outlined.Language
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.filled.Key
import androidx.compose.material3.OutlinedTextField
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.material.icons.filled.PersonRemove
import androidx.compose.material.icons.outlined.Inbox
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ListItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.toClipEntry
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.dto.BirthdayDto
import me.nettrash.familyconnect.util.TimeFormat
import me.nettrash.familyconnect.util.daysInBirthdayMonth
import java.time.LocalDate
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.DestructiveTextButton
import me.nettrash.familyconnect.ui.components.EmptyState
import me.nettrash.familyconnect.ui.components.ErrorCard
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Remove
import me.nettrash.familyconnect.util.MemberCap
import androidx.annotation.StringRes
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.background
import androidx.compose.material.icons.outlined.Flag
import androidx.compose.material.icons.outlined.Block
import androidx.compose.material.icons.outlined.LockOpen
import androidx.compose.material3.LocalContentColor
import me.nettrash.familyconnect.ui.components.ReportSheet
import androidx.compose.material.icons.outlined.Shield
import androidx.compose.foundation.layout.Box

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FamilyAdminScreen(
    onBack: () -> Unit,
    viewModel: FamilyAdminViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val blockedUserIds by viewModel.blockedUserIds.collectAsStateWithLifecycle()
    val supportContact by viewModel.supportContact.collectAsStateWithLifecycle()
    val members by viewModel.members.collectAsStateWithLifecycle()
    val isOwner by viewModel.isOwner.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val clipboard = LocalClipboard.current
    val context = LocalContext.current
    // The copy confirmation, resolved here rather than in the button's
    // onClick — not a composable scope.
    val copiedMessage = stringResource(R.string.s_copied)
    val inviteLabel = stringResource(R.string.s_invite_code)
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }
    val scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior()
    var confirmRotate by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf<MemberEntity?>(null) }
    /// The member whose password the owner is resetting; null while closed.
    var resetTarget by remember { mutableStateOf<MemberEntity?>(null) }
    /// The member whose birthday the owner is editing — their own row included.
    var birthdayTarget by remember { mutableStateOf<MemberEntity?>(null) }
    var reportTarget by remember { mutableStateOf<MemberEntity?>(null) }
    var pickingLanguage by remember { mutableStateOf(false) }

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
                title = { Text(stringResource(if (isOwner) R.string.s_manage_family else R.string.s_family_members)) },
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
            state.error?.let {
                ErrorCard(message = it, modifier = Modifier.padding(16.dp))
            }

            // Owner-only: join requests, the invite code and the join
            // policy are all owner endpoints on the server. A plain
            // member opens this screen for the roster below.
            if (isOwner) {
                // -- Pending requests ---------------------------------------------
                Text(
                    text = stringResource(R.string.s_join_requests),
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
                )
                if (state.requests.isEmpty()) {
                    EmptyState(
                        icon = Icons.Outlined.Inbox,
                        title = stringResource(R.string.s_no_pending_requests),
                        subtitle = stringResource(R.string.s_share_the_invite_code_below_to_add_someone),
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    state.requests.forEach { request ->
                        key(request.id) {
                            // Resolved in composition: the snackbar text is
                            // chosen inside onClick, which is not a
                            // composable scope, and lint refuses resource
                            // lookups through LocalContext anyway.
                            val approvedMessage = stringResource(R.string.s_approved_member, request.user.displayName)
                            val rejectedMessage = stringResource(R.string.s_rejected_member, request.user.displayName)
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
                                                            snackbarHostState.showSnackbar(approvedMessage)
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
                                                    contentDescription = stringResource(R.string.s_approve_member, request.user.displayName),
                                                )
                                            }
                                            IconButton(
                                                onClick = {
                                                    departingRequests += request.id
                                                    viewModel.reject(request.id) {
                                                        scope.launch {
                                                            snackbarHostState.showSnackbar(rejectedMessage)
                                                        }
                                                    }
                                                },
                                                enabled = !state.busy,
                                                modifier = Modifier.size(40.dp),
                                            ) {
                                                Icon(
                                                    Icons.Filled.Close,
                                                    contentDescription = stringResource(R.string.s_reject_member, request.user.displayName),
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
                    text = stringResource(R.string.s_invite_code),
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
                                        ClipData.newPlainText(inviteLabel, code).toClipEntry(),
                                    )
                                    snackbarHostState.showSnackbar(copiedMessage)
                                }
                            },
                            enabled = state.inviteCode != null,
                        ) {
                            Icon(Icons.Filled.ContentCopy, contentDescription = stringResource(R.string.s_copy_invite_code))
                        }
                        IconButton(onClick = { confirmRotate = true }, enabled = !state.busy) {
                            Icon(Icons.Filled.Autorenew, contentDescription = stringResource(R.string.s_rotate_invite_code))
                        }
                    }
                }
                Text(
                    text = stringResource(R.string.s_rotating_invalidates_the_current_code_immediately),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
                SectionDivider()

                // -- Reports -----------------------------------------------------------
                // The owner is the moderator. Above the policy controls
                // because a report is somebody waiting on an answer, while
                // the settings below are not.
                if (isOwner) {
                    Text(
                        text = stringResource(R.string.s_reports),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 8.dp),
                    )
                    // Drawn even when empty: an owner who has never
                    // received a report otherwise has no surface anywhere
                    // naming the inbox, and would not know it exists until
                    // the first one arrives.
                    if (state.reports.isEmpty()) {
                        Text(
                            text = stringResource(R.string.s_reports_empty),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                        )
                    }
                    state.reports.forEach { report ->
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 6.dp),
                        ) {
                            Text(
                                text = stringResource(reportReasonLabel(report.reason)),
                                style = MaterialTheme.typography.titleSmall,
                            )
                            Text(
                                text = stringResource(
                                    R.string.s_reported_by,
                                    report.reporter.displayName,
                                    report.reported.displayName,
                                ),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            // Drawn ALWAYS when present, and never
                            // truncated: it is frozen precisely because the
                            // author may edit the body away and retention
                            // will sweep the message, and an owner judging
                            // a message has to see all of it.
                            report.messageExcerpt?.takeIf { it.isNotEmpty() }?.let { excerpt ->
                                SelectionContainer {
                                    Text(
                                        text = excerpt,
                                        style = MaterialTheme.typography.bodyMedium,
                                        modifier = Modifier
                                            .fillMaxWidth()
                                            .padding(top = 4.dp)
                                            .background(
                                                MaterialTheme.colorScheme.surfaceContainerHighest,
                                                RoundedCornerShape(6.dp),
                                            )
                                            .padding(8.dp),
                                    )
                                }
                            }
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.End,
                            ) {
                                TextButton(
                                    enabled = !state.busy,
                                    onClick = { viewModel.resolveReport(report.id) },
                                ) {
                                    Text(stringResource(R.string.s_mark_as_handled))
                                }
                            }
                        }
                    }
                    SectionDivider()
                }

                // -- Join policy ------------------------------------------------------
                Text(
                    text = stringResource(R.string.s_join_policy),
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
                        shape = SegmentedButtonDefaults.itemShape(index = 0, count = 3),
                        enabled = !state.busy,
                    ) {
                        Text(stringResource(R.string.s_open))
                    }
                    SegmentedButton(
                        selected = state.joinPolicy == "approval",
                        onClick = { viewModel.setJoinPolicy("approval") },
                        shape = SegmentedButtonDefaults.itemShape(index = 1, count = 3),
                        enabled = !state.busy,
                    ) {
                        Text(stringResource(R.string.s_approval))
                    }
                    SegmentedButton(
                        selected = state.joinPolicy == "closed",
                        onClick = { viewModel.setJoinPolicy("closed") },
                        shape = SegmentedButtonDefaults.itemShape(index = 2, count = 3),
                        enabled = !state.busy,
                    ) {
                        Text(stringResource(R.string.s_closed))
                    }
                }
                Text(
                    // "Closed" needs two things said that the other two do
                    // not: the code stops working, and the requests already
                    // waiting are untouched and can still be approved
                    // (docs/protocol.md, approve). An owner closing the
                    // family to stop new arrivals should not be left
                    // wondering whether they just rejected the queue.
                    text = stringResource(
                        when (state.joinPolicy) {
                            "approval" -> R.string.s_join_approval_caption
                            "closed" -> R.string.s_join_closed_caption
                            else -> R.string.s_join_open_caption
                        },
                    ),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
                SectionDivider()

                // -- Member limit ------------------------------------------------------
                // Hidden entirely when the server does not report a
                // ceiling: there is nothing sensible to bound the stepper
                // by, and inventing a number would be worse than omitting
                // the control.
                state.ceiling?.let { ceiling ->
                    Text(
                        text = stringResource(R.string.s_member_limit),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 8.dp),
                    )
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp),
                    ) {
                        Text(
                            text = stringResource(R.string.s_limit_members),
                            style = MaterialTheme.typography.bodyLarge,
                            modifier = Modifier.weight(1f),
                        )
                        Switch(
                            checked = state.maxMembers != null,
                            enabled = !state.busy,
                            onCheckedChange = { on ->
                                // Turning it on freezes the family where it
                                // stands, which is what reaching for this
                                // almost always means in the moment.
                                viewModel.setMemberCap(
                                    if (on) MemberCap.seed(state.memberCount, ceiling) else null,
                                )
                            },
                        )
                    }
                    state.maxMembers?.let { cap ->
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 4.dp),
                        ) {
                            Text(
                                text = stringResource(R.string.s_most_members),
                                style = MaterialTheme.typography.bodyLarge,
                                modifier = Modifier.weight(1f),
                            )
                            IconButton(
                                enabled = !state.busy && cap > 1,
                                onClick = { viewModel.setMemberCap(MemberCap.clamp(cap - 1, ceiling)) },
                            ) {
                                Icon(Icons.Outlined.Remove, contentDescription = stringResource(R.string.s_decrease))
                            }
                            Text(text = "$cap", style = MaterialTheme.typography.titleMedium)
                            IconButton(
                                enabled = !state.busy && cap < ceiling,
                                onClick = { viewModel.setMemberCap(MemberCap.clamp(cap + 1, ceiling)) },
                            ) {
                                Icon(Icons.Outlined.Add, contentDescription = stringResource(R.string.s_increase))
                            }
                        }
                    }
                    Text(
                        text = when (val capState = MemberCap.state(state.maxMembers, state.memberCount, ceiling)) {
                            is MemberCap.State.OpenToCeiling ->
                                stringResource(R.string.s_member_limit_none, capState.ceiling)
                            is MemberCap.State.Frozen ->
                                stringResource(R.string.s_member_limit_frozen, capState.memberCount)
                            is MemberCap.State.Room ->
                                stringResource(R.string.s_member_limit_room, capState.memberCount, capState.cap)
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                    SectionDivider()
                }

                // -- Family language ---------------------------------------------------
                Text(
                    text = stringResource(R.string.s_family_language),
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
                )
                ListItem(
                    headlineContent = {
                        // Unset is not English, on the wire or here: a
                        // family that never chose reads as "Not set", and
                        // choosing English is a different thing they can
                        // see they have done.
                        Text(
                            FAMILY_LANGUAGES.firstOrNull { it.first == state.language }?.second
                                ?: stringResource(R.string.s_not_set),
                        )
                    },
                    leadingContent = {
                        Icon(
                            Icons.Outlined.Language,
                            contentDescription = null,
                            modifier = Modifier.size(24.dp),
                        )
                    },
                    trailingContent = {
                        Icon(
                            Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    },
                    modifier = Modifier.clickable(enabled = !state.busy) { pickingLanguage = true },
                )
                Text(
                    text = stringResource(R.string.s_family_language_explanation),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
                SectionDivider()

                // -- What a mention of the assistant may see ---------------------------
                // This is the setting that decides what leaves the
                // server, so it says so in full rather than behind a
                // label — same treatment as the two privacy switches in
                // Settings.
                ListItem(
                    headlineContent = { Text(stringResource(R.string.s_assistant_history)) },
                    supportingContent = {
                        Text(stringResource(R.string.s_assistant_history_explanation))
                    },
                    trailingContent = {
                        Switch(
                            checked = state.aiHistory,
                            onCheckedChange = viewModel::setAiHistory,
                            enabled = !state.busy,
                        )
                    },
                    modifier = Modifier.clickable(enabled = !state.busy) {
                        viewModel.setAiHistory(!state.aiHistory)
                    },
                )
                SectionDivider()

                // -- Whether the assistant may LOOK at a photo -------------------------
                // Only on a server that has a deployment which can see.
                // Absent, not disabled: a greyed switch here would say
                // "your server could do this, but something is stopping
                // you", which is not what a missing deployment means
                // (docs/protocol.md, "Pictures").
                if (state.assistantVision) {
                    ListItem(
                        headlineContent = { Text(stringResource(R.string.s_assistant_vision)) },
                        supportingContent = {
                            Text(stringResource(R.string.s_assistant_vision_explanation))
                        },
                        trailingContent = {
                            Switch(
                                checked = state.aiVision,
                                onCheckedChange = viewModel::setAiVision,
                                enabled = !state.busy,
                            )
                        },
                        modifier = Modifier.clickable(enabled = !state.busy) {
                            viewModel.setAiVision(!state.aiVision)
                        },
                    )
                    SectionDivider()
                }

                // -- The THIRD switch: the chat's recent photos too ---------------------
                // PRESENT even on a server that cannot see — disabled, with
                // the reason — where the switch above is absent. Not an
                // inconsistency: the protocol asks for exactly this, because
                // one of the two reasons is something the owner can act on
                // (turn the switch above on first; the server refuses this
                // while it is off) and the other is something they deserve
                // to be told rather than left to find missing
                // (docs/protocol.md, "Recent photos from the family chat" —
                // "What a client shows"). The sentence under it says what
                // leaves and what it costs, then the reason it is withheld,
                // or that it is inert while the history switch is off.
                val historyPhotosSwitch = state.historyPhotosSwitch
                val historyPhotosEnabled = !state.busy && historyPhotosSwitch.isEnabled
                ListItem(
                    headlineContent = { Text(stringResource(R.string.s_assistant_history_photos)) },
                    supportingContent = {
                        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(stringResource(R.string.s_assistant_history_photos_explanation))
                            when (historyPhotosSwitch) {
                                FamilyAdminViewModel.HistoryPhotosSwitch.OFFERED ->
                                    if (!state.aiHistory) {
                                        Text(stringResource(R.string.s_assistant_history_photos_needs_history))
                                    }
                                FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_NO_VISION_DEPLOYMENT ->
                                    Text(stringResource(R.string.s_assistant_history_photos_no_deployment))
                                FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_VISION_OFF ->
                                    Text(stringResource(R.string.s_assistant_history_photos_needs_vision))
                            }
                        }
                    },
                    trailingContent = {
                        Switch(
                            checked = state.aiHistoryPhotos,
                            onCheckedChange = viewModel::setAiHistoryPhotos,
                            enabled = historyPhotosEnabled,
                        )
                    },
                    modifier = Modifier.clickable(enabled = historyPhotosEnabled) {
                        viewModel.setAiHistoryPhotos(!state.aiHistoryPhotos)
                    },
                )
                SectionDivider()
            }

            // -- Members ------------------------------------------------------------
            Text(
                text = stringResource(R.string.s_members),
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
                                // Every member sees a birthday that is
                                // set — knowing when it is, is the whole
                                // point of storing one. Day and month
                                // only: there is no year on the wire and
                                // nothing here computes an age.
                                Text(
                                    "@${member.username}" +
                                        (if (member.role == "owner") " · owner" else "") +
                                        (
                                            member.birthday
                                                ?.let { " · " + TimeFormat.birthday(it.month, it.day) }
                                                .orEmpty()
                                            ),
                                )
                            },
                            leadingContent = {
                                Avatar(
                                    name = member.displayName,
                                    userId = member.userId,
                                    avatarVersion = member.avatarVersion,
                                )
                            },
                            trailingContent = {
                                Row {
                                    // OUTSIDE the isOwner gate below, and
                                    // that is the point: the whole purpose
                                    // of these two is a member with no
                                    // other recourse, and the person they
                                    // need them for may BE the owner. Any
                                    // member may block any other, the owner
                                    // included (docs/protocol.md,
                                    // "Blocking a member").
                                    //
                                    // On the roster as well as on a
                                    // message, because a member row is
                                    // where somebody looks for what they
                                    // can do about a PERSON — and it is
                                    // the only way to report one without
                                    // singling out a message of theirs.
                                    if (member.userId != myUserId) {
                                        // One Safety button opening a menu,
                                        // matching the message menu and both
                                        // Apple platforms. On a roster row it
                                        // also keeps the destructive action
                                        // off the row itself, where Birthday
                                        // and Remove sit a few points away.
                                        var safetyOpen by remember(member.userId) {
                                            mutableStateOf(false)
                                        }
                                        val isBlocked = member.userId in blockedUserIds
                                        Box {
                                            IconButton(
                                                onClick = { safetyOpen = true },
                                                enabled = !state.busy,
                                            ) {
                                                Icon(
                                                    Icons.Outlined.Shield,
                                                    contentDescription = stringResource(
                                                        R.string.s_safety_for, member.displayName,
                                                    ),
                                                )
                                            }
                                            DropdownMenu(
                                                expanded = safetyOpen,
                                                onDismissRequest = { safetyOpen = false },
                                            ) {
                                                DropdownMenuItem(
                                                    text = { Text(stringResource(R.string.s_report_ellipsis)) },
                                                    onClick = {
                                                        safetyOpen = false
                                                        reportTarget = member
                                                    },
                                                )
                                                DropdownMenuItem(
                                                    text = {
                                                        Text(
                                                            text = stringResource(
                                                                if (isBlocked) R.string.s_unblock else R.string.s_block,
                                                            ),
                                                            color = if (isBlocked) {
                                                                LocalContentColor.current
                                                            } else {
                                                                MaterialTheme.colorScheme.error
                                                            },
                                                        )
                                                    },
                                                    onClick = {
                                                        safetyOpen = false
                                                        viewModel.setBlocked(member.userId, !isBlocked)
                                                    },
                                                )
                                            }
                                        }
                                    }
                                if (isOwner) {
                                        // Every row, the owner's own
                                        // included: the roster endpoint
                                        // accepts the owner naming
                                        // themselves, because there is no
                                        // proof being skipped the way the
                                        // password reset skips one
                                        // (protocol.md, "Birthdays").
                                        IconButton(
                                            onClick = { birthdayTarget = member },
                                            enabled = !state.busy,
                                        ) {
                                            Icon(
                                                Icons.Outlined.Cake,
                                                contentDescription = stringResource(
                                                    R.string.s_birthday_for,
                                                    member.displayName,
                                                ),
                                            )
                                        }
                                        // Removing is an owner action. The owner
                                        // can't be removed (protocol:
                                        // cannot_remove_owner) — and that's also me here.
                                        if (member.role != "owner" && member.userId != myUserId) {
                                            // The owner changes their own
                                            // password in Settings, with the
                                            // current one — hence the same
                                            // gate as Remove.
                                            IconButton(
                                                onClick = { resetTarget = member },
                                                enabled = !state.busy,
                                            ) {
                                                Icon(
                                                    Icons.Filled.Key,
                                                    contentDescription =
                                                        stringResource(R.string.s_reset_password_of, member.displayName),
                                                )
                                            }
                                            IconButton(
                                                onClick = { confirmRemove = member },
                                                enabled = !state.busy,
                                            ) {
                                                Icon(
                                                    Icons.Filled.PersonRemove,
                                                    contentDescription = stringResource(R.string.s_remove_member, member.displayName),
                                                    tint = MaterialTheme.colorScheme.error,
                                                )
                                            }
                                        }
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
            title = { Text(stringResource(R.string.s_rotate_the_invite_code)) },
            text = { Text(stringResource(R.string.s_the_current_code_stops_working_immediately_pending_request)) },
            confirmButton = {
                DestructiveTextButton(
                    label = stringResource(R.string.s_rotate),
                    onClick = {
                        confirmRotate = false
                        viewModel.rotateInviteCode()
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmRotate = false }) {
                    Text(stringResource(R.string.s_cancel))
                }
            },
        )
    }
    reportTarget?.let { member ->
        ReportSheet(
            displayName = member.displayName,
            supportContact = supportContact,
            // From the roster: the PERSON, with no message named.
            isAboutMessage = false,
            onDismiss = { reportTarget = null },
            onSubmit = { reason ->
                viewModel.reportMember(member.userId, reason) { reportTarget = null }
            },
        )
    }


    confirmRemove?.let { member ->
        // Resolved here: the confirm button's onClick is not a composable scope.
        val removedMessage = stringResource(R.string.s_removed_member, member.displayName)
        AlertDialog(
            onDismissRequest = { confirmRemove = null },
            title = { Text(stringResource(R.string.s_remove_member_q, member.displayName)) },
            text = { Text(stringResource(R.string.s_they_lose_access_to_the_family_chats_history_stays_and_ret)) },
            confirmButton = {
                DestructiveTextButton(
                    label = stringResource(R.string.s_remove),
                    onClick = {
                        departingMembers += member.userId
                        viewModel.removeMember(member.userId) {
                            scope.launch {
                                snackbarHostState.showSnackbar(removedMessage)
                            }
                        }
                        confirmRemove = null
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { confirmRemove = null }) {
                    Text(stringResource(R.string.s_cancel))
                }
            },
        )
    }

    if (pickingLanguage) {
        LanguagePickerDialog(
            selected = state.language,
            onDismiss = { pickingLanguage = false },
            onPick = { tag ->
                viewModel.setLanguage(tag)
                pickingLanguage = false
            },
        )
    }

    birthdayTarget?.let { member ->
        BirthdayDialog(
            title = stringResource(R.string.s_birthday_for, member.displayName),
            birthday = member.birthday,
            busy = state.busy,
            onDismiss = { birthdayTarget = null },
            onSave = { month, day ->
                viewModel.setMemberBirthday(member.userId, month, day)
                birthdayTarget = null
            },
            onRemove = {
                viewModel.clearMemberBirthday(member.userId)
                birthdayTarget = null
            },
        )
    }

    resetTarget?.let { member ->
        // Same sentence as the key icon's description — "Reset X's
        // password" reads as the deed done, too. Resolved here: onConfirm
        // is not a composable scope.
        val resetMessage = stringResource(R.string.s_reset_password_of, member.displayName)
        SetPasswordDialog(
            title = stringResource(R.string.s_reset_password),
            // Said plainly, because it is not obvious and not undoable.
            explanation = stringResource(R.string.s_member_signed_out_explanation, member.displayName),
            confirmLabel = stringResource(R.string.s_reset),
            busy = state.busy,
            onDismiss = { resetTarget = null },
            onConfirm = { password ->
                viewModel.resetMemberPassword(member.userId, password) {
                    scope.launch {
                        snackbarHostState.showSnackbar(resetMessage)
                    }
                }
                resetTarget = null
            },
        )
    }
}

/**
 * Two matching entries, a length rule, one action.
 *
 * The confirmation field is not ceremony: the server cannot tell a typo
 * from an intention, and on a self-hosted server with no reset email the
 * cost of a typo is being locked out.
 *
 * iOS counterpart: PasswordView.swift.
 */
@Composable
fun SetPasswordDialog(
    title: String,
    explanation: String,
    confirmLabel: String,
    busy: Boolean,
    onDismiss: () -> Unit,
    onConfirm: (String) -> Unit,
    /** Shown above the new-password fields when a current one is needed. */
    currentPassword: String? = null,
    onCurrentPasswordChange: ((String) -> Unit)? = null,
) {
    var password by remember { mutableStateOf("") }
    var confirmation by remember { mutableStateOf("") }
    var problem by remember { mutableStateOf<String?>(null) }
    // Resolved in composition: the button's onClick, where they are
    // chosen, is not a composable scope.
    val tooShort = pluralStringResource(
        R.plurals.s_use_at_least_characters,
        MIN_PASSWORD_LENGTH,
        MIN_PASSWORD_LENGTH,
    )
    val mismatch = stringResource(R.string.s_those_two_do_not_match)

    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(explanation, style = MaterialTheme.typography.bodyMedium)
                if (onCurrentPasswordChange != null) {
                    OutlinedTextField(
                        value = currentPassword.orEmpty(),
                        onValueChange = onCurrentPasswordChange,
                        label = { Text(stringResource(R.string.s_current_password)) },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                    )
                }
                OutlinedTextField(
                    value = password,
                    onValueChange = { password = it; problem = null },
                    label = { Text(stringResource(R.string.s_new_password)) },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                )
                OutlinedTextField(
                    value = confirmation,
                    onValueChange = { confirmation = it; problem = null },
                    label = { Text(stringResource(R.string.s_confirm_new_password)) },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                )
                problem?.let {
                    Text(
                        text = it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = !busy && password.isNotEmpty(),
                onClick = {
                    problem = when {
                        // The rule the server enforces, checked here so the
                        // dialog can say so without a round trip.
                        password.length < MIN_PASSWORD_LENGTH -> tooShort
                        password != confirmation -> mismatch
                        else -> null
                    }
                    if (problem == null) onConfirm(password)
                },
            ) { Text(confirmLabel) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, enabled = !busy) { Text(stringResource(R.string.s_cancel)) }
        },
    )
}

const val MIN_PASSWORD_LENGTH = 8

/**
 * The nine languages a family may declare, each written in ITS OWN —
 * the way every operating system's language picker does it, because
 * somebody looking for their language scans for the word they recognise
 * rather than for its English name.
 *
 * Spelled and ordered exactly as docs/protocol.md spells them: the list
 * is fixed, casing is canonical coming back out, and anything outside it
 * is `invalid_language`. Two of the nine name a SCRIPT rather than a
 * language, and that is the point — a family that reads Cyrillic cannot
 * read an answer that comes back in Latin letters.
 *
 * Deliberately NOT in strings.xml: these names read the same in every
 * locale, so translating them would mean nine copies of one list that
 * must never disagree.
 *
 * `internal` so FamilyLanguagesTest can pin the tags and the scripts
 * against protocol.md and against the iOS list they must match.
 */
internal val FAMILY_LANGUAGES = listOf(
    "en" to "English",
    "de" to "Deutsch",
    "es" to "Español",
    "fr" to "Français",
    "ja" to "日本語",
    "ru" to "Русский",
    "sr" to "Српски",
    // In LATIN letters, matching iOS (FamilyLanguage.swift). This is the
    // one row whose entire purpose is the alphabet: written in Cyrillic
    // it is indistinguishable from the row above it to exactly the
    // person the split exists for, who cannot read either of them.
    "sr-Latn" to "Srpski (latinica)",
    "zh-Hans" to "简体中文",
)

/**
 * Ten choices, so a radio list rather than the segmented row the join
 * policy uses — nine languages plus "not set" do not fit two segments,
 * and would not fit nine either.
 *
 * A pick applies immediately: there is nothing to confirm, and the
 * server's answer is what the row below redraws from.
 */
@Composable
private fun LanguagePickerDialog(
    selected: String?,
    onDismiss: () -> Unit,
    onPick: (String?) -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.s_family_language)) },
        text = {
            Column(modifier = Modifier.verticalScroll(rememberScrollState())) {
                // Unset has to be reachable, because it is a state the
                // wire has and English is not it.
                LanguageChoice(
                    label = stringResource(R.string.s_not_set),
                    selected = selected == null,
                    onClick = { onPick(null) },
                )
                FAMILY_LANGUAGES.forEach { (tag, name) ->
                    LanguageChoice(
                        label = name,
                        selected = selected == tag,
                        onClick = { onPick(tag) },
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_close)) }
        },
    )
}

/**
 * A wire reason to its label. An UNRECOGNISED value falls back to "other"
 * rather than failing: a newer server must never make an owner's inbox
 * unreadable (docs/protocol.md, "Reporting a member").
 */
@StringRes
private fun reportReasonLabel(reason: String): Int = when (reason) {
    "spam" -> R.string.s_report_reason_spam
    "harassment" -> R.string.s_report_reason_harassment
    "inappropriate" -> R.string.s_report_reason_inappropriate
    else -> R.string.s_report_reason_other
}

@Composable
private fun LanguageChoice(label: String, selected: Boolean, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .selectable(selected = selected, role = Role.RadioButton, onClick = onClick)
            .padding(vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Null onClick: the whole row is the target, and a button that
        // handled its own click would swallow half of it.
        RadioButton(selected = selected, onClick = null)
        Spacer(Modifier.width(12.dp))
        Text(label, style = MaterialTheme.typography.bodyLarge)
    }
}

/**
 * A day and a month, and no way to enter a year — the picker has none to
 * ask for, because a birthday on the wire has none to carry
 * (docs/protocol.md, "Birthdays").
 *
 * The day list is the month's own length with February at 29: with no
 * year, 29 February cannot fail to exist, and the server accepts it. The
 * server is still the authority on the day-vs-month rule — this only
 * stops an impossible date being offered in the first place, and a
 * refusal from the server is shown rather than swallowed.
 *
 * Shared with Settings, exactly as SetPasswordDialog is: the owner
 * filling one in for somebody else and a member filling in their own are
 * the same editor over two different endpoints.
 */
@Composable
fun BirthdayDialog(
    title: String,
    birthday: BirthdayDto?,
    busy: Boolean,
    onDismiss: () -> Unit,
    onSave: (month: Int, day: Int) -> Unit,
    onRemove: () -> Unit,
) {
    // Today, when there is nothing set yet: a picker that opens on
    // 1 January looks like a value somebody chose.
    val today = remember { LocalDate.now() }
    // Keyed on the value being edited: one dialog serves every roster
    // row, and a picker that kept the last member's date would offer to
    // save it onto the next one.
    var month by remember(birthday) { mutableStateOf(birthday?.month ?: today.monthValue) }
    var day by remember(birthday) { mutableStateOf(birthday?.day ?: today.dayOfMonth) }

    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    stringResource(R.string.s_day_and_month_no_year),
                    style = MaterialTheme.typography.bodyMedium,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    DatePartField(
                        label = stringResource(R.string.s_month),
                        value = TimeFormat.monthName(month),
                        enabled = !busy,
                        options = (1..12).map { it to TimeFormat.monthName(it) },
                        onPick = { picked ->
                            month = picked
                            // 31 March → February has to give: the day
                            // list is about to be shorter than the day
                            // that is selected.
                            day = day.coerceAtMost(daysInBirthdayMonth(picked))
                        },
                        modifier = Modifier.weight(1.6f),
                    )
                    DatePartField(
                        label = stringResource(R.string.s_day),
                        value = day.toString(),
                        enabled = !busy,
                        options = (1..daysInBirthdayMonth(month)).map { it to it.toString() },
                        onPick = { day = it },
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        },
        confirmButton = {
            TextButton(enabled = !busy, onClick = { onSave(month, day) }) {
                Text(stringResource(R.string.s_save))
            }
        },
        dismissButton = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                // Only offered when there is something to remove — a
                // birthday nobody set has nothing to clear, even though
                // the endpoint would happily 204.
                if (birthday != null) {
                    DestructiveTextButton(
                        label = stringResource(R.string.s_remove),
                        onClick = onRemove,
                    )
                }
                TextButton(onClick = onDismiss, enabled = !busy) {
                    Text(stringResource(R.string.s_cancel))
                }
            }
        },
    )
}

/** One labelled value that opens a menu of the values it may take. */
@Composable
private fun DatePartField(
    label: String,
    value: String,
    enabled: Boolean,
    options: List<Pair<Int, String>>,
    onPick: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    var open by remember { mutableStateOf(false) }
    Column(modifier = modifier) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        OutlinedButton(
            onClick = { open = true },
            enabled = enabled,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(value, maxLines = 1)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            options.forEach { (candidate, text) ->
                DropdownMenuItem(
                    text = { Text(text) },
                    onClick = {
                        onPick(candidate)
                        open = false
                    },
                )
            }
        }
    }
}

@Composable
private fun SectionDivider() {
    HorizontalDivider(
        modifier = Modifier.padding(start = 16.dp),
        color = MaterialTheme.colorScheme.outlineVariant,
    )
}
