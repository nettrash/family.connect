/*
 * ChatViewModel.kt
 * Family Connect (Android)
 *
 * One chat's state machine:
 *
 *   items        — DAO flow (windowed by visibleLimit) × chat × members
 *                  → buildChatItems. Pagination grows the window and
 *                  pulls older pages over REST, guarded so a scroll
 *                  storm triggers exactly one fetch.
 *   read markers — while RESUMED, the newest inbound serverId above
 *                  myLastReadId is reported after a 500 ms debounce
 *                  (collectLatest + delay): skimming past a hundred
 *                  messages produces one `read`, not a hundred.
 *   typing       — outbound throttled to one frame per 3 s (matching
 *                  the server's own per-chat throttle); inbound shown
 *                  for 5 s past the last frame (collectLatest restarts
 *                  the expiry timer).
 *   open chat    — registered with ChatRepository while resumed, so
 *                  inbound messages for THIS chat never bump unread.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Chat/ChatViewModel.swift
 */

package me.nettrash.familyconnect.ui.chat

import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.repo.ChatRepository
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.util.Clock
import javax.inject.Inject

@OptIn(ExperimentalCoroutinesApi::class)
@HiltViewModel
class ChatViewModel @Inject constructor(
    savedStateHandle: SavedStateHandle,
    private val messageRepository: MessageRepository,
    private val chatRepository: ChatRepository,
    private val settings: SettingsRepository,
    private val socket: ChatSocket,
    private val clock: Clock,
    memberDao: MemberDao,
    connectivity: ConnectivityObserver,
) : ViewModel() {

    val chatId: Long = checkNotNull(savedStateHandle["chatId"]) { "chatId nav arg missing" }

    // Window into the message table; loadOlder widens it.
    private val visibleLimit = MutableStateFlow(INITIAL_LIMIT)
    private val _loadingOlder = MutableStateFlow(false)

    /** True while an older history page is in flight — drives the list's oldest-end spinner. */
    val loadingOlder: StateFlow<Boolean> = _loadingOlder

    private var reachedStart = false

    private val resumed = MutableStateFlow(false)

    // Eagerly shared (not WhileSubscribed): the read-marker collector
    // reads `.value` off both — a lazily-started StateFlow would hand it
    // stale nulls until the screen happens to subscribe.
    val chat: StateFlow<ChatEntity?> = chatRepository.observeChat(chatId)
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    val myUserId: StateFlow<Long?> = settings.state.map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    // Roster snapshot — sender names in family bubbles, the typing
    // indicator, and the who-reacted popup all resolve through it.
    // Eagerly shared: typing frames can arrive before the items flow has
    // any subscriber.
    val memberNames: StateFlow<Map<Long, String>> = memberDao.observeMembers()
        .map { members -> members.associate { it.userId to it.displayName } }
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyMap())

    // UI-only: flips on the first items emission, so the empty state can
    // tell an actually-empty chat from "the DB flow hasn't answered yet".
    private val _initialLoadSettled = MutableStateFlow(false)

    /** True once the first (possibly empty) items emission has landed. */
    val initialLoadSettled: StateFlow<Boolean> = _initialLoadSettled

    val items: StateFlow<List<ChatListItem>> = combine(
        visibleLimit.flatMapLatest { messageRepository.observeMessages(chatId, it) },
        chat,
        settings.state,
        memberNames,
    ) { messages, chatEntity, settingsState, members ->
        buildChatItems(
            messagesNewestFirst = messages,
            isFamilyChat = chatEntity?.kind == "family",
            myUserId = settingsState.myUserId ?: -1L,
            memberNames = members,
            nowMillis = clock.now(),
        )
    }
        .onEach { _initialLoadSettled.value = true }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    private val _input = MutableStateFlow("")
    val input: StateFlow<String> = _input

    private val _typingUser = MutableStateFlow<String?>(null)

    /** Display name of the member typing right now (5 s expiry). */
    val typingUser: StateFlow<String?> = _typingUser

    val isOnline: StateFlow<Boolean> = connectivity.isOnline
    val socketState: StateFlow<SocketState> = socket.state

    private var lastTypingSentAt = 0L

    init {
        // Read markers: newest inbound acked message, gated on RESUMED,
        // debounced 500 ms. collectLatest restarts the debounce whenever
        // either input changes — the report fires once things settle.
        viewModelScope.launch {
            combine(resumed, items) { isResumed, list ->
                if (!isResumed) null else newestInboundServerId(list)
            }
                .distinctUntilChanged()
                .collectLatest { newest ->
                    if (newest == null) return@collectLatest
                    val lastRead = chat.value?.myLastReadId ?: 0L
                    if (newest > lastRead) {
                        delay(READ_DEBOUNCE_MS)
                        chatRepository.postRead(chatId, newest)
                    }
                }
        }

        // Inbound typing for this chat; each frame restarts the 5 s expiry.
        viewModelScope.launch {
            socket.frames
                .filterIsInstance<ServerFrame.Typing>()
                .collectLatest { frame ->
                    if (frame.chatId != chatId) return@collectLatest
                    _typingUser.value = memberNames.value.getOrDefault(frame.userId, "Someone")
                    delay(TYPING_EXPIRY_MS)
                    _typingUser.value = null
                }
        }
    }

    /** Screen calls this from a LifecycleResumeEffect. */
    fun setResumed(isResumed: Boolean) {
        resumed.value = isResumed
        chatRepository.setOpenChat(if (isResumed) chatId else null)
    }

    fun onInputChange(value: String) {
        _input.value = value
        // Outbound typing, throttled to the server's own 1-per-3s limit —
        // anything faster would be dropped server-side anyway.
        val now = clock.now()
        if (value.isNotBlank() && now - lastTypingSentAt >= TYPING_THROTTLE_MS) {
            if (socket.trySend(ClientFrame.Typing(chatId))) {
                lastTypingSentAt = now
            }
        }
    }

    fun send() {
        val body = _input.value
        if (body.isBlank()) return
        _input.value = ""
        viewModelScope.launch { messageRepository.send(chatId, body) }
    }

    fun retry(clientMsgId: String) {
        viewModelScope.launch { messageRepository.retry(clientMsgId) }
    }

    fun deleteFailed(clientMsgId: String) {
        viewModelScope.launch { messageRepository.deleteFailed(clientMsgId) }
    }

    /**
     * Tap on a chip or a quick-set emoji. The repository decides set vs
     * remove from the row's current state; only acked messages
     * (serverId != null) can be reacted to — the UI gates on that.
     */
    fun toggleReaction(messageServerId: Long, emoji: String) {
        viewModelScope.launch { messageRepository.toggleReaction(chatId, messageServerId, emoji) }
    }

    /**
     * Called when the list scrolls near its old end. Guarded: one fetch
     * at a time, and none once the start of history is reached.
     */
    fun loadOlder() {
        if (reachedStart || !_loadingOlder.compareAndSet(expect = false, update = true)) return
        viewModelScope.launch {
            try {
                reachedStart = messageRepository.loadOlder(chatId)
                visibleLimit.value += PAGE_SIZE
            } finally {
                _loadingOlder.value = false
            }
        }
    }

    private fun newestInboundServerId(list: List<ChatListItem>): Long? {
        val me = myUserId.value
        return list.asSequence()
            .filterIsInstance<ChatListItem.MessageItem>()
            .firstOrNull { it.entity.senderId != me && it.entity.serverId != null }
            ?.entity?.serverId
    }

    companion object {
        const val INITIAL_LIMIT = 100
        const val PAGE_SIZE = 50
        const val READ_DEBOUNCE_MS = 500L
        const val TYPING_THROTTLE_MS = 3_000L
        const val TYPING_EXPIRY_MS = 5_000L
    }
}
