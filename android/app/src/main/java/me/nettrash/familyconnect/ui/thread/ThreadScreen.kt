/*
 * ThreadScreen.kt
 * Family Connect (Android)
 *
 * A chain of replies on a screen of its own (docs/protocol.md, "Threads"):
 * the root at the top, the replies below it in order, a composer at the
 * bottom whose sends answer the root.
 *
 * WHY THE CHAT'S OWN BUBBLE. Rows draw with the chat's MessageBubble — the
 * same balloons, the same quotes, the same hidden-row rule for a blocked
 * member, the same reactions and polls — because a second, simpler
 * renderer here is a second place for those rules to drift.
 *
 * iOS counterpart: Views/ThreadView.swift
 */

package me.nettrash.familyconnect.ui.thread

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableStateSetOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.GallerySaver
import me.nettrash.familyconnect.ui.components.AttachmentAlbum
import me.nettrash.familyconnect.ui.chat.AttachmentViewer
import me.nettrash.familyconnect.ui.chat.ChatListItem
import me.nettrash.familyconnect.ui.chat.DateSeparatorPill
import me.nettrash.familyconnect.ui.chat.MessageBubble
import me.nettrash.familyconnect.util.MemberMention
import me.nettrash.familyconnect.ui.chat.MentionSuggestionsRow
import me.nettrash.familyconnect.ui.chat.shareWithSystem

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ThreadScreen(
    chatId: Long,
    rootId: Long,
    onBack: () -> Unit,
    /** Open another chat — the one-to-one a tapped mention leads to. */
    onOpenChat: (Long) -> Unit = {},
    viewModel: ThreadViewModel = hiltViewModel(),
) {
    val items by viewModel.items.collectAsStateWithLifecycle()
    val state by viewModel.state.collectAsStateWithLifecycle()
    val chat by viewModel.chat.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val memberNames by viewModel.memberNames.collectAsStateWithLifecycle()
    val memberAvatars by viewModel.memberAvatars.collectAsStateWithLifecycle()
    val blockedUserIds by viewModel.blockedUserIds.collectAsStateWithLifecycle()
    val mentionRoster by viewModel.mentionRoster.collectAsStateWithLifecycle()
    val mapPreviewsEnabled by viewModel.mapPreviewsEnabled.collectAsStateWithLifecycle()
    // Rows this reader has peeked at: per row, per screen, never stored —
    // the chat's own reveal (protocol.md, "Blocking a member").
    val revealedMessages = remember { mutableStateSetOf<String>() }
    // A TextFieldValue rather than a String, for the caret: the String
    // overload keeps the previous selection offset when the text is
    // replaced from outside, so accepting "@Anna " while the caret sat
    // after "@An" would leave it INSIDE the name, and the next keystroke
    // would split the token the server checks for
    // (docs/protocol.md, "Mentioning a member").
    var draft by rememberSaveable(stateSaver = TextFieldValue.Saver) {
        mutableStateOf(TextFieldValue(""))
    }
    var viewingAlbum by remember { mutableStateOf<AttachmentAlbum?>(null) }
    var pendingSave by remember { mutableStateOf<AttachmentDto?>(null) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val listState = rememberLazyListState()
    val hasRoot = items.any { it is ChatListItem.MessageItem && it.entity.serverId == rootId }

    LaunchedEffect(chatId, rootId) { viewModel.start(chatId, rootId) }
    // New rows land at the end; follow them, as a chain reads downwards.
    LaunchedEffect(items.size) {
        if (items.isNotEmpty()) listState.animateScrollToItem(items.lastIndex)
    }

    val runSave: (AttachmentDto) -> Unit = { attachment ->
        scope.launch { viewModel.saveToGallery(context, attachment) }
    }
    val requestStorage = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        val attachment = pendingSave
        pendingSave = null
        if (attachment != null && granted) runSave(attachment)
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.s_thread)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.s_back),
                        )
                    }
                },
            )
        },
        bottomBar = {
            Surface(tonalElevation = 2.dp) {
                Column(modifier = Modifier.navigationBarsPadding().imePadding()) {
                // The roster, while a member is being named (docs/protocol.md,
                // "Mentioning a member") — the chat composer's own strip.
                val mentionCandidates = if (chat?.kind == "family") {
                    MemberMention.query(draft.text)?.let { query ->
                        MemberMention.candidates(
                            mentionRoster, query, excluding = blockedUserIds + setOfNotNull(myUserId),
                        )
                    }.orEmpty()
                } else {
                    emptyList()
                }
                if (mentionCandidates.isNotEmpty()) {
                    MentionSuggestionsRow(
                        candidates = mentionCandidates,
                        onPick = { name ->
                            val accepted = MemberMention.accept(draft.text, name)
                            draft = TextFieldValue(accepted, TextRange(accepted.length))
                        },
                    )
                }
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.Bottom,
                ) {
                    OutlinedTextField(
                        value = draft,
                        onValueChange = { draft = it },
                        modifier = Modifier.weight(1f),
                        placeholder = { Text(stringResource(R.string.s_reply_in_thread)) },
                        maxLines = 5,
                    )
                    // Disabled until the root is here to answer: a reply
                    // with nothing to quote is not a reply in this chain.
                    IconButton(
                        onClick = {
                            viewModel.send(draft.text)
                            draft = TextFieldValue("")
                        },
                        enabled = hasRoot && draft.text.isNotBlank(),
                    ) {
                        Icon(
                            Icons.AutoMirrored.Filled.Send,
                            contentDescription = stringResource(R.string.s_send),
                        )
                    }
                }
                }
            }
        },
    ) { padding ->
        Box(modifier = Modifier.fillMaxSize().padding(padding)) {
            when {
                state.loading && items.isEmpty() ->
                    CircularProgressIndicator(modifier = Modifier.align(Alignment.Center))

                items.isEmpty() ->
                    Text(
                        text = stringResource(R.string.e_load_thread_failed),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.align(Alignment.Center).padding(24.dp),
                    )

                else -> Column(modifier = Modifier.fillMaxSize()) {
                    LazyColumn(
                        state = listState,
                        modifier = Modifier.weight(1f),
                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        items(items, key = { it.key }) { item ->
                            when (item) {
                                is ChatListItem.DateSeparator -> DateSeparatorPill(item.label)
                                is ChatListItem.NewMessagesDivider -> Unit
                                is ChatListItem.MessageItem -> MessageBubble(
                                item = item,
                                chat = chat,
                                isMine = item.entity.senderId == myUserId,
                                blockedUserIds = blockedUserIds,
                                revealedMessages = revealedMessages,
                                isStreaming = false,
                                answerFailed = false,
                                myUserId = myUserId,
                                memberNames = memberNames,
                                memberAvatars = memberAvatars,
                                // Link previews are the chat's fetch-and-
                                // cache machinery; a chain does without.
                                linkPreviews = emptyMap(),
                                previewsEnabled = false,
                                mapPreviewsEnabled = mapPreviewsEnabled,
                                onRequestPreview = {},
                                streamUrl = viewModel::attachmentStreamUrl,
                                onFailedTap = {},
                                onToggleReaction = viewModel::toggleReaction,
                                onVote = viewModel::vote,
                                onLongPress = { _, _ -> },
                                onPositioned = { _, _ -> },
                                onTapQuote = {},
                                onTapMention = { userId -> viewModel.openDirectChat(userId, onOpenChat) },
                                onOpenAttachment = { attachment ->
                                    // Photos and videos open in the viewer,
                                    // paged through the message's media as
                                    // in the chat. A file has nothing to
                                    // open here.
                                    if (!attachment.isFile) {
                                        viewingAlbum = AttachmentAlbum.opening(
                                            item.entity.attachmentList,
                                            attachment,
                                        )
                                    }
                                },
                                )
                            }
                        }
                    }
                    if (state.failed) {
                        // What is cached is drawn; what could not be
                        // fetched is said, rather than an empty screen
                        // over a real chain.
                        Text(
                            text = stringResource(R.string.s_thread_partial),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                        )
                    }
                }
            }
        }
    }

    viewingAlbum?.let { album ->
        AttachmentViewer(
            album = album,
            streamUrl = viewModel::attachmentStreamUrl,
            onShare = { attachment ->
                viewingAlbum = null
                scope.launch {
                    viewModel.localFile(attachment)?.let { file ->
                        shareWithSystem(context, file, attachment.mime, "")
                    }
                }
            },
            onSave = { attachment ->
                viewingAlbum = null
                if (viewModel.savingNeedsPermission) {
                    pendingSave = attachment
                    requestStorage.launch(Manifest.permission.WRITE_EXTERNAL_STORAGE)
                } else {
                    runSave(attachment)
                }
            },
            onDismiss = { viewingAlbum = null },
        )
    }
}
