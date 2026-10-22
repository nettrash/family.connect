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

import me.nettrash.familyconnect.ui.components.ProseKeyboard
import me.nettrash.familyconnect.ui.components.EmojiTypingField
import me.nettrash.familyconnect.ui.chat.LocalTranscripts
import androidx.compose.runtime.CompositionLocalProvider
import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.TextFieldDefaults
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
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
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
import me.nettrash.familyconnect.ui.chat.NoLinkPreviews
import me.nettrash.familyconnect.ui.components.LocalMotionHold
import me.nettrash.familyconnect.util.GroupRules
import me.nettrash.familyconnect.util.MemberMention
import me.nettrash.familyconnect.ui.chat.MentionSuggestionsRow
import me.nettrash.familyconnect.ui.chat.ComposerKeys
import me.nettrash.familyconnect.ui.chat.ComposerPill
import me.nettrash.familyconnect.ui.chat.ComposerPillMinHeight
import me.nettrash.familyconnect.ui.chat.ComposerSlot
import me.nettrash.familyconnect.ui.chat.RecordSendButton
import me.nettrash.familyconnect.ui.chat.SLOT_LEVELLING
import me.nettrash.familyconnect.ui.chat.StickerFieldButton
import me.nettrash.familyconnect.ui.chat.composerAction
import me.nettrash.familyconnect.ui.chat.composerFieldColors
import me.nettrash.familyconnect.ui.chat.composerFieldPadding
import me.nettrash.familyconnect.ui.components.ConversationColumn
import me.nettrash.familyconnect.ui.components.conversationComposerColumn
import me.nettrash.familyconnect.ui.chat.shareWithSystem
import me.nettrash.familyconnect.ui.components.AssistantConsentDialog
import me.nettrash.familyconnect.ui.stickers.StickerNotices
import me.nettrash.familyconnect.ui.stickers.StickerPanelSheet
import me.nettrash.familyconnect.ui.stickers.StickerPreviewDialog
import me.nettrash.familyconnect.ui.stickers.StickerViewModel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ThreadScreen(
    chatId: Long,
    rootId: Long,
    onBack: () -> Unit,
    /** Open another chat — the one-to-one a tapped mention leads to. */
    onOpenChat: (Long) -> Unit = {},
    /**
     * The chat itself went (ThreadViewModel.chatGone): leave past it, since
     * the conversation underneath is gone too. Plain Back by default.
     */
    onChatGone: () -> Unit = onBack,
    viewModel: ThreadViewModel = hiltViewModel(),
    /**
     * The family's chat stickers: the panel in this screen's composer, and
     * the larger view a tapped sticker opens (docs/protocol.md, "Sticker
     * pack").
     */
    stickerViewModel: StickerViewModel = hiltViewModel(),
) {
    val items by viewModel.items.collectAsStateWithLifecycle()
    // Leave when the chat itself goes — ChatScreen's rule, for a thread of a
    // group left, removed from or deleted (docs/protocol.md, "Groups").
    val chatGone by viewModel.chatGone.collectAsStateWithLifecycle()
    val currentOnChatGone by rememberUpdatedState(onChatGone)
    LaunchedEffect(chatGone) {
        if (chatGone) currentOnChatGone()
    }
    var viewingSticker by remember { mutableStateOf<AttachmentDto?>(null) }
    // Null on a server that predates packs, and then no button is drawn.
    val stickerLimits by stickerViewModel.limits.collectAsStateWithLifecycle()
    var stickerPanelOpen by rememberSaveable { mutableStateOf(false) }
    val assistantConsentAsk by viewModel.assistantConsentAsk.collectAsStateWithLifecycle()
    StickerNotices(stickerViewModel)
    val state by viewModel.state.collectAsStateWithLifecycle()
    val chat by viewModel.chat.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val memberNames by viewModel.memberNames.collectAsStateWithLifecycle()
    val memberAvatars by viewModel.memberAvatars.collectAsStateWithLifecycle()
    val blockedUserIds by viewModel.blockedUserIds.collectAsStateWithLifecycle()
    // Narrowed to the group's members in a group (GroupRules.mentionRoster).
    val mentionRoster by viewModel.nameableRoster.collectAsStateWithLifecycle()
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
    // Emoji as you type (#101): the live replacement a Backspace takes back.
    val replyEmoji = remember { EmojiTypingField() }
    // The Send disc's own focus, as the chat's slot has one.
    val sendFocus = remember { FocusRequester() }
    var viewingAlbum by remember { mutableStateOf<AttachmentAlbum?>(null) }
    var pendingSave by remember { mutableStateOf<AttachmentDto?>(null) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val listState = rememberLazyListState()
    // The chat's rule for moving pictures, here too: they hold still while
    // the chain scrolls or a viewer is over it (ui/components/MotionHold.kt).
    val motionHold = remember {
        derivedStateOf { listState.isScrollInProgress || viewingAlbum != null || viewingSticker != null }
    }
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

    // "Show text" under every recording here reaches the screen's own
    // Transcripts through this, rather than through every bubble's
    // parameters (docs/protocol.md, "Transcripts on request").
    CompositionLocalProvider(LocalTranscripts provides viewModel.transcripts) {
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
                Column(
                    modifier = Modifier
                        .navigationBarsPadding()
                        .imePadding()
                        // The chat's column (#106): the composer as wide as
                        // the chain above it, centred on a wide pane.
                        .conversationComposerColumn(),
                ) {
                // The roster, while a member is being named (docs/protocol.md,
                // "Mentioning a member") — the chat composer's own strip,
                // members only: a reply in a thread is not where #106 offers
                // the assistant, which the family chat's composer does. In a
                // group, as in the family chat.
                val mentionCandidates = if (GroupRules.offersMemberMentions(chat?.kind)) {
                    MemberMention.query(draft.text)?.let { query ->
                        MemberMention.candidates(
                            mentionRoster, query, excluding = blockedUserIds + setOfNotNull(myUserId),
                        ).map { MemberMention.Suggestion.Member(it) }
                    }.orEmpty()
                } else {
                    emptyList()
                }
                if (mentionCandidates.isNotEmpty()) {
                    MentionSuggestionsRow(
                        candidates = mentionCandidates,
                        onPick = { suggestion ->
                            val accepted = MemberMention.accept(draft.text, suggestion)
                            draft = TextFieldValue(accepted, TextRange(accepted.length))
                        },
                    )
                }
                // Disabled until the root is here to answer: a reply
                // with nothing to quote is not a reply in this chain.
                val canSend = hasRoot && draft.text.isNotBlank()
                val sendReply = {
                    if (canSend) {
                        viewModel.send(draft.text)
                        draft = TextFieldValue("")
                    }
                }
                // The main composer's row, less what a thread does not have
                // (#106, docs/composer-and-wide-layout-2026-10-20.md, 2.1):
                // the same filled pill with the stickers inside it, and the
                // same Send disc, disabled while blank — no paperclip and no
                // microphone, as before. It used to be an outlined field
                // with a grey glyph, a second product beside the first.
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 8.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.Bottom,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    // A sticker can be sent in every chat a message can,
                    // and a thread's composer is one: it answers the root,
                    // like the words beside it.
                    val stickers = stickerLimits != null
                    ComposerPill(
                        modifier = Modifier.weight(1f),
                        trailing = {
                            if (stickers) {
                                StickerFieldButton(
                                    enabled = hasRoot,
                                    onClick = { stickerPanelOpen = true },
                                    modifier = Modifier.testTag("thread-stickers"),
                                )
                            }
                        },
                    ) {
                        // The value-based field, for the caret (see `draft`),
                        // drawn by Material's own decoration so it takes the
                        // pill's padding — the value-based TextField has no
                        // contentPadding of its own.
                        val fieldInteraction = remember { MutableInteractionSource() }
                        val fieldColors = composerFieldColors()
                        BasicTextField(
                            value = draft,
                            // Emoji as you type (#101), like every prose field.
                            onValueChange = { draft = replyEmoji.onValueChange(draft, it) },
                            modifier = Modifier
                                .weight(1f)
                                .heightIn(min = ComposerPillMinHeight)
                                // A hardware keyboard sends with Enter here too
                                // (#106, 2.3); the on-screen keyboard's Enter
                                // still breaks the line.
                                .onPreviewKeyEvent { event ->
                                    when (event.composerAction()) {
                                        ComposerKeys.Action.SEND -> {
                                            sendReply()
                                            true
                                        }
                                        ComposerKeys.Action.NEWLINE, ComposerKeys.Action.PASS -> false
                                    }
                                },
                            textStyle = LocalTextStyle.current.copy(color = MaterialTheme.colorScheme.onSurface),
                            cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                            keyboardOptions = ProseKeyboard.Sentences,
                            maxLines = 5,
                            interactionSource = fieldInteraction,
                            decorationBox = { innerTextField ->
                                TextFieldDefaults.DecorationBox(
                                    value = draft.text,
                                    innerTextField = innerTextField,
                                    enabled = true,
                                    singleLine = false,
                                    visualTransformation = VisualTransformation.None,
                                    interactionSource = fieldInteraction,
                                    placeholder = {
                                        Text(
                                            stringResource(R.string.s_reply_in_thread),
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                    },
                                    colors = fieldColors,
                                    contentPadding = composerFieldPadding(hasTrailing = stickers),
                                )
                            },
                        )
                    }
                    RecordSendButton(
                        slot = if (canSend) ComposerSlot.Slot.Send else ComposerSlot.Slot.SendDisabled,
                        onActivate = {},
                        onSend = sendReply,
                        onStopAndListen = {},
                        onDeleteRecording = {},
                        onRecordFromMenu = {},
                        focusRequester = sendFocus,
                        modifier = Modifier.padding(vertical = SLOT_LEVELLING),
                    )
                }
                }
            }
        },
    ) { padding ->
        BoxWithConstraints(modifier = Modifier.fillMaxSize().padding(padding)) {
            // The chat's column (#106, ConversationColumn): the chain uses a
            // tablet's width, centred, its balloons capped at 560dp by the
            // bubble's own rule — the composer below is held to the same.
            val listPadding = ConversationColumn.listPadding(maxWidth)
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
                    CompositionLocalProvider(LocalMotionHold provides motionHold) {
                        LazyColumn(
                            state = listState,
                            modifier = Modifier.weight(1f),
                            contentPadding = PaddingValues(horizontal = listPadding, vertical = 8.dp),
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
                                    answerFailure = null,
                                    myUserId = myUserId,
                                    memberNames = memberNames,
                                    memberAvatars = memberAvatars,
                                    // Link previews are the chat's fetch-and-
                                    // cache machinery; a chain does without.
                                    linkPreviews = NoLinkPreviews,
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
                                        if (attachment.isSticker) {
                                            // A sticker is shown larger, with
                                            // "Add to family stickers" — as in
                                            // the chat, not in the photo viewer.
                                            viewingSticker = attachment
                                        } else if (!attachment.isFile) {
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
    }

    if (stickerPanelOpen) {
        StickerPanelSheet(
            viewModel = stickerViewModel,
            onPick = { item ->
                // One tap sends — no caption, no confirmation — and the
                // sheet closes so the sticker is seen landing in the chain.
                stickerPanelOpen = false
                viewModel.beginStickerSend { chat, quote ->
                    stickerViewModel.send(item, chat, quote)
                }
            },
            onDismiss = { stickerPanelOpen = false },
        )
    }

    // The consent screen, raised by a sticker that would reach the model
    // (docs/protocol.md, "Consenting to the assistant") — the chat's own.
    assistantConsentAsk?.let { ask ->
        AssistantConsentDialog(
            processor = ask.processor,
            familyHistory = ask.familyHistory,
            familyVision = ask.familyVision,
            transcripts = ask.transcripts,
            onAgree = { viewModel.agreeToTheAssistant(withLookups = false) },
            onDismiss = viewModel::dismissAssistantConsent,
            lookupProviders = ask.lookupProviders,
            onAgreeWithLookups = { viewModel.agreeToTheAssistant(withLookups = true) },
        )
    }

    // The consent screen, raised by "Show text": the asker is the one
    // sending the recording's sound to the provider (docs/protocol.md,
    // "Transcripts on request"). Agreeing asks for the text again.
    val transcriptConsentAsk by viewModel.transcripts.consentAsk.collectAsStateWithLifecycle()
    transcriptConsentAsk?.let { ask ->
        AssistantConsentDialog(
            processor = ask.processor,
            familyHistory = ask.familyHistory,
            familyVision = ask.familyVision,
            transcripts = ask.transcripts,
            onAgree = viewModel.transcripts::agreed,
            onDismiss = viewModel.transcripts::dismissed,
        )
    }

    viewingSticker?.let { attachment ->
        StickerPreviewDialog(
            attachment = attachment,
            viewModel = stickerViewModel,
            onDismiss = { viewingSticker = null },
        )
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
