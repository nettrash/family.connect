/*
 * ChatScreen.kt
 * Family Connect (Android)
 *
 * The conversation. reverseLayout LazyColumn (index 0 = newest = bottom)
 * keyed by clientMsgId — the key survives the ack because ack UPDATEs
 * the row instead of replacing it. Bubbles: mine end-aligned on
 * primaryContainer; theirs start-aligned Cards on surfaceVariant.
 * Status glyphs on my bubbles: clock (sending), ✓ (sent), ✓✓ (read —
 * direct chats only, serverId ≤ peerLastReadId), red error → retry/
 * delete dialog. Date pills between days; typing shows as the app-bar
 * subtitle. Scrolling within 10 items of the old end triggers loadOlder.
 * Reactions: chips under the bubble (tap toggles); long-press on an
 * acked bubble opens the quick-set sheet.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Chat/ChatView.swift
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.DoneAll
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.ui.components.OfflineBanner
import me.nettrash.familyconnect.util.TimeFormat

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatScreen(
    onBack: () -> Unit,
    viewModel: ChatViewModel = hiltViewModel(),
) {
    val items by viewModel.items.collectAsStateWithLifecycle()
    val chat by viewModel.chat.collectAsStateWithLifecycle()
    val input by viewModel.input.collectAsStateWithLifecycle()
    val typingUser by viewModel.typingUser.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val isOnline by viewModel.isOnline.collectAsStateWithLifecycle()
    val socketState by viewModel.socketState.collectAsStateWithLifecycle()

    var failedActionTarget by remember { mutableStateOf<String?>(null) }

    // The message the reaction quick-set sheet is open for (a snapshot —
    // fine for a transient sheet; a tap toggles and dismisses).
    var reactionTarget by remember { mutableStateOf<ChatListItem.MessageItem?>(null) }
    val listState = rememberLazyListState()

    // The screen counts as "reading" only while RESUMED — this also
    // registers/clears the open chat for the unread-bump rule.
    LifecycleResumeEffect(Unit) {
        viewModel.setResumed(true)
        onPauseOrDispose { viewModel.setResumed(false) }
    }

    // Pagination: nearing the visually-top (oldest) end triggers one
    // guarded loadOlder. derivedStateOf collapses scroll churn into a
    // boolean edge; the effect only runs on the flip to true.
    val nearOldEnd by remember {
        derivedStateOf {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull()?.index ?: 0
            info.totalItemsCount > 0 && last >= info.totalItemsCount - 10
        }
    }
    LaunchedEffect(nearOldEnd) {
        if (nearOldEnd) viewModel.loadOlder()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(chat?.title ?: "")
                        typingUser?.let {
                            Text(
                                text = "$it is typing…",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.primary,
                            )
                        }
                    }
                },
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
                .imePadding()
                .navigationBarsPadding(),
        ) {
            OfflineBanner(isOnline = isOnline, socketState = socketState)

            LazyColumn(
                state = listState,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth(),
                reverseLayout = true,
                contentPadding = androidx.compose.foundation.layout.PaddingValues(
                    horizontal = 12.dp,
                    vertical = 8.dp,
                ),
            ) {
                items(items, key = { it.key }) { item ->
                    when (item) {
                        is ChatListItem.DateSeparator -> DateSeparatorPill(item.label)
                        is ChatListItem.MessageItem -> MessageBubble(
                            item = item,
                            chat = chat,
                            isMine = item.entity.senderId == myUserId,
                            onFailedTap = { failedActionTarget = it },
                            onToggleReaction = viewModel::toggleReaction,
                            onReactionSheet = { reactionTarget = it },
                        )
                    }
                }
            }

            InputBar(
                value = input,
                onValueChange = viewModel::onInputChange,
                onSend = viewModel::send,
            )
        }
    }

    reactionTarget?.let { target ->
        ModalBottomSheet(onDismissRequest = { reactionTarget = null }) {
            Text(
                text = "React",
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )
            Row(
                horizontalArrangement = Arrangement.SpaceEvenly,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            ) {
                QUICK_REACTIONS.forEach { emoji ->
                    // Tapping my current reaction removes it — the same
                    // toggle the chips perform.
                    val selected = target.myReaction == emoji
                    Surface(
                        shape = RoundedCornerShape(20.dp),
                        color = if (selected) {
                            MaterialTheme.colorScheme.primaryContainer
                        } else {
                            MaterialTheme.colorScheme.surface
                        },
                        border = if (selected) {
                            BorderStroke(1.dp, MaterialTheme.colorScheme.primary)
                        } else {
                            null
                        },
                        modifier = Modifier
                            .clip(RoundedCornerShape(20.dp))
                            .clickable {
                                target.entity.serverId?.let { viewModel.toggleReaction(it, emoji) }
                                reactionTarget = null
                            },
                    ) {
                        Text(
                            text = emoji,
                            style = MaterialTheme.typography.headlineSmall,
                            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                        )
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }

    failedActionTarget?.let { clientMsgId ->
        AlertDialog(
            onDismissRequest = { failedActionTarget = null },
            title = { Text("Message not sent") },
            text = { Text("Try sending it again, or delete the draft.") },
            confirmButton = {
                TextButton(onClick = {
                    viewModel.retry(clientMsgId)
                    failedActionTarget = null
                }) {
                    Text("Retry")
                }
            },
            dismissButton = {
                TextButton(onClick = {
                    viewModel.deleteFailed(clientMsgId)
                    failedActionTarget = null
                }) {
                    Text("Delete")
                }
            },
        )
    }
}

@Composable
private fun DateSeparatorPill(label: String) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceVariant,
        ) {
            Text(
                text = label,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MessageBubble(
    item: ChatListItem.MessageItem,
    chat: ChatEntity?,
    isMine: Boolean,
    onFailedTap: (String) -> Unit,
    onToggleReaction: (Long, String) -> Unit,
    onReactionSheet: (ChatListItem.MessageItem) -> Unit,
) {
    val entity = item.entity
    // Long-press opens the quick-set sheet — acked messages only: a
    // pending row has no server id to react to yet. clip keeps the
    // press ripple inside the bubble's rounded shape.
    val bubbleModifier = Modifier
        .widthIn(max = 300.dp)
        .clip(RoundedCornerShape(12.dp))
        .then(
            if (entity.serverId != null) {
                Modifier.combinedClickable(
                    onClick = {},
                    onLongClick = { onReactionSheet(item) },
                )
            } else {
                Modifier
            },
        )
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        horizontalAlignment = if (isMine) Alignment.End else Alignment.Start,
    ) {
        if (item.showSenderName) {
            Text(
                text = item.senderName ?: "Member ${entity.senderId}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = 12.dp, bottom = 2.dp),
            )
        }
        if (isMine) {
            Surface(
                shape = RoundedCornerShape(12.dp),
                color = MaterialTheme.colorScheme.primaryContainer,
                modifier = bubbleModifier,
            ) {
                BubbleContent(entity = entity, item = item, chat = chat, isMine = true, onFailedTap = onFailedTap)
            }
        } else {
            Card(
                shape = RoundedCornerShape(12.dp),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant,
                ),
                modifier = bubbleModifier,
            ) {
                BubbleContent(entity = entity, item = item, chat = chat, isMine = false, onFailedTap = onFailedTap)
            }
        }
        // In the bubble's Column (not BubbleContent) so the chips inherit
        // the side alignment of the bubble they belong to.
        if (item.reactionChips.isNotEmpty()) {
            ReactionChipsRow(
                chips = item.reactionChips,
                onTap = { emoji -> entity.serverId?.let { onToggleReaction(it, emoji) } },
            )
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ReactionChipsRow(
    chips: List<ReactionChip>,
    onTap: (String) -> Unit,
) {
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
        modifier = Modifier
            .widthIn(max = 300.dp)
            .padding(top = 2.dp),
    ) {
        chips.forEach { chip ->
            val shape = RoundedCornerShape(12.dp)
            Surface(
                shape = shape,
                color = if (chip.includesMe) {
                    MaterialTheme.colorScheme.primaryContainer
                } else {
                    MaterialTheme.colorScheme.surfaceVariant
                },
                border = if (chip.includesMe) {
                    BorderStroke(1.dp, MaterialTheme.colorScheme.primary)
                } else {
                    null
                },
                modifier = Modifier
                    .clip(shape)
                    .clickable { onTap(chip.emoji) },
            ) {
                Text(
                    text = if (chip.count > 1) "${chip.emoji} ${chip.count}" else chip.emoji,
                    style = MaterialTheme.typography.labelMedium,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 3.dp),
                )
            }
        }
    }
}

@Composable
private fun BubbleContent(
    entity: MessageEntity,
    item: ChatListItem.MessageItem,
    chat: ChatEntity?,
    isMine: Boolean,
    onFailedTap: (String) -> Unit,
) {
    Column(modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
        Text(
            text = entity.body,
            style = MaterialTheme.typography.bodyMedium,
        )
        if (item.showTimestamp || isMine) {
            Spacer(Modifier.size(2.dp))
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.End,
                modifier = Modifier.align(Alignment.End),
            ) {
                if (item.showTimestamp) {
                    Text(
                        text = TimeFormat.bubbleTime(entity.createdAt),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (isMine) {
                    Spacer(Modifier.width(4.dp))
                    StatusGlyph(entity = entity, chat = chat, onFailedTap = onFailedTap)
                }
            }
        }
    }
}

@Composable
private fun StatusGlyph(
    entity: MessageEntity,
    chat: ChatEntity?,
    onFailedTap: (String) -> Unit,
) {
    when (entity.status) {
        MessageStatus.SENDING -> Icon(
            imageVector = Icons.Filled.Schedule,
            contentDescription = "Sending",
            modifier = Modifier.size(14.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        MessageStatus.SENT -> {
            // ✓✓ only in direct chats: the family chat has many readers
            // and one peer marker would lie.
            val read = chat?.kind == "direct" &&
                entity.serverId != null &&
                entity.serverId <= (chat.peerLastReadId ?: 0L)
            Icon(
                imageVector = if (read) Icons.Filled.DoneAll else Icons.Filled.Check,
                contentDescription = if (read) "Read" else "Sent",
                modifier = Modifier.size(14.dp),
                tint = if (read) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
            )
        }
        MessageStatus.FAILED -> Icon(
            imageVector = Icons.Filled.ErrorOutline,
            contentDescription = "Failed — tap to retry",
            modifier = Modifier
                .size(16.dp)
                .clickable { onFailedTap(entity.clientMsgId) },
            tint = MaterialTheme.colorScheme.error,
        )
    }
}

@Composable
private fun InputBar(
    value: String,
    onValueChange: (String) -> Unit,
    onSend: () -> Unit,
) {
    Surface(tonalElevation = 3.dp) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp, vertical = 6.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            OutlinedTextField(
                value = value,
                onValueChange = onValueChange,
                modifier = Modifier.weight(1f),
                placeholder = { Text("Message") },
                minLines = 1,
                maxLines = 5,
                shape = RoundedCornerShape(24.dp),
            )
            IconButton(
                onClick = onSend,
                enabled = value.isNotBlank(),
            ) {
                Icon(
                    imageVector = Icons.AutoMirrored.Filled.Send,
                    contentDescription = "Send",
                    tint = if (value.isNotBlank()) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                )
            }
        }
    }
}
