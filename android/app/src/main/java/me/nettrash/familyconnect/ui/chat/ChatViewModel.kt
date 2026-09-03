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
 *   read markers — read means SEEN: the newest inbound serverId is
 *                  reported only while the screen is RESUMED, the list
 *                  is parked at the newest message, *and* the screen
 *                  has SETTLED, after a 500 ms debounce (collectLatest
 *                  + delay) so skimming past a hundred messages
 *                  produces one `read`, not a hundred. All three gates
 *                  are load-bearing — the server's marker is monotonic,
 *                  so a read posted for a message nobody looked at is a
 *                  badge that never comes back, on every device this
 *                  person owns.
 *   settled      — the screen has finished OPENING. An empty list has
 *                  firstVisibleItemIndex == 0, so the screen reports
 *                  "at newest" on its very first frame; without this
 *                  third gate any opening scroll that takes longer than
 *                  the debounce marks the whole chat read before the
 *                  reader has seen a thing. iOS has had the same flag
 *                  (`hasSettled`) for exactly this. It lives HERE
 *                  rather than in the screen so it is testable without
 *                  Compose, and it is one-way: nothing ever unsettles.
 *   open anchor  — a chat with unread messages opens at the OLDEST of
 *                  them, under a "N new messages" divider. Decided ONCE
 *                  from the chat row and the cache as they stood at
 *                  open (see OpenAnchor.kt) — the count is zeroed and
 *                  the marker advances the moment the reader reaches
 *                  the bottom, so anything recomputed would delete the
 *                  divider out from under them.
 *   typing       — outbound throttled to one frame per 3 s (matching
 *                  the server's own per-chat throttle); inbound shown
 *                  for 5 s past the last frame (collectLatest restarts
 *                  the expiry timer).
 *   open chat    — registered with ChatRepository while resumed, with
 *                  the same at-newest signal, so an inbound message for
 *                  THIS chat skips the unread bump only when it actually
 *                  lands in front of the reader.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Chat/ChatViewModel.swift
 */

package me.nettrash.familyconnect.ui.chat

import dagger.hilt.android.qualifiers.ApplicationContext
import me.nettrash.familyconnect.R
import android.net.Uri
import androidx.core.content.FileProvider
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.getAndUpdate
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.mapLatest
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.calls.CallStarter
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.net.LinkPreviewRepository
import me.nettrash.familyconnect.data.net.LinkPreviewState
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.push.PushNotifications
import me.nettrash.familyconnect.data.repo.ChatRepository
import android.content.Context
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.data.repo.GallerySaver
import me.nettrash.familyconnect.data.repo.VoiceRecorder
import kotlinx.coroutines.Job
import me.nettrash.familyconnect.data.repo.LocationProvider
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.MessageBody
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.data.repo.PastedMedia
import me.nettrash.familyconnect.data.repo.ShareStash
import android.content.ClipData
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.Clock
import me.nettrash.familyconnect.util.resolvedDisplayNames
import java.io.File
import javax.inject.Inject
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.net.ApiResult

@OptIn(ExperimentalCoroutinesApi::class)
@HiltViewModel
class ChatViewModel @Inject constructor(
    /**
     * The application context, for `getString` only.
     *
     * A ViewModel holding a Context is usually a smell; the APPLICATION
     * context is the exception — it outlives every screen, so there is
     * nothing to leak. The alternative, carrying @StringRes ids through
     * every state field, spreads resource plumbing across code whose job
     * is state. The trade-off: a message is resolved when it is produced
     * rather than when it is drawn, so one already on screen keeps its
     * language if the system locale changes underneath it — and Android
     * recreates the activity then anyway.
     */
    @param:ApplicationContext private val appContext: Context,
    savedStateHandle: SavedStateHandle,
    private val messageRepository: MessageRepository,
    private val chatRepository: ChatRepository,
    private val familyRepository: FamilyRepository,
    private val settings: SettingsRepository,
    private val socket: ChatSocket,
    private val clock: Clock,
    private val linkPreviewRepository: LinkPreviewRepository,
    private val mediaPrep: MediaPrep,
    private val voiceRecorder: VoiceRecorder,
    private val attachmentApi: AttachmentApi,
    private val attachments: AttachmentRepository,
    private val gallerySaver: GallerySaver,
    private val locationProvider: LocationProvider,
    @param:AppScope private val appScope: CoroutineScope,
    memberDao: MemberDao,
    connectivity: ConnectivityObserver,
    /**
     * Defaulted so the tests can build the ViewModel by hand; Dagger
     * ignores the default and injects CallManager (the same trick
     * SessionRepository plays with its push-token repository).
     */
    private val callStarter: CallStarter = CallStarter { _, _, _ -> false },
    /**
     * Where an OS share parks what it prepared until this chat's
     * composer collects it. Defaulted with the same trick as
     * [callStarter]: the tests never share, and Dagger injects the
     * app-wide singleton the share flow deposited into.
     */
    private val shareStash: ShareStash = ShareStash(),
) : ViewModel() {

    val chatId: Long = checkNotNull(savedStateHandle["chatId"]) { "chatId nav arg missing" }

    // Window into the message table; loadOlder widens it.
    private val visibleLimit = MutableStateFlow(INITIAL_LIMIT)
    private val _loadingOlder = MutableStateFlow(false)

    /** True while an older history page is in flight — drives the list's oldest-end spinner. */
    val loadingOlder: StateFlow<Boolean> = _loadingOlder

    private var reachedStart = false

    private val resumed = MutableStateFlow(false)

    // Written by the screen (see [setAtNewest]); the second half of
    // "read means seen".
    private val atNewest = MutableStateFlow(false)

    // The third half of it: the screen has finished opening (see
    // [setSettled]). False until the screen says so, because an empty
    // LazyColumn reports firstVisibleItemIndex == 0 — i.e. "at the
    // newest message" — on the first frame of EVERY chat, unread or
    // not.
    private val _settled = MutableStateFlow(false)

    /**
     * Whether the opening scroll is done and the list's position means
     * something.
     *
     * Public because the screen consults it too: it is what stops a
     * rotation from re-anchoring a reader who has already scrolled
     * away, and what suppresses the near-old-end pagination trigger
     * during the opening window.
     */
    val settled: StateFlow<Boolean> = _settled

    // Eagerly shared (not WhileSubscribed): the read-marker collector
    // reads myUserId `.value` to tell inbound from my own — a lazily
    // started StateFlow would hand it a stale null until the screen
    // happens to subscribe, and every message would look inbound. `chat`
    // keeps the same treatment: it feeds `items`, which the collector is
    // built on.
    val chat: StateFlow<ChatEntity?> = chatRepository.observeChat(chatId)
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    val myUserId: StateFlow<Long?> = settings.state.map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /**
     * Whether the server signals voice calls (`GET /me` → calls_enabled).
     * The call button is drawn behind this rather than letting somebody
     * find out at the moment they want to talk.
     */
    val callsEnabled: StateFlow<Boolean> = settings.state.map { it.callsEnabled }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

    /**
     * One-shot, already-localised messages for the screen to toast.
     *
     * A SharedFlow with no replay: these are transient reports about an
     * action that has just failed, and a replayed one would fire again on
     * every recomposition after a rotation.
     */
    private val _transientMessages = MutableSharedFlow<String>(extraBufferCapacity = 4)
    val transientMessages: SharedFlow<String> = _transientMessages

    /**
     * Everybody this reader has blocked, for the menu's Block/Unblock row.
     * The message LIST does not need this — `buildChatItems` already folds
     * it into each item — but the menu asks about one sender at a time.
     */
    val blockedUserIds: StateFlow<Set<Long>> = settings.state.map { it.blockedUserIds }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    /** The operator's published contact, for the report sheet. */
    val supportContact: StateFlow<String?> = settings.state.map { it.supportContact }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * Block or unblock one member.
     *
     * The request first, then the local write — never optimistic (see
     * FamilyRepository.block). A failure surfaces as an error rather than
     * silently hiding rows the reader does not know are hidden.
     */
    fun setBlocked(userId: Long, blocked: Boolean) {
        viewModelScope.launch {
            val result = if (blocked) {
                familyRepository.block(userId)
            } else {
                familyRepository.unblock(userId)
            }
            if (result !is ApiResult.Ok<*>) {
                _transientMessages.tryEmit(appContext.getString(R.string.e_block_failed))
            }
        }
    }

    /**
     * Report a member, optionally naming one of their messages.
     *
     * Raising a report that matches an OPEN one returns that row and
     * creates nothing, so a double tap is not two rows in the owner's
     * list — which is why this needs no local de-duplication.
     */
    fun report(reportedUserId: Long, reason: String, messageId: Long?, onDone: () -> Unit) {
        viewModelScope.launch {
            val result = familyRepository.report(reportedUserId, reason, messageId)
            if (result is ApiResult.Ok<*>) {
                onDone()
            } else {
                _transientMessages.tryEmit(appContext.getString(R.string.e_report_failed))
            }
        }
    }

    /**
     * Whether it also allows VIDEO calls (`GET /me` → video_calls_enabled,
     * docs/protocol.md, "Video") — gates the video-call button alone.
     */
    val videoCallsEnabled: StateFlow<Boolean> = settings.state.map { it.videoCallsEnabled }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

    /**
     * Ring the other person in this direct chat. The screen has already
     * secured the microphone permission (and asked for the camera when
     * [video] — a denied camera still places the call, camera off;
     * docs/protocol.md, "Video"). False when this device is on a call
     * already, or this is not a direct chat.
     */
    fun startCall(video: Boolean): Boolean {
        val current = chat.value ?: return false
        val peer = current.peerUserId ?: return false
        if (current.kind != "direct") return false
        return callStarter.startCall(current.id, peer, video)
    }

    // Roster snapshot — sender names in family bubbles, the typing
    // indicator, and the who-reacted popup all resolve through it.
    // Eagerly shared: typing frames can arrive before the items flow has
    // any subscriber.
    val memberNames: StateFlow<Map<Long, String>> = memberDao.observeMembers()
        // The FULL roster, tombstones included — a bubble from somebody
        // whose account is gone still has to say who wrote it. Their
        // stored name is the server's English placeholder, so the map is
        // built through resolvedDisplayNames rather than off displayName
        // (docs/protocol.md, "Deleting an account").
        .map { members -> members.resolvedDisplayNames(appContext) }
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyMap())

    /**
     * How many people are actually in the family right now — the
     * denominator of a poll's "3 of 5 voted" footer.
     *
     * The ACTIVE roster, so somebody who left and somebody whose account
     * is gone are not counted among those who have yet to answer. Eager,
     * like the name map it sits beside: `items` is built on it, and a
     * lazily started flow would hand the first build a zero.
     */
    val familyMemberCount: StateFlow<Int> = memberDao.observeActiveMembers()
        .map { it.size }
        .stateIn(viewModelScope, SharingStarted.Eagerly, 0)

    /** userId → profile-picture version, for the avatars beside reactors. */
    val memberAvatars: StateFlow<Map<Long, Long>> = memberDao.observeMembers()
        .map { members -> members.associate { it.userId to it.avatarVersion } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

    // UI-only: flips on the first items emission, so the empty state can
    // tell an actually-empty chat from "the DB flow hasn't answered yet".
    private val _initialLoadSettled = MutableStateFlow(false)

    /** True once the first (possibly empty) items emission has landed. */
    val initialLoadSettled: StateFlow<Boolean> = _initialLoadSettled

    /**
     * Message ids the assistant is still writing into.
     *
     * Held in memory only, like iOS and macOS: a row that was mid-stream
     * when the app was killed must not come back looking live. Exposed
     * because the BUBBLE needs it — until now the repository published this
     * and no UI read it, so an assistant placeholder rendered as a
     * completely blank balloon for the whole latency of the call. In the
     * family chat that blank balloon is visible to everyone, not just the
     * person who asked.
     */
    val streamingMessageIds: StateFlow<Set<Long>> = messageRepository.streamingMessageIds

    /**
     * The assistant's reserved account id, or null when the server has none.
     * Null is the capability check: a composer that offered `@ai` against a
     * server without an assistant would offer an affordance that silently
     * does nothing.
     */
    val assistantUserId: StateFlow<Long?> = settings.state
        .map { it.assistantUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    // Where the chat opens, and what the divider says. Null while the
    // decision has not been made yet — which is also why the screen
    // must not act on it until it is non-null, and why nothing else
    // ever writes it (see the init block).
    private val _openAnchor = MutableStateFlow<OpenAnchor?>(null)

    /**
     * Where this chat opens: at the newest message, or anchored at the
     * oldest one the reader has not seen.
     *
     * Null means "not decided yet" and is NOT the same as
     * [OpenAnchor.Newest]: the screen settles on the decision, so acting
     * on a null would settle it before the anchor exists — which is the
     * data-losing race [settled] is here to close.
     */
    val openAnchor: StateFlow<OpenAnchor?> = _openAnchor

    val items: StateFlow<List<ChatListItem>> = combine(
        visibleLimit.flatMapLatest { messageRepository.observeMessages(chatId, it) },
        chat,
        settings.state,
        memberNames,
        // Paired because combine tops out at five flows, and because
        // these two are the only ones that are not a message: the
        // roster's size and the anchor captured at open.
        combine(familyMemberCount, _openAnchor) { memberCount, anchor -> memberCount to anchor },
    ) { messages, chatEntity, settingsState, members, memberCountAndAnchor ->
        val (memberCount, anchor) = memberCountAndAnchor
        buildChatItems(
            messagesNewestFirst = messages,
            isFamilyChat = chatEntity?.kind == "family",
            myUserId = settingsState.myUserId ?: -1L,
            memberNames = members,
            nowMillis = clock.now(),
            assistantUserId = settingsState.assistantUserId,
            assistantName = settingsState.assistantName,
            familyMemberCount = memberCount,
            firstUnreadServerId = (anchor as? OpenAnchor.Message)?.serverId,
            newMessageCount = (anchor as? OpenAnchor.Message)?.newCount ?: 0,
            // Rides in on `settings.state`, which is already the third
            // flow here — the combine is at its five-flow ceiling.
            blockedUserIds = settingsState.blockedUserIds,
        )
    }
        .onEach { _initialLoadSettled.value = true }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /**
     * The input field's text, owned as TextFieldState rather than a
     * StateFlow<String>: the field edits this buffer synchronously — it
     * is the same buffer the IME talks to — so the programmatic clear in
     * [send] cannot race a late IME event resurrecting the sent text,
     * which is the documented failure mode of driving a TextField
     * through a value/onValueChange round-trip over an async flow.
     */
    val inputState = TextFieldState()

    /**
     * The message being answered, while the composer is primed. Lives here
     * rather than in the screen so it survives a configuration change with
     * the draft text it belongs to.
     */
    private val _replyDraft = MutableStateFlow<ReplyToDto?>(null)
    val replyDraft: StateFlow<ReplyToDto?> = _replyDraft

    fun beginReply(quote: ReplyToDto) {
        _replyDraft.value = quote
    }

    fun cancelReply() {
        _replyDraft.value = null
    }

    /**
     * The message being rewritten, while the composer is in edit mode,
     * with the draft it displaced. Mutually exclusive with [replyDraft]:
     * you are either answering a message or rewriting one.
     */
    private val _editTarget = MutableStateFlow<EditTarget?>(null)
    val editTarget: StateFlow<EditTarget?> = _editTarget

    data class EditTarget(val messageId: Long, val displacedDraft: String)

    fun beginEdit(messageId: Long, body: String) {
        _replyDraft.value = null
        _editTarget.value = EditTarget(messageId, inputState.text.toString())
        inputState.setTextAndPlaceCursorAtEnd(body)
    }

    /// Give the composer back exactly as it was borrowed.
    fun cancelEdit() {
        val displaced = _editTarget.value?.displacedDraft.orEmpty()
        _editTarget.value = null
        inputState.setTextAndPlaceCursorAtEnd(displaced)
    }

    /**
     * The poll being written, while the composer's poll sheet is open.
     *
     * Here rather than in the screen for the same reason [replyDraft] is:
     * a rotation must not throw away a half-written poll. Null means the
     * sheet is closed — there is no such thing as a draft with no sheet.
     */
    private val _pollDraft = MutableStateFlow<PollDraft?>(null)
    val pollDraft: StateFlow<PollDraft?> = _pollDraft

    /**
     * Whether this chat may hold a poll at all.
     *
     * The family chat only: a poll is a family deciding something
     * together, and anywhere else the server answers `invalid_poll`
     * (docs/protocol.md, "Polls"). The attach menu hides the item rather
     * than offering an affordance that can only fail.
     */
    val canCreatePoll: StateFlow<Boolean> = chat
        .map { it?.kind == "family" }
        .stateIn(viewModelScope, SharingStarted.Eagerly, false)

    // -- Pictures -----------------------------------------------------------------

    /**
     * Whether this composer may offer to show the assistant a picture.
     *
     * TWO locks, and both have to be open before a single pixel can leave
     * (docs/protocol.md, "Pictures"): the operator has configured a
     * deployment that can SEE (`assistant.vision`), and the family's
     * OWNER has turned `ai_vision` on — which is false by default, the
     * deliberate opposite of `ai_history`. Neither is consent for a
     * particular photograph: that is a third thing, the member attaching
     * it to the question, and it is never a remembered setting.
     *
     * The member's OWN `ai` chat and nowhere else — and that is a fact
     * about this DOOR, the "Show the assistant a picture" item, not about
     * what travels. Since #56 a photo on an `@ai` message in the family
     * chat, or on the message it replies to, goes to the model under the
     * same two locks (docs/protocol.md, "Showing the assistant a picture
     * from the family chat"); it rides the ordinary "Photo or video" door
     * and the reply affordance the family composer has always had, and
     * either is the member pointing the assistant at that picture. So
     * this stays false in the family chat even with both locks open, for
     * a narrower reason than it used to: a second item there would offer
     * nothing the first does not. What still never goes is a photo the
     * member did not point at — somebody else's picture elsewhere in the
     * window stays `[photo]`.
     */
    val canShowAssistantPicture: StateFlow<Boolean> =
        combine(chat, settings.state) { chatEntity, settingsState ->
            chatEntity?.kind == "ai" &&
                settingsState.assistantVision &&
                settingsState.familyAiVision
        }.stateIn(viewModelScope, SharingStarted.Eagerly, false)

    /**
     * Whether this composer may offer `/draw`.
     *
     * One lock and no family switch, because what leaves the server on a
     * picture request is strictly SMALLER than what an ordinary text
     * question sends: the words after the token and nothing else — not
     * the thread, not the transcript, not the system prompt, not the
     * family's language, not the member's name, and not any picture the
     * message carries (docs/protocol.md, "Pictures").
     *
     * Both surfaces take it: the member's own `ai` chat, and the family
     * chat, where the whole family sees the answer arrive. Never a direct
     * chat, which the assistant is not in. And never on a server with no
     * images deployment: `/draw` is just text there, answered in words,
     * so the affordance would be one that silently does nothing.
     */
    val canAskForPicture: StateFlow<Boolean> =
        combine(chat, settings.state) { chatEntity, settingsState ->
            settingsState.assistantImages &&
                when (chatEntity?.kind) {
                    "ai" -> true
                    "family" -> settingsState.assistantUserId != null
                    else -> false
                }
        }.stateIn(viewModelScope, SharingStarted.Eagerly, false)

    /**
     * Turn whatever is in the composer into a picture request.
     *
     * The token cannot be appended the way `@ai` is — it has to be FIRST,
     * and in the family chat it has to sit after one leading mention — so
     * the whole body is rebuilt by the shared grammar rather than
     * assembled here. Pressing it twice is a no-op: a body that already
     * asks for a picture comes back untouched.
     */
    fun insertDrawToken() {
        if (!canAskForPicture.value) return
        if (_editTarget.value != null) return
        val inFamilyChat = chat.value?.kind == "family"
        val rewritten = AssistantMention.withDraw(inputState.text.toString(), inFamilyChat)
        inputState.setTextAndPlaceCursorAtEnd(rewritten)
    }

    /**
     * Assistant replies an `ai_error` frame named, by server id.
     *
     * The bubble needs it: a picture answer that failed has an empty row
     * and no deltas ever arrived, so without this it is a blank balloon
     * that never resolves (docs/protocol.md, "Pictures").
     */
    val failedAssistantMessageIds: StateFlow<Set<Long>> =
        messageRepository.failedAssistantMessageIds

    /** Open the poll sheet on a fresh draft — two empty options, no question. */
    fun beginPoll() {
        if (!canCreatePoll.value) return
        _pollDraft.value = PollDraft()
    }

    /** Close the sheet and throw the draft away. */
    fun cancelPoll() {
        _pollDraft.value = null
    }

    fun setPollQuestion(text: String) {
        _pollDraft.update { it?.withQuestion(text) }
    }

    fun setPollOption(index: Int, text: String) {
        _pollDraft.update { it?.withOption(index, text) }
    }

    fun addPollOption() {
        _pollDraft.update { it?.plusOption() }
    }

    fun removePollOption(index: Int) {
        _pollDraft.update { it?.minusOption(index) }
    }

    /**
     * Post the draft as an ordinary message whose body is the question.
     *
     * Takes the primed reply with it and clears it, exactly as [send]
     * does — a quote belongs to the message being sent, and leaving it
     * armed would silently quote the next one too. The sheet closes
     * immediately: the send is optimistic, so the bubble is already
     * there to look at.
     */
    fun sendPoll() {
        val draft = _pollDraft.value ?: return
        if (!draft.isValid) return
        val quote = _replyDraft.value
        _replyDraft.value = null
        _pollDraft.value = null
        viewModelScope.launch {
            messageRepository.sendPoll(chatId, draft.question, draft.sendableOptions, quote)
        }
    }

    /**
     * Tap on a poll option: the repository decides set vs clear from the
     * row's current state. Only acked messages (serverId != null) can be
     * voted on — the UI gates on that, exactly as it does for reactions.
     */
    fun vote(messageServerId: Long, optionId: Long) {
        viewModelScope.launch { messageRepository.toggleVote(chatId, messageServerId, optionId) }
    }

    /**
     * Close a poll. The author's, and one-way. A refusal reports through
     * the composer's strip — the one place this screen already says
     * something did not work.
     */
    fun closePoll(messageServerId: Long) {
        viewModelScope.launch {
            if (!messageRepository.closePoll(chatId, messageServerId)) {
                _mediaState.value =
                    MediaSendState.Failed(appContext.getString(R.string.e_close_poll_failed))
            }
        }
    }

    private val _typingUser = MutableStateFlow<String?>(null)

    /** Display name of the member typing right now (5 s expiry). */
    val typingUser: StateFlow<String?> = _typingUser

    val isOnline: StateFlow<Boolean> = connectivity.isOnline
    val socketState: StateFlow<SocketState> = socket.state

    /** Every known link's preview state, keyed by URL. */
    val linkPreviews: StateFlow<Map<String, LinkPreviewState>> = linkPreviewRepository.states

    /**
     * Whether bubbles may show link previews at all — off means this
     * device never requests a linked page (see LinkPreviewRepository).
     */
    val linkPreviewsEnabled: StateFlow<Boolean> = settings.state
        .map { it.linkPreviewsEnabled }
        .stateIn(viewModelScope, SharingStarted.Eagerly, true)

    /** Whether a shared location draws a map — drawing one asks Google. */
    val mapPreviewsEnabled: StateFlow<Boolean> = settings.state
        .map { it.mapPreviewsEnabled }
        .stateIn(viewModelScope, SharingStarted.Eagerly, true)

    /** Composition asks for a link the first time a bubble renders it. */
    fun requestLinkPreview(url: String) {
        if (!linkPreviewsEnabled.value) return
        linkPreviewRepository.request(url)
    }

    private var lastTypingSentAt = 0L

    init {
        // WHERE THIS CHAT OPENS. Decided exactly once, from the chat row
        // and the cache as they stand the moment the screen appears, and
        // never revisited: reaching the bottom zeroes the count and
        // advances the marker, so an anchor derived on every pass would
        // erase itself the instant it worked.
        //
        // The wait is on the FIRST items emission rather than on a
        // timer: it proves both halves have answered — the message flow,
        // and settings.state (items combines it), which is where
        // myUserId comes from. The read collector below subscribes to
        // `items` from this same scope, so that emission always arrives,
        // screen or no screen.
        //
        // The read path cannot race this: it needs [settled], and the
        // screen only settles on a decision that is already made.
        viewModelScope.launch {
            _initialLoadSettled.first { it }
            val chatRow = chatRepository.chatSnapshot(chatId)
            _openAnchor.value = openAnchor(
                unreadCount = chatRow?.unreadCount ?: 0,
                myLastReadId = chatRow?.myLastReadId ?: 0L,
                // Read further back than the render window, because the
                // anchor may sit behind it — the screen's bounded page
                // loop is what brings the window out to meet it, and
                // ANCHOR_CAP is exactly how far that loop can go.
                cachedNewestFirst = messageRepository.anchorRows(chatId, ANCHOR_CAP),
                myUserId = myUserId.value ?: -1L,
                cap = ANCHOR_CAP,
            )
        }

        // Read markers: newest inbound acked message, gated on RESUMED,
        // on the list being at the newest message, *and* on the screen
        // having settled, debounced
        // 500 ms. collectLatest restarts the debounce whenever any input
        // changes — the report fires once things settle. Null means "not
        // reading right now", which is also what stops a scroll back up
        // the thread from re-reporting on the way past.
        //
        // No `newest > myLastReadId` guard here: postRead applies the
        // monotonic rule itself, and it has to clear the local badge
        // BEFORE applying it. Short-circuiting on the marker in this
        // collector instead would leave a chat whose marker is already
        // ahead of the server (a read that never landed) showing a count
        // that opening it never clears.
        viewModelScope.launch {
            combine(resumed, atNewest, _settled, items) { isResumed, isAtNewest, isSettled, list ->
                // isSettled is the one that closes the opening race: an
                // empty list reports index 0 — "at the newest message" —
                // on the first frame, so a chat that opens anchored
                // thirty messages up would post a read for the NEWEST id
                // as soon as the debounce elapsed, and the server's
                // marker never comes back down on any of this person's
                // devices.
                if (!isResumed || !isAtNewest || !isSettled) null else newestInboundServerId(list)
            }
                .distinctUntilChanged()
                .collectLatest { newest ->
                    if (newest == null) return@collectLatest
                    delay(READ_DEBOUNCE_MS)
                    chatRepository.postRead(chatId, newest)
                    // The chat is read, so the tray entry about it is
                    // stale — and on Android the tray entry IS the
                    // launcher dot, which would otherwise stay lit until
                    // the user went and tapped a notification for
                    // messages they have already read.
                    PushNotifications.cancelChat(appContext, chatId)
                }
        }

        // Outbound typing, throttled to the server's own 1-per-3s limit —
        // anything faster would be dropped server-side anyway. Watches
        // the field's snapshot state directly (there is no
        // onValueChange to hook since the input became TextFieldState).
        viewModelScope.launch {
            snapshotFlow { inputState.text.toString() }.collect { value ->
                val now = clock.now()
                if (value.isNotBlank() && now - lastTypingSentAt >= TYPING_THROTTLE_MS) {
                    if (socket.trySend(ClientFrame.Typing(chatId))) {
                        lastTypingSentAt = now
                    }
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

        // An OS share aimed at THIS chat: drain the stash into the
        // composer — items land STAGED and words land in the field,
        // nothing auto-sends. The claim is target-checked, so a chat
        // opened by any other route finds nothing here (mirrors iOS's
        // pendingShareImport).
        viewModelScope.launch {
            val share = shareStash.claim(chatId) ?: return@launch
            share.items.forEach { stage(it) }
            share.text?.let { appendPasted(it) }
        }
    }

    /** What the composer is doing with a picked photo or video. */
    sealed interface MediaSendState {
        data object Idle : MediaSendState
        data object Preparing : MediaSendState
        data object Uploading : MediaSendState

        /**
         * A download the user asked for (share, open), with its own
         * wording — the same strip reports it, since it is the one place
         * this screen already says what it is busy with.
         */
        data class Working(val label: String) : MediaSendState
        data class Failed(val reason: String) : MediaSendState

        /**
         * Whether the composer is occupied with something a new
         * attachment would collide with.
         *
         * [Failed] is deliberately NOT busy: it is a sentence sitting in
         * the strip until somebody dismisses it, and treating it as busy
         * is what made an error from one paste block the next one — the
         * attach button greyed out, and the field's own paste refusing,
         * for no reason the user could see.
         */
        val isBusy: Boolean
            get() = this is Preparing || this is Uploading || this is Working
    }

    private val _mediaState = MutableStateFlow<MediaSendState>(MediaSendState.Idle)

    /** Drives the input bar's busy strip. */
    val mediaState: StateFlow<MediaSendState> = _mediaState

    /**
     * Media prepared and waiting for Send — up to
     * [AttachmentDto.MAX_PER_MESSAGE] of them, in the order they were
     * added, which is the order the message will carry them
     * (docs/protocol.md, "Photos, videos, audio, files and locations").
     */
    private val _staged = MutableStateFlow<List<MediaPrep.Prepared>>(emptyList())
    val staged: StateFlow<List<MediaPrep.Prepared>> = _staged

    /**
     * Whether a picture is staged right now — which, in an `ai` chat, is
     * the moment the disclosure has to be on screen.
     *
     * The switch lives on a settings screen somebody read once; the
     * photograph is chosen in a composer, later, by someone who may not
     * have been the one who read it. So the notice hangs off the STAGED
     * items rather than off the door they came through — the picker, a
     * paste, a drop and the camera all end up here.
     *
     * Started EAGERLY, like the two capability flags it sits beside: a
     * lazily started flow would hand the composer a null on the frame a
     * photo is staged and the sentence one frame later, so the disclosure
     * would appear AFTER the thumbnail it is about. It has to be there
     * when the picture is.
     */
    val assistantPictureNotice: StateFlow<AiPictureNotice?> =
        combine(chat, staged, canShowAssistantPicture) { chatEntity, items, allowed ->
            if (chatEntity?.kind != "ai") {
                null
            } else {
                AiPictureNotice.of(
                    items.map { item ->
                        // What the item will BE on the wire, not what was
                        // picked: the server prefers the preview, so the
                        // preview's type and length are what its rule
                        // reads (docs/protocol.md, "Pictures").
                        StagedPicture(
                            kind = item.kind,
                            mime = AiPictureNotice.wireMime(
                                item.mime, hasPreview = item.previewJpeg != null),
                            bytes = AiPictureNotice.wireBytes(
                                item.previewJpeg?.size) { item.file.length() },
                        )
                    },
                    allowed,
                )
            }
        }.stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /**
     * The draft as typed, as a flow — the field is a [TextFieldState], so
     * this is the same snapshot watch the typing indicator keeps, shared
     * here so the family composer's strip can read the draft for `@ai`.
     */
    private val draftText: StateFlow<String> =
        snapshotFlow { inputState.text.toString() }
            .stateIn(viewModelScope, SharingStarted.Eagerly, "")

    /**
     * What the message being replied to carries, looked up once per reply
     * target — from this device's own rows, because a [ReplyToDto] holds
     * an excerpt and nothing else. Empty while nothing is primed.
     */
    private val quotedAttachments: StateFlow<List<AttachmentDto>> =
        _replyDraft
            .mapLatest { quote -> quote?.let { messageRepository.attachmentsOf(it.messageId) } ?: emptyList() }
            .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    /**
     * The FAMILY composer's own disclosure, for the case that did not
     * exist before #56: an `@ai` draft with a photo staged on it, or
     * replying to a message that carries one, is about to send that photo
     * to the model under the same two locks a private question needs
     * (docs/protocol.md, "Showing the assistant a picture from the family
     * chat" — "What a client's family-chat composer must say"). Null in
     * every other chat and whenever there is nothing to say; the rule is
     * [AiPictureNotice.forMention], pinned by its own tests, and the
     * counting is the server's.
     *
     * With the owner's third switch on (`ai_history_photos`, and
     * `ai_history` with it) the same rule also says that the chat's most
     * recent photos may go — "up to N", N being what the draft and the
     * quote left of the four — and the strip then shows for an `@ai`
     * draft with no photo of its own at all, since that is the mention on
     * which every one of the four may be somebody else's picture
     * (docs/protocol.md, "Recent photos from the family chat").
     *
     * Eager, for [assistantPictureNotice]'s reason: the line has to be
     * there on the frame the photo is, not one frame later.
     */
    val mentionPictureNotice: StateFlow<MentionPictureNotice?> =
        combine(chat, draftText, staged, quotedAttachments, settings.state) {
                chatEntity, draft, items, quoted, settingsState ->
            if (chatEntity?.kind != "family") {
                null
            } else {
                AiPictureNotice.forMention(
                    draft = draft,
                    staged = items.map { item ->
                        StagedPicture(
                            kind = item.kind,
                            mime = AiPictureNotice.wireMime(
                                item.mime, hasPreview = item.previewJpeg != null),
                            bytes = AiPictureNotice.wireBytes(
                                item.previewJpeg?.size) { item.file.length() },
                        )
                    },
                    quoted = quoted.map(StagedPicture::of),
                    allowed = settingsState.assistantVision && settingsState.familyAiVision,
                    serverCanDraw = settingsState.assistantImages,
                    familyHistory = settingsState.familyAiHistory,
                    familyHistoryPhotos = settingsState.familyAiHistoryPhotos,
                )
            }
        }.stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /** Screen calls this from a LifecycleResumeEffect. */
    fun setResumed(isResumed: Boolean) {
        resumed.value = isResumed
        publishOpenChat()
    }

    /**
     * Screen reports whether the list is parked at the newest message
     * (`firstVisibleItemIndex <= 1` on the reverseLayout thread).
     *
     * The ViewModel cannot derive this — only the LazyListState knows —
     * and it defaults to false on purpose: until the screen has said
     * otherwise, nothing is known to be on screen, and the cost of
     * guessing wrong in that direction is a read that can never be
     * undone.
     */
    fun setAtNewest(value: Boolean) {
        atNewest.value = value
        publishOpenChat()
    }

    /**
     * Screen reports that it has finished opening — the end of BOTH
     * opening branches, the anchored scroll and the plain
     * open-at-the-newest one.
     *
     * One-way, and deliberately so: this is not "the list is idle", it
     * is "the list's position now means what it says". Nothing ever
     * unsettles a screen, because nothing after the open can put the
     * list back into a state where index 0 is a lie.
     */
    fun setSettled() {
        if (_settled.value) return
        _settled.value = true
        publishOpenChat()
    }

    // Paused means the screen is showing nobody anything, whatever the
    // list is parked on — so the claim drops wholesale rather than
    // leaving a stale "at newest" behind for the bump rule to trust.
    //
    // UNSETTLED publishes "not at newest" for the same reason the read
    // collector refuses to fire: during the opening window nothing is
    // known to be in front of anybody. That is the safe direction — a
    // message arriving mid-open counts as unread, and a message that
    // genuinely landed in front of the reader costs one badge they can
    // clear by reaching the bottom, which they are about to do anyway.
    private fun publishOpenChat() {
        val isResumed = resumed.value
        chatRepository.setOpenChat(
            chatId = if (isResumed) chatId else null,
            atNewest = isResumed && _settled.value && atNewest.value,
        )
    }

    fun send() {
        val body = inputState.text.toString()
        // An attachment can travel with no words at all — that is how a
        // photo is normally sent — so a blank draft only stops the send
        // when there is nothing staged either.
        // Taken ATOMICALLY (one swap): stage() can land concurrently
        // from the appScope prepare loops and the share drain, and a
        // separate read-then-clear would silently drop — and leak the
        // file of — anything staged between the two writes.
        val attachments = _staged.getAndUpdate { emptyList() }
        if (body.isBlank() && attachments.isEmpty()) return
        if (attachments.isNotEmpty()) {
            sendStaged(attachments, body)
            return
        }
        // Edit mode: the composer was borrowed to rewrite an existing
        // message. The field is cleared only once the server takes it —
        // a refused edit leaves the text there to fix, rather than
        // dropping what the user typed.
        val editing = _editTarget.value
        if (editing != null) {
            viewModelScope.launch {
                if (messageRepository.edit(chatId, editing.messageId, body)) {
                    _editTarget.value = null
                    inputState.setTextAndPlaceCursorAtEnd(editing.displacedDraft)
                }
            }
            return
        }
        inputState.clearText()
        // Read and clear together: the draft belongs to the message being
        // sent, and leaving it primed would silently quote the next one too.
        val quote = _replyDraft.value
        _replyDraft.value = null
        viewModelScope.launch { messageRepository.send(chatId, body, quote) }
    }

    /**
     * Prepare and send a picked photo or video.
     *
     * Not optimistic, unlike [send]: the bubble appears once the server
     * has the bytes, so the composer stays visibly busy until then (see
     * MessageRepository.sendMedia). [mediaState] is what the input bar
     * draws while that runs.
     */
    fun stageMedia(uri: Uri, isVideo: Boolean) = stageMedia(listOf(uri to isVideo))

    /**
     * The multi-picker's entry: each (uri, isVideo) prepared and staged
     * in the order it was picked — which is the order the message will
     * carry them.
     */
    fun stageMedia(items: List<Pair<Uri, Boolean>>) {
        if (items.isEmpty()) return
        if (_mediaState.value == MediaSendState.Preparing ||
            _mediaState.value == MediaSendState.Uploading
        ) {
            return
        }
        _mediaState.value = MediaSendState.Preparing
        // APP scope, not viewModelScope: pressing Back or opening another
        // chat clears the ViewModel, and with it went a 90 MB upload —
        // no bubble, no FAILED row to retry, no error. A text message sent
        // at the same moment survives the same navigation, and so must
        // this. The state writes below are harmless once nobody is reading.
        appScope.launch {
            items.forEachIndexed { index, (uri, isVideo) ->
                val prepared = try {
                    if (isVideo) mediaPrep.prepareVideo(uri) else mediaPrep.preparePhoto(uri)
                } catch (_: MediaPrep.TooLargeAfterCompression) {
                    // The one failure the user can act on, so it says what
                    // would help rather than just refusing. What was
                    // already staged stays staged.
                    _mediaState.value = MediaSendState.Failed(
                        appContext.getString(R.string.e_still_too_large),
                    )
                    return@launch
                } catch (_: Exception) {
                    _mediaState.value =
                        MediaSendState.Failed(appContext.getString(R.string.e_prepare_failed))
                    return@launch
                }

                if (!stage(prepared)) return@launch
                // stage() reports Idle so each chip appears as it lands;
                // the strip goes back to busy while more are still coming.
                if (index < items.lastIndex) _mediaState.value = MediaSendState.Preparing
            }
        }
    }

    /**
     * Commit staged media with whatever the composer holds.
     *
     * The composer is taken atomically FIRST — caption, quote and the file
     * together — so nothing typed during the upload is swallowed into the
     * caption and a primed reply cannot leak onto the next message. It all
     * goes back if the send never lands, so it can be retried.
     */
    /** Has the person already allowed location? Decides which of the two
     *  the screen does: ask for permission, or ask for a fix. */
    fun hasLocationPermission(): Boolean = locationProvider.hasPermission()

    /**
     * Share where this device is, once.
     *
     * Take-then-restore, like every other send here: whatever was typed
     * travels with the pin, and comes back if the send never happened. App
     * scope rather than viewModelScope, for the reason `sendStaged` gives —
     * navigating away must not silently take the send with it.
     */
    fun shareLocation() {
        if (_mediaState.value != MediaSendState.Idle) return
        _mediaState.value = MediaSendState.Uploading
        val caption = inputState.text.toString()
        val quote = _replyDraft.value
        inputState.clearText()
        _replyDraft.value = null
        appScope.launch {
            when (val result = locationProvider.currentFix()) {
                is LocationProvider.Result.Found -> {
                    val sent = messageRepository.sendLocation(
                        latitude = result.fix.latitude,
                        longitude = result.fix.longitude,
                        accuracyM = result.fix.accuracyM,
                        label = null,
                        caption = caption,
                        chatId = chatId,
                        replyTo = quote,
                    )
                    if (sent) {
                        _mediaState.value = MediaSendState.Idle
                    } else {
                        restoreComposer(caption, quote)
                        _mediaState.value =
                            MediaSendState.Failed(appContext.getString(R.string.e_send_failed))
                    }
                }
                LocationProvider.Result.Denied -> {
                    restoreComposer(caption, quote)
                    _mediaState.value = MediaSendState.Failed(
                        appContext.getString(R.string.e_location_permission),
                    )
                }
                LocationProvider.Result.Unavailable -> {
                    restoreComposer(caption, quote)
                    _mediaState.value = MediaSendState.Failed(
                        appContext.getString(R.string.e_location_unavailable),
                    )
                }
            }
        }
    }

    /** Put back what a failed send took, without clobbering newer typing. */
    private fun restoreComposer(caption: String, quote: ReplyToDto?) {
        if (inputState.text.isEmpty() && caption.isNotEmpty()) {
            inputState.setTextAndPlaceCursorAtEnd(caption)
        }
        if (_replyDraft.value == null) _replyDraft.value = quote
    }

    private fun sendStaged(prepared: List<MediaPrep.Prepared>, caption: String) {
        val quote = _replyDraft.value
        inputState.clearText()
        _replyDraft.value = null
        // send() already took the staged set atomically; clearing it again
        // here would clobber anything staged in between.
        _mediaState.value = MediaSendState.Uploading
        // App scope, not viewModelScope: navigating away used to take a
        // 90 MB upload with it — no bubble, no FAILED row, no error.
        appScope.launch {
            val sent = messageRepository.sendMedia(
                prepared,
                caption,
                chatId,
                quote,
            ) { index, total ->
                // "Uploading 2 of 5…" — only a set worth counting gets a
                // counter; a single upload keeps the plain busy strip.
                if (total > 1) {
                    _mediaState.value = MediaSendState.Working(
                        appContext.getString(R.string.s_uploading_n_of_m, index, total),
                    )
                }
            }
            if (sent) {
                _mediaState.value = MediaSendState.Idle
            } else {
                _mediaState.value =
                    MediaSendState.Failed(appContext.getString(R.string.e_send_failed))
                // sendMedia deletes an item's prepared file only once
                // ITS upload has landed (iOS parity), so the files still
                // on disk are exactly the unsent tail — put them back in
                // the composer for a one-tap retry. Uploads that landed
                // before the failure are left to the server's 24-hour
                // sweep of unclaimed attachments. PREPENDED atomically:
                // they were staged before anything added mid-upload.
                val remainder = prepared.filter { it.file.exists() }
                if (remainder.isNotEmpty()) {
                    _staged.update { current -> remainder + current }
                }
                if (inputState.text.isEmpty()) {
                    inputState.setTextAndPlaceCursorAtEnd(caption)
                }
                if (_replyDraft.value == null) _replyDraft.value = quote
            }
        }
    }

    /**
     * Hold prepared media in the composer until the user presses Send.
     *
     * Picking used to send immediately, so a caption had to be typed
     * BEFORE choosing the photo and there was no way to back out once
     * picked. A message now carries up to
     * [AttachmentDto.MAX_PER_MESSAGE] attachments, so staging APPENDS —
     * the old one-per-message discard-first rule died with plurality —
     * and at the cap the new item is refused with a notice rather than
     * silently replacing anything.
     */
    /**
     * Test seam: staging normally follows a real pick + prepare, which a
     * unit test cannot drive. The app never calls this.
     */
    fun stagePrepared(prepared: MediaPrep.Prepared) = stage(prepared)

    /** True when the item was taken; false (file deleted, notice shown) at the cap. */
    private fun stage(prepared: MediaPrep.Prepared): Boolean {
        // CAS loop, not a plain read-modify-write: stage() runs on the
        // appScope prepare loops (a Default-pool thread each), the share
        // drain and the main thread at once, and two racing
        // `value = value + x` writes can lose an item — whose file then
        // leaks, because only staged items are ever cleaned up. The cap
        // check sits INSIDE the loop so refusal and append decide
        // against the same list.
        while (true) {
            val current = _staged.value
            if (current.size >= AttachmentDto.MAX_PER_MESSAGE) {
                prepared.file.delete()
                _mediaState.value = MediaSendState.Failed(
                    appContext.getString(
                        R.string.e_attachment_limit,
                        AttachmentDto.MAX_PER_MESSAGE,
                    ),
                )
                return false
            }
            if (_staged.compareAndSet(current, current + prepared)) {
                _mediaState.value = MediaSendState.Idle
                return true
            }
        }
    }

    /**
     * A destination for the camera to write into, as a FileProvider Uri.
     *
     * The capture intents write into a Uri the CALLER provides — the
     * capture itself still happens in the camera app. But the manifest now
     * DECLARES `android.permission.CAMERA` (video calls), and the capture
     * intents throw a SecurityException for an app that declares the
     * permission without HOLDING it — so the screen gates the hand-off on
     * the runtime grant first (see CaptureGate).
     *
     * Returns null if the directory cannot be made, which is the only
     * failure worth reporting here; the launcher simply does not start.
     */
    fun newCaptureUri(context: Context, isVideo: Boolean): Uri? {
        return try {
            val dir = File(context.cacheDir, "captures").apply { mkdirs() }
            val suffix = if (isVideo) ".mp4" else ".jpg"
            val file = File.createTempFile("capture-", suffix, dir)
            FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
        } catch (_: Exception) {
            null
        }
    }

    /** Throw away ONE staged item and the temp file nothing else will clean up. */
    fun discardStaged(index: Int) {
        // CAS: a concurrent stage() append must not be clobbered by this
        // write (nor vice versa). The victim is resolved against the SAME
        // list the swap replaces, and its file is deleted only after the
        // swap took — never the file of an item that stays visible.
        while (true) {
            val current = _staged.value
            val victim = current.getOrNull(index) ?: return
            if (_staged.compareAndSet(current, current.filterIndexed { i, _ -> i != index })) {
                victim.file.delete()
                return
            }
        }
    }

    /**
     * Where the video player streams from, with the auth header it needs.
     * Suspending because the token is a stored read.
     */
    suspend fun attachmentStreamUrl(attachmentId: Long): Pair<String, Map<String, String>>? =
        attachmentApi.streamUrl(attachmentId)

    /**
     * Prepare and send a picked document. Nothing is re-encoded — a file
     * goes as it is (protocol.md, "Files").
     */
    /**
     * Recording state for the composer's strip. Elapsed is polled rather
     * than pushed: MediaRecorder has no progress callback, and a 200 ms
     * tick is cheaper than the alternative of not showing one at all.
     */
    private val _recordingMs = MutableStateFlow<Long?>(null)
    val recordingMs: StateFlow<Long?> = _recordingMs

    @Suppress("unused")
    private var recordingTicker: Job? = null

    /** Begin a voice note. The caller has already secured the permission. */
    fun startRecording() {
        if (voiceRecorder.isRecording) return
        if (!voiceRecorder.start()) {
            _mediaState.value = MediaSendState.Failed(appContext.getString(R.string.e_record_failed))
            return
        }
        _recordingMs.value = 0
        recordingTicker = viewModelScope.launch {
            while (voiceRecorder.isRecording) {
                _recordingMs.value = voiceRecorder.elapsedMs
                delay(200)
            }
            _recordingMs.value = null
        }
    }

    /** Stop and STAGE it, so a caption can be added before sending. */
    fun stopRecording() {
        recordingTicker?.cancel()
        recordingTicker = null
        _recordingMs.value = null
        val file = voiceRecorder.stop()
        if (file == null) {
            _mediaState.value =
                MediaSendState.Failed(appContext.getString(R.string.e_recording_too_short))
            return
        }
        _mediaState.value = MediaSendState.Preparing
        appScope.launch {
            val prepared = try {
                mediaPrep.prepareAudio(Uri.fromFile(file))
            } catch (_: Exception) {
                file.delete()
                _mediaState.value =
                    MediaSendState.Failed(appContext.getString(R.string.e_prepare_failed))
                return@launch
            }
            file.delete()
            stage(prepared)
        }
    }

    fun cancelRecording() {
        recordingTicker?.cancel()
        recordingTicker = null
        _recordingMs.value = null
        voiceRecorder.cancel()
    }

    fun stageFile(uri: Uri) = stageFiles(listOf(uri))

    /** The multi-document picker's entry: each Uri prepared and staged in order. */
    fun stageFiles(uris: List<Uri>) {
        if (uris.isEmpty()) return
        if (_mediaState.value == MediaSendState.Preparing ||
            _mediaState.value == MediaSendState.Uploading
        ) {
            return
        }
        _mediaState.value = MediaSendState.Preparing
        // App scope for the same reason as sendMedia.
        appScope.launch {
            uris.forEachIndexed { index, uri ->
                val declared = providerType(uri).orEmpty()
                val prepared = try {
                    // Audio the server's magic check knows gets a player rather
                    // than a document row; anything else it would refuse falls
                    // through to the file path, where nothing is verified.
                    if (declared in MediaPrep.SENDABLE_AUDIO_TYPES) {
                        mediaPrep.prepareAudio(uri)
                    } else {
                        mediaPrep.prepareFile(uri)
                    }
                } catch (_: MediaPrep.TooLargeAfterCompression) {
                    // A document cannot be compressed the way a video can, so
                    // the advice is different: there is nothing to try.
                    _mediaState.value =
                        MediaSendState.Failed(appContext.getString(R.string.e_file_too_large))
                    return@launch
                } catch (_: Exception) {
                    _mediaState.value =
                        MediaSendState.Failed(appContext.getString(R.string.e_read_file_failed))
                    return@launch
                }

                if (!stage(prepared)) return@launch
                if (index < uris.lastIndex) _mediaState.value = MediaSendState.Preparing
            }
        }
    }

    // -- Pasting ---------------------------------------------------------

    /** What a paste did with what it was given. */
    enum class PasteResult {
        /** Being prepared now; the strip shows it, then the chip appears. */
        STAGING,

        /**
         * Words. Whether they were APPENDED here or left for the text
         * field to insert at the caret depends on which door asked —
         * see [pasteFromClipboard] and [pasteIntoField].
         */
        TEXT,

        /**
         * Nothing was taken because attaching is not possible right now —
         * an edit is in progress, or the composer is already busy with an
         * upload or a download.
         */
        BUSY,

        /** More words than there was room for; what fit was taken. */
        TRUNCATED,

        /** None of it fit: the draft was already at the ceiling. */
        FULL,

        /** Nothing in it this composer can take. */
        NOTHING,
    }

    /**
     * Attach one item somebody copied in another app.
     *
     * No protocol and no server change: a pasted item becomes an ordinary
     * attachment upload followed by the existing claim-on-send, and it is
     * STAGED rather than sent — so a caption can be added, and a paste by
     * accident can be discarded. It goes through the same [stage] as the
     * picker, so it APPENDS behind whatever is already staged, up to the
     * cap a message may carry (docs/protocol.md).
     *
     * [declaredMime] is what the clipboard said the item is, used only
     * when the provider will not answer for itself. [keepAlive] is the
     * platform payload the Uri's read grant hangs off — see [stagePasted].
     *
     * Returns [PasteResult.NOTHING] for anything that is not an item to
     * attach — a copied LINK, most of all — so the caller can let it paste
     * as ordinary text instead.
     */
    fun pasteAttachment(
        uri: Uri,
        declaredMime: String? = null,
        keepAlive: Any? = null,
    ): PasteResult {
        // The provider's own answer first: it describes THIS item, while
        // the clip's type describes the clip.
        val mime = providerType(uri) ?: declaredMime
        // The RULE, before anything else — before the busy guard most of
        // all. It used to run after, which meant a copied LINK pasted
        // mid-edit came back BUSY: the door then swallowed an address
        // that was never an attachment in the first place and had every
        // right to land in the composer as words.
        val kind = PastedMedia.kindFor(uri.scheme, mime) ?: return PasteResult.NOTHING
        return stagePasted(listOf(PasteTarget(uri, mime, kind)), keepAlive)
    }

    /** One item the paste rule has already called an attachment. */
    private data class PasteTarget(val uri: Uri, val mime: String?, val kind: String)

    /**
     * Prepare and stage the items the rule has already called
     * attachments, in clip order.
     *
     * Private, and reachable only through [PastedMedia] having said so:
     * the preparation must never be the thing that decides what a
     * clipboard is, or there are two policies and they drift.
     *
     * [keepAlive] is whatever object the platform hung these Uris' read
     * grants on — for content committed by a KEYBOARD that is an
     * `InputContentInfo`, and the grants die with it, not at some later
     * timeout. The copies below run on another thread, so that object is
     * referenced until the last copy is done and cannot be collected out
     * from under it.
     */
    private fun stagePasted(
        targets: List<PasteTarget>,
        keepAlive: Any?,
    ): PasteResult {
        // The same guard the attach menu carries: the composer is borrowed
        // for an edit (which has no attachment), or already busy with one
        // upload. Repeated here because a paste can arrive from the text
        // field's own menu, which the attach button does not gate.
        if (_editTarget.value != null) {
            // Said out loud, because the edit banner explains the MODE and
            // not the refusal — a picture pasted into an edit otherwise
            // just does nothing at all.
            _mediaState.value =
                MediaSendState.Failed(appContext.getString(R.string.e_finish_editing_first))
            return PasteResult.BUSY
        }
        if (_mediaState.value.isBusy) {
            // Deliberately silent: this state IS the strip's message
            // ("Preparing…", "Sending…", or what a download is fetching),
            // and overwriting it with an error would both hide live
            // progress and release the busy guard — it is the same field a
            // second paste checks.
            return PasteResult.BUSY
        }

        _mediaState.value = MediaSendState.Preparing
        // Held, not stashed: the grant on a Uri a keyboard committed is
        // revoked the moment its InputContentInfo is collected, and the
        // copy below is on another thread. Cleared as soon as the bytes
        // are ours, so nothing outlives the one paste it belongs to.
        pasteGrant = keepAlive
        // App scope, like every other prepare-and-send here: leaving the
        // screen must not take a 90 MB upload with it.
        appScope.launch {
            try {
                targets.forEachIndexed { index, target ->
                    val prepared = try {
                        when (target.kind) {
                            // The SAME preparation the picker uses — the
                            // downscaled photo, the poster frame, the
                            // duration. A second path would be a second set
                            // of bugs.
                            AttachmentDto.KIND_PHOTO -> mediaPrep.preparePhoto(target.uri)
                            AttachmentDto.KIND_VIDEO ->
                                mediaPrep.prepareVideo(target.uri, declaredMime = target.mime)
                            AttachmentDto.KIND_AUDIO -> mediaPrep.prepareAudio(
                                target.uri,
                                declaredMime = target.mime,
                                fallbackName = pastedName(target.mime),
                            )
                            // `kind=file` REQUIRES a name of 1–255 characters
                            // and a clipboard item usually has none: MediaPrep
                            // prefers the provider's DISPLAY_NAME and falls
                            // back to this one.
                            else -> mediaPrep.prepareFile(
                                target.uri,
                                declaredMime = target.mime,
                                fallbackName = pastedName(target.mime),
                            )
                        }
                    } catch (_: MediaPrep.TooLargeAfterCompression) {
                        // The existing two messages, already translated: a
                        // video can be shortened, a document cannot be made
                        // smaller. What was already staged stays staged.
                        _mediaState.value = MediaSendState.Failed(
                            appContext.getString(
                                if (target.kind == AttachmentDto.KIND_VIDEO) {
                                    R.string.e_still_too_large
                                } else {
                                    R.string.e_file_too_large
                                },
                            ),
                        )
                        return@launch
                    } catch (_: Exception) {
                        _mediaState.value = MediaSendState.Failed(
                            appContext.getString(
                                if (target.kind == AttachmentDto.KIND_PHOTO ||
                                    target.kind == AttachmentDto.KIND_VIDEO
                                ) {
                                    R.string.e_prepare_failed
                                } else {
                                    R.string.e_read_file_failed
                                },
                            ),
                        )
                        return@launch
                    }

                    if (!stage(prepared)) return@launch
                    if (index < targets.lastIndex) {
                        _mediaState.value = MediaSendState.Preparing
                    }
                }
            } finally {
                pasteGrant = null
            }
        }
        return PasteResult.STAGING
    }

    /**
     * The read grant of the paste being prepared right now, held only for
     * as long as that takes. See [stagePasted].
     */
    private var pasteGrant: Any? = null

    /**
     * The attach menu's Paste: take whatever is on the clipboard, whether
     * or not the text field has focus.
     *
     * This door APPENDS the words it finds, because the text field may not
     * even have focus and there is no caret to insert at — the same rule
     * the assistant-mention button follows, and the same one the Mac and
     * the phone follow: moving somebody's cursor is worse than adding to
     * the end of what they were writing.
     */
    fun pasteFromClipboard(clip: ClipData?): PasteResult =
        paste(clip, appendsText = true, keepAlive = null)

    /**
     * The text field's OWN paste: its long-press menu, Ctrl+V from a
     * hardware keyboard, a keyboard that inserts pictures, a drop onto the
     * composer.
     *
     * Same rule, same answer — but the words are LEFT for the field, which
     * inserts them where the caret is. That is the one thing this platform
     * can do that the append-only doors cannot, and throwing it away to
     * match them would be a regression nobody asked for.
     *
     * [keepAlive] is the platform payload the item's read grant hangs off;
     * see [stagePasted].
     */
    fun pasteIntoField(clip: ClipData?, keepAlive: Any? = null): PasteResult =
        paste(clip, appendsText = false, keepAlive = keepAlive)

    /**
     * Every paste door, once the door has stopped having opinions.
     *
     * The clipboard is DESCRIBED first — scheme, media type, text, no
     * bytes — and [PastedMedia.decide] says what it is. Only then is a
     * preparation reached, and only for the item the rule named. A door's
     * whole remaining job is [appendsText]: whether it puts words in the
     * composer itself, or hands them back to something that will.
     */
    private fun paste(clip: ClipData?, appendsText: Boolean, keepAlive: Any?): PasteResult {
        val count = clip?.itemCount ?: 0
        val items = (0 until count).map { index ->
            val item = clip!!.getItemAt(index)
            val uri = item.uri
            PastedMedia.Item(
                scheme = uri?.scheme,
                // The provider's own answer first: it describes THIS item,
                // while the clip's type describes the clip.
                mime = uri?.let { providerType(it) } ?: clipMime(clip, index),
                text = item.text?.toString(),
            )
        }
        return when (val verdict = PastedMedia.decide(items)) {
            is PastedMedia.Verdict.Attach -> stagePasted(
                targets = verdict.picks.map { pick ->
                    PasteTarget(
                        uri = clip!!.getItemAt(pick.index).uri!!,
                        mime = items[pick.index].mime,
                        kind = pick.kind,
                    )
                },
                keepAlive = keepAlive,
            )

            is PastedMedia.Verdict.Words ->
                if (appendsText) appendPasted(verdict.text) else PasteResult.TEXT

            PastedMedia.Verdict.Empty -> {
                // Only the menu says so. The field's own paste reaches this
                // for anything it could not classify, and the field then
                // does whatever it does with it — an error strip over a
                // paste that pasted normally would be a lie.
                if (appendsText) {
                    _mediaState.value =
                        MediaSendState.Failed(appContext.getString(R.string.e_nothing_to_paste))
                }
                PasteResult.NOTHING
            }
        }
    }

    /**
     * Add pasted words to the draft, within the body limit.
     *
     * Nothing enforced the 4000-character limit anywhere before this: a
     * pasted wall of text looked like it had worked and then failed at
     * Send with `message_too_long`, by which time the clipboard had often
     * moved on (docs/protocol.md, "Limits"). What fits is kept and the
     * sentence says the rest was not pasted — the same choice the Apple
     * clients make, so a family that uses both sees one behaviour.
     */
    private fun appendPasted(text: String): PasteResult =
        when (val outcome = MessageBody.appending(text, inputState.text)) {
            is MessageBody.Paste.Appended -> {
                inputState.setTextAndPlaceCursorAtEnd(outcome.draft)
                PasteResult.TEXT
            }

            is MessageBody.Paste.Truncated -> {
                inputState.setTextAndPlaceCursorAtEnd(outcome.draft)
                reportPasteTruncated()
                PasteResult.TRUNCATED
            }

            // None of it fit, so the draft is left exactly as it was —
            // "the rest wasn't pasted" would be the wrong sentence when
            // none of it was.
            MessageBody.Paste.Full -> {
                _mediaState.value = MediaSendState.Failed(
                    appContext.getString(R.string.e_message_at_limit, MessageBody.MAX_CHARS),
                )
                PasteResult.FULL
            }
        }

    /**
     * Part of a paste was cut off for length — either here, or by the
     * composer's own input transformation, which is where the text field's
     * caret paste lands and is silent by design while somebody is typing.
     */
    fun reportPasteTruncated() {
        _mediaState.value = MediaSendState.Failed(
            appContext.getString(R.string.e_paste_truncated, MessageBody.MAX_CHARS),
        )
    }

    /**
     * What the clip says its item at [index] is.
     *
     * The mime list belongs to the DESCRIPTION rather than to the items,
     * and is not guaranteed to be as long — so an item past the end falls
     * back to the first type, which is what a single-type clip has anyway.
     */
    private fun clipMime(clip: ClipData, index: Int): String? {
        val description = clip.description ?: return null
        if (description.mimeTypeCount == 0) return null
        return description.getMimeType(index.coerceAtMost(description.mimeTypeCount - 1))
    }

    /**
     * The media type this item's provider gives it, or null.
     *
     * Guarded: `getType` reaches into another app's provider and throws
     * for a Uri whose read grant has lapsed — which is what a clipboard
     * Uri eventually does. It used to sit outside the try below, where a
     * throw reached an app-scope coroutine that has no handler.
     */
    private fun providerType(uri: Uri): String? =
        runCatching { appContext.contentResolver.getType(uri) }.getOrNull()

    /**
     * A name for a pasted item that arrived without one.
     *
     * Localised, because it is what the rest of the family will see on the
     * bubble — never the cache file's `upload-<UUID>.bin`, which is this
     * device's business and nobody else's.
     */
    private fun pastedName(mime: String?): String {
        val base = when (PastedMedia.topLevelType(mime)) {
            "image" -> R.string.s_pasted_image
            "audio" -> R.string.s_pasted_sound
            else -> R.string.s_pasted_file
        }
        return PastedMedia.nameFor(appContext.getString(base), mime)
    }

    /**
     * Download a file attachment if needed and hand back where it landed,
     * for the screen to open with whatever app can read it.
     */
    suspend fun localFile(attachment: AttachmentDto): File? = attachments.fileFor(attachment)

    /**
     * Opening a file failed. Two different causes, two different messages:
     * the bytes never arrived, or nothing on this phone can read them.
     */
    fun reportAttachmentOpenFailed(downloaded: Boolean) {
        _mediaState.value = MediaSendState.Failed(
            if (downloaded) {
                appContext.getString(R.string.e_no_app_for_file)
            } else {
                appContext.getString(R.string.e_download_failed)
            },
        )
    }

    /**
     * Download if needed, then copy into the phone's gallery.
     *
     * Android's chooser has no save-to-gallery action of its own (iOS's
     * share sheet does), so this is the only route to it.
     */
    suspend fun saveToGallery(context: Context, attachment: AttachmentDto): GallerySaver.Result {
        _mediaState.value = MediaSendState.Working("Saving…")
        val file = attachments.fileFor(attachment)
        if (file == null) {
            _mediaState.value = MediaSendState.Failed(appContext.getString(R.string.e_download_to_save_failed))
            return GallerySaver.Result.FAILED
        }
        val result = gallerySaver.save(
            context = context,
            file = file,
            mime = attachment.mime,
            displayName = attachment.name ?: attachment.fallbackFileName,
            isVideo = attachment.isVideo,
        )
        _mediaState.value = when (result) {
            GallerySaver.Result.SAVED -> MediaSendState.Idle
            GallerySaver.Result.NEEDS_PERMISSION -> MediaSendState.Idle
            GallerySaver.Result.FAILED -> MediaSendState.Failed(appContext.getString(R.string.e_save_failed))
        }
        return result
    }

    /** The user declined (or the system refused) the storage permission. */
    fun reportSaveNeedsPermission() {
        _mediaState.value = MediaSendState.Failed(
            appContext.getString(R.string.e_gallery_permission),
        )
    }

    /** True when this device needs the legacy storage permission to save. */
    val savingNeedsPermission: Boolean get() = gallerySaver.needsLegacyPermission

    /** Show what the screen is busy fetching, in the composer's strip. */
    fun reportAttachmentBusy(label: String) {
        _mediaState.value = MediaSendState.Working(label)
    }

    /** Dismiss a media failure notice. */
    fun clearMediaState() {
        _mediaState.value = MediaSendState.Idle
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
        if (!_loadingOlder.compareAndSet(expect = false, update = true)) return
        viewModelScope.launch {
            try {
                // `reachedStart` bounds the FETCH, not the window. It
                // used to bound both, and that was a real hole: it means
                // "the server has nothing older", while the window means
                // "how much of what Room holds is on screen" — and a
                // resync page can leave rows in Room the window has
                // never reached. Once the fetch was exhausted the window
                // could never grow again, so those rows were unreachable
                // for good, a quote jump into them stalled forever, and
                // the opening anchor could never be paged into view.
                if (!reachedStart) {
                    reachedStart = messageRepository.loadOlder(chatId)
                }
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

        /**
         * How many older pages a jump — a quote tap, or the opening
         * anchor — will page through before giving up. Bounded on its
         * OWN count and never on the window growing: [loadOlder] widens
         * the window whether or not the network answered, so a loop
         * watching the window would page forever while offline.
         */
        const val MAX_JUMP_PAGES = 3

        /**
         * How far back an opening anchor may reach: where the window
         * starts, plus everything the jump loop can add to it. Tied to
         * the loop by construction, because the give-up rule and the
         * loop's bound have to be the same number — a target the
         * arithmetic accepts and the loop cannot reach is a chat that
         * never finishes opening.
         *
         * This is a technical floor, not a product threshold: nettrash
         * chose to jump whenever there is ANYTHING unread.
         */
        const val ANCHOR_CAP = INITIAL_LIMIT + MAX_JUMP_PAGES * PAGE_SIZE

        const val READ_DEBOUNCE_MS = 500L
        const val TYPING_THROTTLE_MS = 3_000L
        const val TYPING_EXPIRY_MS = 5_000L
    }
}
