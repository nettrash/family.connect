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
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Badge
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
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
    viewModel: ChatListViewModel = hiltViewModel(),
) {
    val chats by viewModel.chats.collectAsStateWithLifecycle()
    val members by viewModel.pickableMembers.collectAsStateWithLifecycle()
    val isOnline by viewModel.isOnline.collectAsStateWithLifecycle()
    val socketState by viewModel.socketState.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()
    var showPicker by remember { mutableStateOf(false) }

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
        topBar = {
            TopAppBar(
                title = { Text("Chats") },
                actions = {
                    IconButton(onClick = onOpenSettings) {
                        Icon(Icons.Filled.Settings, contentDescription = "Settings")
                    }
                },
            )
        },
        floatingActionButton = {
            FloatingActionButton(onClick = { showPicker = true }) {
                Icon(Icons.Filled.Add, contentDescription = "New chat")
            }
        },
    ) { padding ->
        Column(modifier = Modifier.fillMaxSize().padding(padding)) {
            OfflineBanner(isOnline = isOnline, socketState = socketState)
            error?.let {
                ErrorCard(
                    message = it,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    onRetry = viewModel::dismissError,
                )
            }
            if (chats.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center,
                ) {
                    EmptyState(
                        icon = Icons.AutoMirrored.Filled.Chat,
                        title = "No chats yet",
                        subtitle = "The family chat appears as soon as you're connected.",
                    )
                }
            } else {
                LazyColumn(modifier = Modifier.fillMaxSize()) {
                    items(chats, key = { it.id }) { chat ->
                        ChatRow(chat = chat, onClick = { onOpenChat(chat.id) })
                    }
                }
            }
        }
    }

    if (showPicker) {
        ModalBottomSheet(onDismissRequest = { showPicker = false }) {
            Text(
                text = "New chat",
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )
            if (members.isEmpty()) {
                Text(
                    text = "No other family members yet — share the invite code first.",
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
                            Avatar(name = member.displayName, userId = member.userId)
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
    onClick: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Family chat keys its avatar color off the chat id; direct
        // chats off the peer, so the color matches the person everywhere.
        Avatar(
            name = chat.title,
            userId = chat.peerUserId ?: chat.id,
            size = 48,
        )
        Spacer(Modifier.width(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = chat.title,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = chat.lastMessageBody ?: "No messages yet",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
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
