/*
 * ThreadViewModel.kt
 * Family Connect (Android)
 *
 * A chain of replies on a screen of its own (docs/protocol.md, "Threads").
 *
 * WHY THE STORE AND NOT A LIST OF DTOs. The thread read answers whole
 * messages and they go through the same apply a page of history does, so
 * this screen observes the STORE — the root and every row that names it —
 * and is live for free: a reply arriving on the socket, a reaction, an
 * edit, the reader's own send from the composer below, all land in the
 * store and the list redraws. A list held from one fetch would be stale
 * the moment the family answered again.
 *
 * iOS counterpart: Views/ThreadView.swift
 */

package me.nettrash.familyconnect.ui.thread

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import java.io.File
import javax.inject.Inject
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.util.resolvedDisplayName
import me.nettrash.familyconnect.util.MemberMention
import me.nettrash.familyconnect.data.repo.ChatRepository
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.data.repo.GallerySaver
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.ui.chat.AssistantConsent
import me.nettrash.familyconnect.ui.chat.ChatListItem
import me.nettrash.familyconnect.ui.chat.ChatViewModel
import me.nettrash.familyconnect.ui.chat.buildChatItems
import me.nettrash.familyconnect.util.Clock
import me.nettrash.familyconnect.util.resolvedDisplayNames

@OptIn(ExperimentalCoroutinesApi::class)
@HiltViewModel
class ThreadViewModel @Inject constructor(
    @param:ApplicationContext private val appContext: Context,
    private val messageRepository: MessageRepository,
    private val messageDao: MessageDao,
    private val chatDao: ChatDao,
    memberDao: MemberDao,
    private val attachmentApi: AttachmentApi,
    private val attachments: AttachmentRepository,
    private val gallerySaver: GallerySaver,
    private val chatRepository: ChatRepository,
    private val familyRepository: FamilyRepository,
    private val settings: SettingsRepository,
    private val clock: Clock,
) : ViewModel() {

    data class State(
        /** The server read is still on its way; the store may already draw. */
        val loading: Boolean = true,
        /** The server read failed; what is cached is drawn and the gap is said. */
        val failed: Boolean = false,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** (chatId, rootId), once [start] has named them. */
    private val target = MutableStateFlow<Pair<Long, Long>?>(null)
    private val chatId: Long get() = target.value?.first ?: 0L
    private val rootId: Long get() = target.value?.second ?: 0L

    val chat: StateFlow<ChatEntity?> = target.filterNotNull()
        .flatMapLatest { chatDao.observeById(it.first) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    val myUserId: StateFlow<Long?> = settings.state
        .map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    val blockedUserIds: StateFlow<Set<Long>> = settings.state
        .map { it.blockedUserIds }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    val mapPreviewsEnabled: StateFlow<Boolean> = settings.state
        .map { it.mapPreviewsEnabled }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), true)

    val memberAvatars: StateFlow<Map<Long, Long>> = memberDao.observeMembers()
        .map { members -> members.associate { it.userId to it.avatarVersion } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

    val memberNames: StateFlow<Map<Long, String>> = memberDao.observeMembers()
        .map { it.resolvedDisplayNames(appContext) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

    /** Everybody a mention may name (docs/protocol.md, "Mentioning a member"). */
    val mentionRoster: StateFlow<List<MentionDto>> = memberDao.observeActiveMembers()
        .map { members -> members.map { MentionDto(it.userId, it.resolvedDisplayName(appContext)) } }
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    /** A tap on a name opens the one-to-one chat with that member. */
    fun openDirectChat(userId: Long, onOpened: (Long) -> Unit) {
        if (userId == myUserId.value) return
        if (mentionRoster.value.none { it.userId == userId }) return
        viewModelScope.launch {
            when (val result = chatRepository.createDirect(userId)) {
                is ApiResult.Ok -> onOpened(result.value.id)
                else -> Unit
            }
        }
    }

    /**
     * The chain, oldest first, as the rows the chat draws — built by the
     * chat's own item builder so runs, hidden rows, polls and the day
     * pills follow exactly the chat's rules; a chain that runs over
     * several days needs the pills as much as the chat does, because a
     * bubble carries only the time. Only the "N new messages" divider is
     * dropped: it is the chat's, decided at its open.
     */
    val items: StateFlow<List<ChatListItem>> = combine(
        target.filterNotNull().flatMapLatest { messageDao.observeThread(it.second) },
        chat,
        memberDao.observeMembers(),
        memberDao.observeActiveMembers(),
        settings.state,
    ) { rows, chat, members, active, settingsState ->
        buildChatItems(
            // The builder reads newest-first, the chat list's order; the
            // thread reads the other way, so both ends are reversed.
            messagesNewestFirst = rows.asReversed(),
            isFamilyChat = chat?.kind == "family",
            myUserId = settingsState.myUserId ?: -1L,
            memberNames = members.resolvedDisplayNames(appContext),
            nowMillis = clock.now(),
            assistantUserId = settingsState.assistantUserId,
            assistantName = settingsState.assistantName,
            familyMemberCount = active.size,
            blockedUserIds = settingsState.blockedUserIds,
        ).filterNot { it is ChatListItem.NewMessagesDivider }.asReversed()
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    fun start(chatId: Long, rootId: Long) {
        if (target.value == chatId to rootId) return
        target.value = chatId to rootId
        viewModelScope.launch {
            val completed = messageRepository.loadThread(chatId, rootId)
            _state.update { it.copy(loading = false, failed = !completed) }
        }
    }

    /**
     * A reply to the ROOT, whatever the reader was looking at — the
     * iMessage rule, and the one that keeps the root's count honest
     * (protocol.md, "Answering from the thread").
     */
    fun send(body: String) {
        val quote = rootQuote() ?: return
        val trimmed = body.trim()
        if (trimmed.isEmpty()) return
        // The members named, resolved from the text as the chat's composer
        // does (docs/protocol.md, "Mentioning a member") — family chat only.
        val mentions = if (chat.value?.kind == "family") {
            MemberMention.resolve(trimmed, mentionRoster.value).takeIf { it.isNotEmpty() }
        } else {
            null
        }
        viewModelScope.launch {
            messageRepository.send(chatId, trimmed, quote, mentions)
        }
    }

    /** The root, as the quote every answer from this screen carries — or null until it is here. */
    private fun rootQuote(): ReplyToDto? {
        val root = items.value
            .filterIsInstance<ChatListItem.MessageItem>()
            .firstOrNull { it.entity.serverId == rootId }
            ?.entity ?: return null
        val serverId = root.serverId ?: return null
        return ReplyToDto(
            messageId = serverId,
            senderId = root.senderId,
            // Cut exactly as the server will, so the bubble and its ack
            // agree.
            excerpt = ReplyToDto.excerpt(root.body),
        )
    }

    /**
     * A sticker was tapped in this screen's panel: decide whether it may
     * go, and hand [go] the chat and the quote it answers — the ROOT, like
     * everything sent from here.
     *
     * The sticker itself is sent by StickerViewModel, as in the chat. What
     * is this model's is the question before it: in the member's own `ai`
     * chat a sticker is a photo to the assistant, so the consent question
     * is asked first, exactly as the chat's composer asks it
     * (docs/protocol.md, "Consenting to the assistant") — the same gate,
     * [AssistantConsent.stickerGate], and never a way around it.
     */
    fun beginStickerSend(go: (chatId: Long, replyTo: ReplyToDto) -> Unit) {
        val quote = rootQuote() ?: return
        val chatIdNow = chatId
        val chatKind = chat.value?.kind
        viewModelScope.launch {
            val settingsState = settings.state.first()
            when (
                AssistantConsent.stickerGate(
                    chatKind = chatKind,
                    hasAssistant = settingsState.assistantUserId != null,
                    processor = settingsState.assistantProcessor,
                    agreedAt = settingsState.assistantConsentAt,
                )
            ) {
                AssistantConsent.StickerGate.WITHHELD -> Unit
                AssistantConsent.StickerGate.ASK -> _assistantConsentAsked.value = true
                AssistantConsent.StickerGate.SEND -> go(chatIdNow, quote)
            }
        }
    }

    /** Whether the consent screen is up, because a sticker would have reached the model. */
    private val _assistantConsentAsked = MutableStateFlow(false)

    /** What the consent screen must say, or null while it is not up — the chat's own shape. */
    val assistantConsentAsk: StateFlow<ChatViewModel.AssistantConsentAsk?> =
        combine(_assistantConsentAsked, settings.state) { asked, settingsState ->
            val processor = settingsState.assistantProcessor
            if (!asked || processor.isNullOrBlank()) {
                null
            } else {
                ChatViewModel.AssistantConsentAsk(
                    processor = processor,
                    familyHistory = settingsState.familyAiHistory,
                    familyVision = settingsState.familyAiVision,
                )
            }
        }.stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /**
     * Agree. The stamp is the server's. Nothing is sent afterwards: one tap
     * sends a sticker, and the tap that raised the question was answered
     * with the question — the next one goes.
     */
    fun agreeToTheAssistant() {
        viewModelScope.launch {
            _assistantConsentAsked.value = false
            familyRepository.setAssistantConsent(true)
        }
    }

    /** "Not Now": the screen closes and nothing was sent. */
    fun dismissAssistantConsent() {
        _assistantConsentAsked.value = false
    }

    fun vote(messageServerId: Long, optionId: Long) {
        viewModelScope.launch { messageRepository.toggleVote(chatId, messageServerId, optionId) }
    }

    fun toggleReaction(messageServerId: Long, emoji: String) {
        viewModelScope.launch { messageRepository.toggleReaction(chatId, messageServerId, emoji) }
    }

    suspend fun attachmentStreamUrl(attachmentId: Long): Pair<String, Map<String, String>>? =
        attachmentApi.streamUrl(attachmentId)

    suspend fun localFile(attachment: AttachmentDto): File? = attachments.fileFor(attachment)

    /** True when this device needs the legacy storage permission to save. */
    val savingNeedsPermission: Boolean get() = gallerySaver.needsLegacyPermission

    suspend fun saveToGallery(context: Context, attachment: AttachmentDto): GallerySaver.Result {
        val file = attachments.fileFor(attachment) ?: return GallerySaver.Result.FAILED
        return gallerySaver.save(
            context = context,
            file = file,
            mime = attachment.mime,
            displayName = attachment.name ?: attachment.fallbackFileName,
            isVideo = attachment.isVideo,
        )
    }
}
