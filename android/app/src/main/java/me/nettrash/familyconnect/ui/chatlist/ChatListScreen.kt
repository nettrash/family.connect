/*
 * ChatListScreen.kt
 * Family Connect (Android)
 *
 * The home screen: family chat pinned on top, direct chats by recency.
 * Row = avatar + title + last-message preview + relative time + unread
 * badge. FAB opens a bottom-sheet member picker (idempotent get-or-
 * create direct chat). OfflineBanner under the app bar.
 *
 * iOS counterpart: ios/FamilyConnect/UI/ChatList/ChatListView.swift
 */

package me.nettrash.familyconnect.ui.chatlist

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.outlined.StickyNote2
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material3.Badge
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.compose.material3.BadgedBox
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.EmptyState
import me.nettrash.familyconnect.ui.components.ErrorCard
import me.nettrash.familyconnect.ui.components.OfflineBanner
import me.nettrash.familyconnect.util.TimeFormat

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatListScreen(
    onOpenChat: (Long) -> Unit,
    onOpenSettings: () -> Unit,
    onOpenBoard: () -> Unit,
    /** The chat open beside this list in the two-pane shape, drawn as selected; null on the phone. */
    selectedChatId: Long? = null,
    viewModel: ChatListViewModel = hiltViewModel(),
) {
    // The chats StateFlow seeds with null in the ViewModel until Room's
    // first read lands: the null sentinel separates "not settled yet"
    // (skeleton rows) from "settled and truly empty" (real empty state),
    // so cold entry never flashes stringResource(R.string.s_no_chats_yet).
    val chats by viewModel.chats.collectAsStateWithLifecycle()
    val members by viewModel.pickableMembers.collectAsStateWithLifecycle()
    val avatarVersions by viewModel.avatarVersions.collectAsStateWithLifecycle()
    val newNoteCount by viewModel.newNoteCount.collectAsStateWithLifecycle()
    val isOnline by viewModel.isOnline.collectAsStateWithLifecycle()
    val blockedUserIds by viewModel.blockedUserIds.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val socketState by viewModel.socketState.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()
    var showPicker by remember { mutableStateOf(false) }
    val listState = rememberLazyListState()
    val scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior()

    LaunchedEffect(Unit) {
        viewModel.navigateToChat.collect { chatId ->
            showPicker = false
            onOpenChat(chatId)
        }
    }

    // Android 13+ notification permission, asked the FIRST time the chat
    // list becomes visible: this is the moment notifications visibly earn
    // their keep (you're in a family, messages will arrive) — unlike an
    // app-launch prompt with zero context, which users reflexively deny.
    // A decline is non-fatal (everything works, foreground delivery rides
    // the socket; pushes are simply never shown) and is deliberately not
    // re-prompted in-session: nagging converts "not now" into "never
    // allow". rememberSaveable scopes "in-session" to this back-stack
    // entry's lifetime, which matches a session in practice.
    val context = LocalContext.current
    var notificationPermissionRequested by rememberSaveable { mutableStateOf(false) }
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { /* decline is non-fatal — see above */ }
    LaunchedEffect(Unit) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            !notificationPermissionRequested &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            notificationPermissionRequested = true
            permissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    LifecycleResumeEffect(Unit) {
        viewModel.refresh()
        onPauseOrDispose { }
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.s_chats)) },
                actions = {
                    IconButton(onClick = {
                        viewModel.markBoardSeen()
                        onOpenBoard()
                    }) {
                        // Notes pinned since this device last showed the
                        // board — see SettingsRepository.boardSeenNoteId for
                        // why it is not the sync cursor.
                        BadgedBox(
                            badge = {
                                if (newNoteCount > 0) {
                                    Badge { Text(newNoteCount.toString()) }
                                }
                            },
                        ) {
                            Icon(
                                Icons.Outlined.StickyNote2,
                                contentDescription = stringResource(R.string.s_board),
                            )
                        }
                    }
                    IconButton(onClick = onOpenSettings) {
                        Icon(Icons.Filled.Settings, contentDescription = stringResource(R.string.s_settings))
                    }
                },
                scrollBehavior = scrollBehavior,
            )
        },
        floatingActionButton = {
            ExtendedFloatingActionButton(
                text = { Text(stringResource(R.string.s_new_chat)) },
                icon = { Icon(Icons.Filled.Add, contentDescription = stringResource(R.string.s_new_chat)) },
                onClick = { showPicker = true },
                expanded = !listState.canScrollBackward,
            )
        },
    ) { padding ->
        Column(modifier = Modifier.fillMaxSize().padding(padding)) {
            OfflineBanner(isOnline = isOnline, socketState = socketState)
            // The exit animation still needs text while shrinking, so
            // remember the last non-null error (same trick as
            // OfflineBanner).
            var lastError by remember { mutableStateOf("") }
            error?.let { lastError = it }
            AnimatedVisibility(
                visible = error != null,
                enter = expandVertically() + fadeIn(),
                exit = shrinkVertically() + fadeOut(),
            ) {
                ErrorCard(
                    message = lastError,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    onRetry = viewModel::dismissError,
                )
            }
            val chatList = chats
            when {
                chatList == null -> ChatListSkeleton()
                chatList.isEmpty() -> {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        EmptyState(
                            icon = Icons.AutoMirrored.Filled.Chat,
                            title = stringResource(R.string.s_no_chats_yet),
                            subtitle = stringResource(R.string.s_the_family_chat_appears_as_soon_as_you_re_connected),
                        )
                    }
                }
                else -> {
                    // Bottom room for the New-chat FAB, which otherwise
                    // sat over the last row's time and unread badge.
                    LazyColumn(
                        state = listState,
                        modifier = Modifier.fillMaxSize(),
                        contentPadding = PaddingValues(bottom = 96.dp),
                    ) {
                        items(chatList, key = { it.id }) { chat ->
                            ChatRow(
                                chat = chat,
                                previewHidden = chat.lastMessageSenderId
                                    ?.let { it != myUserId && it in blockedUserIds } == true,
                                selected = chat.id == selectedChatId,
                                onClick = { onOpenChat(chat.id) },
                                peerAvatarVersion = chat.peerUserId
                                    ?.let { avatarVersions[it] } ?: 0L,
                                modifier = Modifier.animateItem(
                                    fadeInSpec = tween(200),
                                    placementSpec = spring(stiffness = Spring.StiffnessMediumLow),
                                ),
                            )
                        }
                    }
                }
            }
        }
    }

    if (showPicker) {
        ModalBottomSheet(
            onDismissRequest = { showPicker = false },
            containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
        ) {
            Text(
                text = stringResource(R.string.s_new_chat),
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 16.dp),
            )
            if (members.isEmpty()) {
                Text(
                    text = stringResource(R.string.s_no_other_family_members_yet_share_the_invite_code_first),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 16.dp),
                )
            } else {
                members.forEach { member ->
                    ListItem(
                        headlineContent = { Text(member.displayName) },
                        supportingContent = { Text("@${member.username}") },
                        leadingContent = {
                            Avatar(
                                name = member.displayName,
                                userId = member.userId,
                                size = 48,
                                avatarVersion = member.avatarVersion,
                            )
                        },
                        modifier = Modifier.clickable { viewModel.openDirectChat(member.userId) },
                    )
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun ChatRow(
    chat: ChatEntity,
    /**
     * Whether the last message came from somebody blocked, in which case
     * the preview is the placeholder rather than their text.
     *
     * The row still counts and still sorts by that message: in the family
     * chat a blocked member's message keeps moving `unread_count` and may
     * still BE `last_message`, because the count is the other half of the
     * read marker and a count that changed when you blocked somebody is a
     * quantity their own behaviour can be tested against
     * (docs/protocol.md, `GET /chats`).
     */
    previewHidden: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    /** The peer's profile-picture version; 0 for the family chat. */
    peerAvatarVersion: Long = 0,
    /** Open beside the list (two-pane shape): drawn as the selected row. */
    selected: Boolean = false,
) {
    val hasUnread = chat.unreadCount > 0
    Row(
        modifier = modifier
            .fillMaxWidth()
            .then(
                if (selected) {
                    Modifier.background(MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.6f))
                } else {
                    Modifier
                },
            )
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // The family chat gets a role mark, not initials: it's everyone's
        // chat, so no one person's color. Direct chats key their avatar
        // color off the peer, so the color matches the person everywhere.
        if (chat.kind == "family") {
            FamilyAvatarMark()
        } else {
            Avatar(
                name = chat.title,
                userId = chat.peerUserId ?: chat.id,
                size = 48,
                avatarVersion = peerAvatarVersion,
            )
        }
        Spacer(Modifier.width(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = chat.title,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = if (hasUnread) FontWeight.SemiBold else null,
                // The family chat's title is the family's NAME — "The
                // Harper-Whittington Family" — the one title here that is
                // not a person's, and the one the top bar no longer
                // carries. It gets a second line; a person's name keeps one.
                maxLines = if (chat.kind == "family") 2 else 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                // Not revealable from here: the peek is a per-ROW gesture
                // in the thread, and a chat list has no row to reveal.
                text = when {
                    previewHidden -> stringResource(R.string.s_hidden_blocked_member)
                    else -> chat.lastMessageBody ?: stringResource(R.string.s_no_messages_yet)
                },
                style = MaterialTheme.typography.bodyMedium,
                color = if (hasUnread) {
                    MaterialTheme.colorScheme.onSurface
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Spacer(Modifier.width(8.dp))
        Column(horizontalAlignment = Alignment.End) {
            chat.lastMessageAt?.let {
                Text(
                    text = TimeFormat.listTime(it),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (chat.unreadCount > 0) {
                Spacer(Modifier.height(4.dp))
                Badge {
                    Text(
                        text = chat.unreadCount.toString(),
                        fontWeight = FontWeight.SemiBold,
                    )
                }
            }
        }
    }
}

@Composable
internal fun FamilyAvatarMark(size: Int = 48) {
    Box(
        modifier = Modifier
            .size(size.dp)
            .background(MaterialTheme.colorScheme.primaryContainer, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = Icons.Outlined.Home,
            contentDescription = null,
            // The glyph keeps its proportion of the disc at every size.
            modifier = Modifier.size((size * 22 / 48).dp),
            tint = MaterialTheme.colorScheme.onPrimaryContainer,
        )
    }
}

// Shown only while the first Room emission is pending; mirrors the real
// row geometry (48dp avatar, 16/10dp padding) so settling doesn't shift
// anything.
@Composable
private fun ChatListSkeleton() {
    val pulse by rememberInfiniteTransition(label = "chatListSkeleton")
        .animateFloat(
            initialValue = 0.4f,
            targetValue = 0.8f,
            animationSpec = infiniteRepeatable(tween(700), RepeatMode.Reverse),
            label = "skeletonAlpha",
        )
    val barColor = MaterialTheme.colorScheme.surfaceContainerHigh
    Column {
        repeat(3) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 10.dp)
                    .alpha(pulse),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(Modifier.size(48.dp).background(barColor, CircleShape))
                Spacer(Modifier.width(12.dp))
                Column {
                    Box(
                        Modifier
                            .width(140.dp)
                            .height(14.dp)
                            .background(barColor, RoundedCornerShape(4.dp)),
                    )
                    Spacer(Modifier.height(8.dp))
                    Box(
                        Modifier
                            .width(220.dp)
                            .height(12.dp)
                            .background(barColor, RoundedCornerShape(4.dp)),
                    )
                }
            }
        }
    }
}
