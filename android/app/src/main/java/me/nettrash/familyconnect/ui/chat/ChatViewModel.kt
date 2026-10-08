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

import me.nettrash.familyconnect.data.repo.TranscriptRepository
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
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
import me.nettrash.familyconnect.util.resolvedDisplayName
import me.nettrash.familyconnect.util.MemberMention
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.calls.CallStarter
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.calls.CallStateSource
import me.nettrash.familyconnect.util.Uptime
import android.os.SystemClock
import kotlinx.coroutines.flow.drop
import androidx.annotation.StringRes
import me.nettrash.familyconnect.data.repo.ParkedRecording
import me.nettrash.familyconnect.data.repo.ParkedRecordings
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
import me.nettrash.familyconnect.data.repo.AssistantFailure
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.data.repo.GallerySaver
import me.nettrash.familyconnect.data.repo.VoiceRecorder
import me.nettrash.familyconnect.data.repo.Waveform
import kotlinx.coroutines.Job
import me.nettrash.familyconnect.data.repo.LocationProvider
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.MessageBody
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.data.repo.PastedMedia
import me.nettrash.familyconnect.data.repo.ShareStash
import android.content.ClipData
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.net.dto.PollCodec
import me.nettrash.familyconnect.util.OpenPollsBadge
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.Clock
import me.nettrash.familyconnect.util.resolvedDisplayNames
import java.io.File
import javax.inject.Inject
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.repo.roundVideoLimits

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
    /**
     * Where a recording somebody did not get to finish deciding about waits:
     * the "Voice message not sent" rows (#79, S2.8).
     */
    private val parked: ParkedRecordings,
    private val attachmentApi: AttachmentApi,
    private val attachments: AttachmentRepository,
    private val gallerySaver: GallerySaver,
    private val locationProvider: LocationProvider,
    transcriptRepository: TranscriptRepository,
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
    /**
     * Whether a call is on, in any phase (#79, S1.2's **call**): nothing
     * records during one, and one starting stops and keeps a recording.
     * Defaulted with [callStarter]'s trick: Dagger injects CallManager, and
     * a test that never calls sees no call at all.
     */
    private val calls: CallStateSource = CallStateSource.NONE,
    /**
     * A clock that never jumps, for the voice reducer's activation guard
     * (#79). Defaulted with [callStarter]'s trick; the tests hand in
     * their scheduler's virtual time.
     */
    private val uptime: Uptime = Uptime { SystemClock.uptimeMillis() },
    /**
     * The chats whose own thread or polls screen is over them (#79, S4):
     * not left, but left the moment another chat comes on. Defaulted with
     * [callStarter]'s trick: Dagger injects the app-wide one; a test that
     * needs two chats to share it hands one in.
     */
    private val covers: CoveredChats = CoveredChats(),
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
     * Report an ASSISTANT reply. A separate path from [report]: the assistant
     * belongs to no family, so the member endpoint refuses it, and the people
     * who run the server read this one rather than the family owner
     * (docs/protocol.md, "Reporting the assistant").
     */
    fun reportAssistant(messageId: Long, reason: String, note: String?, onDone: () -> Unit) {
        viewModelScope.launch {
            val result = familyRepository.reportAssistant(messageId, reason, note)
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
     * How many open polls in this chat this reader still has to answer — the
     * badge on the toolbar's open-polls button (docs/protocol.md, "Finding
     * the open ones").
     *
     * Over the WHOLE cached chat (`observePolls`, no window), not over [items]:
     * that flow is the thread's render window and grows as the reader scrolls
     * up, so a badge derived from it undercounted older polls and changed
     * with scroll position, while iOS counted its whole store. The two ports
     * now feed the shared rule the same input.
     *
     * A poll by somebody this reader has blocked is a hidden row on the
     * surface and counts toward nothing — the same rule the thread applies
     * (`BlockedMessageRule.isHidden`).
     *
     * The RULE lives in `util/OpenPollsBadge.kt` and is mirrored on iOS.
     */
    val openPollsToAnswer: StateFlow<Int> = combine(
        messageRepository.observePolls(chatId),
        myUserId,
        blockedUserIds,
    ) { rows, me, blocked ->
        val mine = me ?: -1L
        val visible = rows
            .filterNot { BlockedMessageRule.isHidden(it.senderId, mine, blocked) }
            .mapNotNull { PollCodec.decode(it.pollJson) }
        OpenPollsBadge.count(visible, mine)
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), 0)

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
     * chat, where the whole family sees the answer arrive (typed there —
     * the button is the assistant chat's alone, [offersDrawButton]). Never a direct
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
     * Whether the composer shows the "Ask for a picture" BUTTON — the
     * member's own `ai` chat only (#78, docs/attachment-menu-2026-10-07.md:
     * the owner moved it out of the attach menu into its own paintbrush,
     * in the assistant chat, as on iOS and the Mac).
     *
     * Narrower than [canAskForPicture] on purpose: a family member can
     * still TYPE `@ai /draw …` and the server still draws it, so the
     * description hint keeps following [canAskForPicture]; only the
     * button is the assistant chat's.
     */
    val offersDrawButton: StateFlow<Boolean> =
        combine(chat, canAskForPicture) { chatEntity, canAsk ->
            canAsk && chatEntity?.kind == "ai"
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
     * that never resolves (docs/protocol.md, "Pictures"). Each carries HOW
     * it failed, which picks the sentence it shows (AssistantAnswer).
     */
    val failedAssistantAnswers: StateFlow<Map<Long, AssistantFailure>> =
        messageRepository.failedAssistantAnswers

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
            messageRepository.sendPoll(
                chatId, draft.question, draft.sendableOptions, quote,
                resolvedMentions(draft.question),
            )
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

        /**
         * Something went wrong, in the strip until dismissed. [opensSettings]:
         * the microphone was refused for good, and the strip offers Open
         * Settings beside the sentence (#79, S2.2).
         */
        data class Failed(val reason: String, val opensSettings: Boolean = false) : MediaSendState

        /**
         * A sentence that is not an error — why a dimmed microphone did not
         * record, what letting go will do next time, the five-minute stop
         * (#79, S1.3: dimmed controls SAY why, in this line). Drawn without
         * the error styling; like [Failed], not busy.
         */
        data class Notice(val text: String) : MediaSendState

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

    // -- Member mentions (docs/protocol.md, "Mentioning a member") ---------

    /** Everybody a mention may name: the active roster, by the name the app calls them. */
    private val mentionRoster: StateFlow<List<MentionDto>> = memberDao.observeActiveMembers()
        .map { members -> members.map { MentionDto(it.userId, it.resolvedDisplayName(appContext)) } }
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    /**
     * What the strip above the composer offers: the roster narrowed to the
     * `@prefix` being typed — never the reader themself, never the
     * blocked, and never the assistant, which is not in the roster and has
     * its own button. Empty outside the family chat and outside a token.
     */
    val mentionCandidates: StateFlow<List<MentionDto>> = combine(
        draftText,
        mentionRoster,
        chat,
        settings.state,
    ) { draft, roster, chatEntity, settingsState ->
        if (chatEntity?.kind != "family") return@combine emptyList()
        val query = MemberMention.query(draft) ?: return@combine emptyList()
        MemberMention.candidates(
            roster,
            query,
            excluding = settingsState.blockedUserIds + setOfNotNull(settingsState.myUserId),
        )
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /** A name picked from the strip: the trailing `@prefix` becomes `@Name `. */
    fun acceptMention(name: String) {
        inputState.setTextAndPlaceCursorAtEnd(MemberMention.accept(inputState.text.toString(), name))
    }

    /** The members the text names, resolved at send — family chat only. */
    private fun resolvedMentions(body: String): List<MentionDto>? {
        if (chat.value?.kind != "family") return null
        return MemberMention.resolve(body, mentionRoster.value).takeIf { it.isNotEmpty() }
    }

    /**
     * A tap on a name opens the one-to-one chat with that member — never
     * the reader's own name, a member who has left or a deleted account.
     */
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

    /**
     * Whether the composer says, under a picture request being typed,
     * that real names and brands are often refused (docs/protocol.md,
     * "Pictures"). Only where [canAskForPicture] offers `/draw` at all,
     * never while the composer is borrowed for an edit; the rule is
     * [PictureDescriptionHint.inComposer], pinned by its own tests.
     *
     * Eager, for [mentionPictureNotice]'s reason: the line has to be
     * there on the frame the "ask for a picture" button leaves `/draw `.
     */
    val showsPictureDescriptionHint: StateFlow<Boolean> =
        combine(chat, draftText, canAskForPicture, _editTarget) {
                chatEntity, draft, offered, editing ->
            PictureDescriptionHint.inComposer(
                draft = draft,
                picturesOffered = offered,
                inFamilyChat = chatEntity?.kind == "family",
                editing = editing != null,
            )
        }.stateIn(viewModelScope, SharingStarted.Eagerly, false)

    /**
     * WHO ANSWERS, verbatim as the operator named them, or null on a
     * server that named nobody — which is a server whose assistant this
     * client does not offer at all (docs/protocol.md, "Consenting to the
     * assistant").
     */
    val assistantProcessor: StateFlow<String?> = settings.state
        .map { it.assistantProcessor }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /**
     * Whether this draft would go to the model with nobody having agreed
     * to that yet. The composer shows the line that says where it would
     * go, and the send raises the screen that asks.
     *
     * Eager, for [mentionPictureNotice]'s reason: the line has to be there
     * on the frame the `@ai` is, not one frame later.
     */
    val assistantConsentNeeded: StateFlow<Boolean> =
        combine(chat, draftText, settings.state, _editTarget) {
                chatEntity, draft, settingsState, editing ->
            // Not while the composer is borrowed for an edit: rewriting an
            // old message calls no model, and the draft holding an `@ai`
            // there is the message being fixed rather than a question.
            editing == null &&
                AssistantConsent.isRequired(
                    chatKind = chatEntity?.kind,
                    body = draft,
                    processor = settingsState.assistantProcessor,
                    agreedAt = settingsState.assistantConsentAt,
                )
        }.stateIn(viewModelScope, SharingStarted.Eagerly, false)

    /**
     * Whether this draft would reach an assistant this server refuses to
     * name — in which case it goes nowhere, and the composer says why.
     */
    val assistantIsUnnamed: StateFlow<Boolean> =
        combine(chat, draftText, settings.state, _editTarget) {
                chatEntity, draft, settingsState, editing ->
            editing == null &&
                AssistantConsent.isWithheldFromAnUnnamedAssistant(
                    chatKind = chatEntity?.kind,
                    body = draft,
                    hasAssistant = settingsState.assistantUserId != null,
                    processor = settingsState.assistantProcessor,
                )
        }.stateIn(viewModelScope, SharingStarted.Eagerly, false)

    /**
     * Answer the assistant question, and send what was waiting.
     *
     * The stamp is the SERVER's: agreeing twice keeps the first one, and a
     * client that invented a date would show one the server would not.
     * [send] is called again afterwards because the draft is still in the
     * box — agreeing finishes the send the person already asked for.
     */
    fun agreeToTheAssistant(withLookups: Boolean = false) {
        viewModelScope.launch {
            _assistantConsentAsked.value = false
            // Asked for by a "Voice message not sent" row's Send rather than
            // by the composer's: agreeing finishes THAT send, and the draft
            // in the box — which nobody asked to send — stays there.
            val notSentSend = notSentAwaitingConsent
            notSentAwaitingConsent = null
            // "Agree With Lookups" records the lookup consent too, after the
            // first one (the server refuses it otherwise). If only that
            // second write fails the send still goes — answered as it would
            // be without lookups, which is the safe direction.
            if (familyRepository.agreeToAssistant(withLookups).assistant) {
                // The draft never left the box, so this finishes the send
                // the person already asked for.
                if (notSentSend != null) sendNotSent(notSentSend) else send()
            }
        }
    }

    /** "Not Now": the screen closes and the draft stays where it is. */
    fun dismissAssistantConsent() {
        _assistantConsentAsked.value = false
        notSentAwaitingConsent = null
    }

    /**
     * Whether the consent screen is up, because a send would have reached
     * the model (docs/protocol.md, "Consenting to the assistant").
     */
    private val _assistantConsentAsked = MutableStateFlow(false)

    /**
     * What the consent screen must say, or null while it is not up.
     *
     * One value rather than three, because the screen's promises depend on
     * the family's own switches: with `ai_history` off a mention takes
     * nothing but itself, and a screen claiming the last 30 days would be
     * asking permission for something that does not happen — and hiding
     * what does.
     */
    val assistantConsentAsk: StateFlow<AssistantConsentAsk?> =
        combine(_assistantConsentAsked, settings.state) { asked, settingsState ->
            val processor = settingsState.assistantProcessor
            if (!asked || processor.isNullOrBlank()) {
                null
            } else {
                AssistantConsentAsk(
                    processor = processor,
                    familyHistory = settingsState.familyAiHistory,
                    familyVision = settingsState.familyAiVision,
                    transcripts = settingsState.assistantTranscribe,
                    // The second question rides on the same screen when this
                    // server can look things up (docs/protocol.md,
                    // "Consenting to the assistant", amended 2026-10-03).
                    lookupProviders = AssistantLookups.providers(settingsState.assistantLookups),
                )
            }
        }.stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /**
     * "Show text" under the recordings in this chat, and the consent
     * question it may raise — drawn as its own dialog by the screen
     * (docs/protocol.md, "Transcripts on request").
     */
    val transcripts = Transcripts(
        scope = viewModelScope,
        settings = settings,
        repository = transcriptRepository,
        agree = { familyRepository.setAssistantConsent(true) },
    )

    /** What the consent screen is drawn from. */
    data class AssistantConsentAsk(
        val processor: String,
        val familyHistory: Boolean,
        val familyVision: Boolean,
        /** `assistant.transcribe`: the screen says a recording's sound goes too, when asked. */
        val transcripts: Boolean = false,
        /**
         * `assistant.lookups`: when non-empty the screen also asks the
         * lookup question, naming these. Empty where the caller cannot
         * record that answer, or the server has no source.
         */
        val lookupProviders: List<String> = emptyList(),
    )

    /** Screen calls this from a LifecycleResumeEffect. */
    fun setResumed(isResumed: Boolean) {
        resumed.value = isResumed
        publishOpenChat()
        // Resumed is BACK — not merely composed, which a cancelled predictive
        // Back also does — so this is where a chat stops being covered by its
        // own thread or polls, and where every other chat still covered is
        // left: the person is here now (#79, S4; CoveredChats).
        if (isResumed) {
            coveredByOwnScreen = false
            covers.uncover(coveredChat)
            covers.leaveAllBut(coveredChat)
        }
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
        // NOTHING REACHES THE MODEL UNASKED. Before the staged list is
        // taken, so a refused send consumes nothing: the server refuses
        // this with `assistant_consent_required` anyway, and asking here
        // is what turns that refusal into a question with the message
        // still in hand (docs/protocol.md, "Consenting to the assistant").
        if (assistantConsentNeeded.value) {
            _assistantConsentAsked.value = true
            return
        }
        // And a server that will not say WHO answers gets nothing at all.
        if (assistantIsUnnamed.value) return
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
        viewModelScope.launch { messageRepository.send(chatId, body, quote, resolvedMentions(body)) }
    }

    /**
     * A sticker was tapped in the panel: decide whether it may go, and hand
     * [go] the quote it answers.
     *
     * The sticker itself is sent by StickerViewModel — the pack is not this
     * model's — but two things about a send ARE this model's, and a sticker
     * is a send like any other (docs/protocol.md, "Sending one"):
     *
     *  - THE REPLY DRAFT. A sticker may be a reply, which is how one
     *    answers something. Read and cleared together, as [send] does, so
     *    the quote does not silently ride on the next message too.
     *  - NOTHING REACHES THE MODEL UNASKED. In the member's own `ai` chat a
     *    sticker is a photo to the assistant, so the consent question is
     *    asked first, exactly as for words; the server would refuse it with
     *    `assistant_consent_required` otherwise, and that is a red bubble
     *    where a question belongs. A server that names no processor gets
     *    nothing at all. (In the family chat a sticker has no body and so
     *    can never say `@ai`.)
     *
     * The typed draft is left alone: one tap sends the sticker, not the
     * sentence somebody was in the middle of.
     */
    fun beginStickerSend(go: (ReplyToDto?) -> Unit) {
        // An edit has borrowed the composer; the button is disabled then,
        // and this is the same answer for anything that got past it.
        if (_editTarget.value != null) return
        viewModelScope.launch {
            val settingsState = settings.state.first()
            when (
                AssistantConsent.stickerGate(
                    chatKind = chat.value?.kind,
                    hasAssistant = settingsState.assistantUserId != null,
                    processor = settingsState.assistantProcessor,
                    agreedAt = settingsState.assistantConsentAt,
                )
            ) {
                AssistantConsent.StickerGate.WITHHELD -> Unit
                AssistantConsent.StickerGate.ASK -> _assistantConsentAsked.value = true
                AssistantConsent.StickerGate.SEND -> {
                    val quote = _replyDraft.value
                    _replyDraft.value = null
                    go(quote)
                }
            }
        }
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
                personChangedStrip()
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
                        mentions = resolvedMentions(caption),
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

    /**
     * Hand a staged set to the outbox and get the composer back.
     *
     * Nothing is awaited and nothing can be lost here any more: the
     * repository writes the message row and its item rows — and moves the
     * bytes out of the evictable cache — before the first byte goes out,
     * so the send belongs to the database from this instant. A failure is
     * a red bubble with a Retry button, exactly like a text message,
     * rather than a set this view model has to catch and hold.
     */
    private fun sendStaged(prepared: List<MediaPrep.Prepared>, caption: String) {
        val quote = _replyDraft.value
        inputState.clearText()
        _replyDraft.value = null
        // App scope, not viewModelScope: the staging and the first upload
        // must not be cancelled by navigating away.
        appScope.launch {
            val queued =
                messageRepository.sendMedia(prepared, caption, chatId, quote, resolvedMentions(caption))
            if (queued == null) {
                // Only when not one item could be staged — a full disk, or
                // a file that vanished between picking and sending.
                _mediaState.value =
                    MediaSendState.Failed(appContext.getString(R.string.e_send_failed))
                val remainder = prepared.filter { it.file.exists() }
                if (remainder.isNotEmpty()) {
                    _staged.update { current -> remainder + current }
                }
                restoreComposer(caption, quote)
            } else {
                _mediaState.value = MediaSendState.Idle
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
    fun stagePrepared(prepared: MediaPrep.Prepared) = stage(prepared).also { if (it) personChangedStrip() }

    /**
     * True when the item was taken; false (file deleted, notice shown) at the
     * cap — or, with [keepRefused], false with the file left alone, for a
     * caller that has somewhere else to keep it: a recording cannot be made
     * again, and a full strip must not be what deletes one.
     */
    private fun stage(prepared: MediaPrep.Prepared, keepRefused: Boolean = false): Boolean {
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
                if (!keepRefused) prepared.file.delete()
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

    /**
     * Throw away ONE staged item and the temp file nothing else will clean
     * up — except a voice note of ten seconds or more, whose ✕ asks "Delete
     * this recording?" first (#79, S2.7): a recording cannot be made again.
     */
    fun discardStaged(index: Int) {
        val victim = _staged.value.getOrNull(index) ?: return
        if (victim.voiceNote && VoiceNoteRules.deleteAsks(victim.durationMs?.toLong() ?: 0L)) {
            _stagedDeleteAsk.value = victim.file
            return
        }
        removeStaged(victim.file)
        personChangedStrip()
    }

    /** Remove the staged item holding [file], and delete it. */
    private fun removeStaged(file: File) {
        // CAS: a concurrent stage() append must not be clobbered by this
        // write (nor vice versa). The victim is resolved against the SAME
        // list the swap replaces, and its file is deleted only after the
        // swap took — never the file of an item that stays visible.
        while (true) {
            val current = _staged.value
            val victim = current.firstOrNull { it.file == file } ?: return
            if (_staged.compareAndSet(current, current.filterNot { it.file == file })) {
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

    // -- Voice messages from the Send slot ------------------------------------
    //
    // #79, docs/audio-video-messages-2026-10-04.md.
    //
    // PHASE 0 made today's recorder safe. The recorder is one per process,
    // and this ViewModel used to start it and then let it run whatever
    // happened to the screen: Back mid-recording left the microphone open and
    // every later voice note dead until the process died; a call rang over a
    // recording; the five-minute cap stopped the hardware while the counter
    // ran on. Now nothing records during a call; interruptions stop and KEEP,
    // never send and never discard, and what they keep waits in its own
    // "Voice message not sent" row ([notSent], S2.8) with the reply it was
    // recorded under; leaving the chat does the same and turns a voice
    // message in review into one too; onCleared is the last word.
    //
    // PHASE 1 put voice in the Send slot. Every decision about an
    // activation, Stop, Delete, the cap and an interruption is the SHARED
    // reducer's (RecordGesture, held to the reference's vectors): this
    // ViewModel feeds it ([step]) and carries out what comes back
    // ([perform]).
    //
    //  - A TAP — a click, Enter or Space, TalkBack — records hands-free; the
    //    same slot, now an arrow, sends it; Stop keeps it for review; Delete
    //    deletes it (asking from ten seconds).
    //  - There is NO HOLD (revised 2026-10-06): a long press on the
    //    microphone records nothing and opens nothing while the finger is
    //    down, and is a tap when it lifts inside. With it went the Undo
    //    window, its "sending" entry, the first-release lesson, the coach
    //    mark and Review Before Sending.

    /**
     * The recording's own clock for the composer's timer, or null while
     * nothing records. Polled: MediaRecorder has no progress callback.
     */
    private val _recordingMs = MutableStateFlow<Long?>(null)
    val recordingMs: StateFlow<Long?> = _recordingMs

    /** Whether the composer is showing a recording of its own (the recording row). */
    private var recordingHere = false

    /** Whether the one recorder the app has is running for THIS composer. */
    private var recorderOpen = false

    private var recordingTicker: Job? = null

    /**
     * Hears every way the recorder ends a recording on its own — the cap, a
     * failure, another app, another chat starting one — always on the main
     * thread, where the recorder is driven.
     */
    private val recorderOwner = VoiceRecorder.Listener { ending, kept -> onRecorderEnded(ending, kept) }

    /** The session this chat opened in: a park that lands after a sign-out is dropped. */
    private val session: Long = parked.session()

    /** A call in any phase but idle or ended (S1.2's **call**). */
    val callLive: StateFlow<Boolean> = calls.state
        .map { it is CallState.Live }
        .stateIn(viewModelScope, SharingStarted.Eagerly, calls.state.value is CallState.Live)

    /** This chat's voice messages that were not sent: the rows above the field (S2.8). */
    val notSent: StateFlow<List<ParkedRecording>> = parked.forChat(chatId)
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    /** Not-sent messages whose send is under way, so a second tap does nothing. */
    private val _notSentInFlight = MutableStateFlow<Set<String>>(emptySet())
    val notSentInFlight: StateFlow<Set<String>> = _notSentInFlight

    /** A not-sent message whose Send raised the assistant question, sent once it is answered yes. */
    private var notSentAwaitingConsent: String? = null

    /**
     * The not-sent message "Delete this recording?" is about, or null while
     * nothing is asked. Only at ten seconds or more: shorter ones go at once.
     */
    private val _deleteAsk = MutableStateFlow<String?>(null)
    val deleteAsk: StateFlow<String?> = _deleteAsk

    /**
     * The staged voice note "Delete this recording?" is about (S2.7: its ✕
     * asks at ten seconds or more), by its file, or null.
     */
    private val _stagedDeleteAsk = MutableStateFlow<File?>(null)
    val stagedDeleteAsk: StateFlow<File?> = _stagedDeleteAsk

    /** The screen's facts at an activation, which the reducer's [RecordGesture.Situation] reads. */
    data class VoiceEnvironment(
        val permission: RecordGesture.Permission = RecordGesture.Permission.GRANTED,
        /**
         * TalkBack's touch exploration runs (S6): the microphone opens only
         * once "Recording" has been spoken.
         */
        val assistive: Boolean = false,
    )

    /** The slot's voice recording as the shared reducer has it. */
    private val _hold = MutableStateFlow(RecordGesture.HoldState())
    val hold: StateFlow<RecordGesture.HoldState> = _hold

    /** S1.1's numbers. */
    private val holdConstants = RecordGesture.HoldConstants()

    /**
     * The words the composer held when the slot's own activation last set
     * the activation guard — which holds back a Send only while they are
     * still the same: words typed since are never guarded (S1.1).
     */
    private var draftAtGuard = ""

    /** The screen's facts at the last activation. */
    private var voiceEnv = VoiceEnvironment()

    /** With TalkBack, the microphone waits until "Recording" has been spoken. */
    private var pendingStart: Job? = null

    /** What the recorder kept when it ended a recording by itself, for the effects to take. */
    private var endedRecording: VoiceRecorder.Recording? = null

    /** A recording stopped to be asked about ("Delete this recording?"). */
    private var askedRecording: VoiceRecorder.Recording? = null

    /**
     * Voice notes on their way into review — being prepared on the app's
     * scope after a Stop or a Keep — and the lock that makes leaving and
     * arriving one or the other: leaving either finds a note STAGED (and
     * parks it with the words) or CLAIMS it on its way — never neither,
     * which is how a quick second Back used to lose one (S2.8, S4).
     */
    private val reviewLock = Any()
    private val reviewsInFlight = mutableListOf<ReviewInFlight>()

    /**
     * How many times leaving has turned review into "not sent" (S4) — and
     * whether it is doing so now, under [reviewLock]: what tells a voice note
     * Send could not prepare or hand off whether the review it lands in
     * (S2.5) is still the composer somebody sees ([reviewTicket]).
     */
    private var leaves = 0
    private var leaving = false

    /** Some peak since the recording started rose above digital silence. */
    private var heardSound = false

    /** The last refusal is for good: the strip offers Open Settings. */
    private var denialPermanent = false

    /** The level meter: how many of its five bars the last peak lit (S2.9). */
    private val _voiceLevel = MutableStateFlow(0)
    val voiceLevel: StateFlow<Int> = _voiceLevel

    /**
     * The hands-free row's LIVE waveform (#79, the approved design): the
     * newest [LIVE_LEVELS] peaks as waveform levels, 0–15, oldest first — the
     * same measure the note's own waveform is made of, so what scrolls in
     * while recording is what the reader will see.
     */
    private val _voiceLevels = MutableStateFlow<List<Int>>(emptyList())
    val voiceLevels: StateFlow<List<Int>> = _voiceLevels

    /** The recording row's line, shown in the level meter's place. */
    enum class VoiceLine {
        /** From 4:30: "30 seconds left", and the timer turns orange (S2.5). */
        THIRTY_SECONDS_LEFT,

        /** Three seconds in with nothing heard: "We can't hear anything. Is the microphone muted?" */
        CANT_HEAR,
    }

    private val _voiceLine = MutableStateFlow<VoiceLine?>(null)
    val voiceLine: StateFlow<VoiceLine?> = _voiceLine

    /** What the composer's polite live region says next (S6) — state changes, never the clock. */
    data class VoiceAnnouncement(val text: String, val serial: Long)

    private val _announcement = MutableStateFlow<VoiceAnnouncement?>(null)
    val announcement: StateFlow<VoiceAnnouncement?> = _announcement
    private var announcementSerial = 0L

    /** The one-off things only the screen can do. */
    sealed interface VoiceEffect {
        /** Raise the system's microphone prompt; the answer comes back as [permissionAnswered]. */
        data object AskPermission : VoiceEffect

        /** S2.9's haptics — the screen plays them on phones only. */
        data class Haptic(val haptic: RecordGesture.Haptic) : VoiceEffect

        /** Recording started: focus to the slot, the keyboard down (S2.4). */
        data object FocusSlot : VoiceEffect

        /** It ended by Send, Delete, "too short" or into review: focus back to the field (S2.4). */
        data object Ended : VoiceEffect
    }

    private val _voiceEffects = MutableSharedFlow<VoiceEffect>(extraBufferCapacity = 16)
    val voiceEffects: SharedFlow<VoiceEffect> = _voiceEffects

    /**
     * The server has video messages: it sent `max_round_video_ms` and
     * `max_round_video_bytes` on `GET /families/mine` (#79). Without them no
     * video entry is offered anywhere (S1.2's **round available**).
     */
    val roundVideoOffered: StateFlow<Boolean> = settings.state.map { it.roundVideoLimits != null }
        .stateIn(viewModelScope, SharingStarted.Eagerly, false)

    init {
        // A call stops and keeps a recording the moment it rings or is
        // placed — before the call screen has even come up (S4).
        viewModelScope.launch {
            callLive.collect { live -> if (live) interruptRecording() }
        }
        // A character typed, deleted, pasted or suggested is the person's own
        // change (S1.1): it lifts the slot's activation guard. The field
        // emptied by the slot's own Send is not a change of the person's: it
        // arrives here (later) as the very words the guard was set over, and
        // lifts nothing.
        viewModelScope.launch {
            draftText.drop(1).collect { text ->
                if (text != draftAtGuard) otherAction()
            }
        }
    }

    /**
     * The slot was activated: the microphone, the Send arrow or the Stop
     * square — by a completed tap however long it was held, a click, Enter,
     * Space or TalkBack. Nothing is said to the reducer while a press is
     * down: a long press is not a gesture (the reducer's contract).
     */
    fun activateSlot(env: VoiceEnvironment) {
        voiceEnv = env
        denialPermanent = false
        step(RecordGesture.HoldEvent.Activate(uptime.now(), situation(), recordedNow()))
    }

    /**
     * The slot's row-4 Save or row-5 Send. Asked of the guard first — a
     * double tap on the Stop square must not send what it staged (S1.1) —
     * and once it has emptied the composer, the microphone it turns into is
     * guarded in turn: a double tap on Send cannot start a recording.
     */
    fun sendFromSlot() {
        val at = uptime.now()
        if (sendGuarded(at)) return
        send()
        step(RecordGesture.HoldEvent.Emptied(at))
    }

    /**
     * Whether a press going down on the slot NOW is ignored whole — asked by
     * RecordSendButton at the press's DOWN, for every press and every Enter
     * or Space. S1.1: "A press that goes down while the guard runs
     * is ignored whole" — so a slow second tap, whose lift comes after the
     * 600 ms, still cannot send the recording the first tap started, nor send
     * what a Stop has just staged. Send and Save ask [sendGuarded]: words
     * typed since the guard began are never held back.
     */
    fun slotPressIgnored(slot: ComposerSlot.Slot): Boolean {
        val at = uptime.now()
        return when (slot) {
            ComposerSlot.Slot.Send, is ComposerSlot.Slot.Save -> sendGuarded(at)
            else -> _hold.value.guarded(at)
        }
    }

    /**
     * The guard as a row-4 or row-5 Send meets it: the slot's own last
     * activation was under 600 ms ago AND the words are still what that
     * activation left (S1.1, Decision 15). A change made by typing, pasting,
     * a suggestion, deleting, staging or taking something off is never
     * guarded — it lifts the guard ([otherAction]) — so "ok" or an emoji
     * typed after a send and sent at once still goes. The words are compared
     * as well because the field's change reaches [otherAction] through a
     * snapshot flow, a frame later than the keystroke.
     */
    private fun sendGuarded(at: Long): Boolean =
        _hold.value.guarded(at) && inputState.text.toString() == draftAtGuard

    /**
     * "Record voice message" — the paperclip, the microphone's menu, or
     * Ctrl+Shift+R; during a recording (only the shortcut can be) it STOPS
     * it into review: a shortcut never sends (S1.6). Not in the assistant's
     * chat and not during an edit, where no recording is offered at all.
     */
    fun recordVoiceMessage(env: VoiceEnvironment = VoiceEnvironment()) {
        if (chat.value?.kind == "ai" || _editTarget.value != null) return
        voiceEnv = env
        denialPermanent = false
        val besideDraft = inputState.text.isNotBlank() || _staged.value.isNotEmpty()
        step(RecordGesture.HoldEvent.Record(uptime.now(), besideDraft, situation(), recordedNow()))
    }

    /** The paperclip's "Record voice message", with a granted microphone. */
    fun startRecording() = recordVoiceMessage()

    /**
     * Stop into review — the stop icon, Esc, Android's Back while recording,
     * "Stop and listen first" (S2.5). Under a second it is too short.
     */
    fun stopRecording() {
        step(RecordGesture.HoldEvent.Stop(uptime.now(), recordedNow()))
    }

    /** The recording row's Delete: at once under ten seconds, asked from ten (S2.5). */
    fun deleteRecording() {
        step(RecordGesture.HoldEvent.Delete(uptime.now(), recordedNow()))
    }

    /** "Delete this recording?" about the recording just stopped: Delete, or Keep (review). */
    fun answerRecordingDelete(delete: Boolean) {
        step(RecordGesture.HoldEvent.Answer(uptime.now(), delete))
    }

    /**
     * Anything else the person does in the composer — the paperclip, a
     * sticker, `@ai`, a character typed or deleted, something staged or taken
     * off — which, outside a recording, lifts the slot's activation guard:
     * the next press is a decision of its own (S1.1). The reducer decides.
     */
    fun otherAction() {
        step(RecordGesture.HoldEvent.OtherAction(uptime.now()))
    }

    /**
     * The person staged something, or took something off the strip: their
     * own change, never guarded (S1.1) — said to the reducer on the main
     * thread, since staging lands from the prepare loops' threads. A
     * recording the slot's own Stop staged is not this.
     */
    private fun personChangedStrip() {
        viewModelScope.launch { otherAction() }
    }

    /** The system's microphone prompt answered; [permanent]: it will not be asked again. */
    fun permissionAnswered(granted: Boolean, permanent: Boolean) {
        denialPermanent = !granted && permanent
        step(RecordGesture.HoldEvent.PermissionAnswer(uptime.now(), granted))
    }

    /** A play control tapped while recording: what it says instead (S1.7). */
    fun explainPlaybackWhileRecording() {
        notice(R.string.s_play_after_recording)
    }

    // -- Video messages (#79, Phase 3) ----------------------------------------

    /**
     * Whether the video recorder may open from this composer now — asked by
     * every way in (S3.1): the video button, the paperclip, the microphone's
     * menu and its TalkBack action. Family and direct chats only, never
     * mid-edit. During a call (row 7) or while an attachment is busy (row 8)
     * it SAYS why instead, in the composer's notice line — dimmed is not
     * disabled (S1.3, S1.4, S1.6). Row 9 (a voice message not sent) opens it:
     * that rule is about voice. And one recording at a time (S1.7): a voice
     * recording running here is stopped and kept as "not sent" first.
     */
    fun mayOpenVideoRecorder(): Boolean {
        val kind = chat.value?.kind
        if ((kind != "family" && kind != "direct") || _editTarget.value != null) return false
        if (callLive.value) {
            notice(R.string.e_record_after_the_call)
            return false
        }
        if (_mediaState.value.isBusy) {
            notice(R.string.e_wait_for_the_attachment)
            return false
        }
        if (_hold.value.recording != ComposerSlot.Recording.NONE) interruptRecording()
        return true
    }

    /**
     * The recorder sent a video message carrying [replyId]: the composer's
     * reply is spent — the sticker's rule (S1.5). A reply primed since, to
     * another message, stays.
     */
    fun videoMessageSent(replyId: Long?) {
        if (replyId != null && _replyDraft.value?.messageId == replyId) _replyDraft.value = null
    }

    /** What the recorder says once it has closed — "Video message sent", "Camera turned off" (S6). */
    fun announceFromRecorder(@StringRes text: Int) {
        say(appContext.getString(text))
    }

    /**
     * Abandon whatever is recording here, at once and without a question.
     * Never offered to the person — Delete asks at ten seconds — but the
     * tests' teardown, and a last resort.
     */
    fun cancelRecording() {
        discardRecording()
        _hold.value = RecordGesture.HoldState(guardUntilMs = _hold.value.guardUntilMs)
    }

    /**
     * The screen came on. Paired with [screenDetached]; the count is what
     * tells a rotation, whose new screen arrives, from a screen that is gone.
     */
    fun screenAttached() {
        screens++
        orphanCheck?.cancel()
        orphanCheck = null
    }

    /**
     * The chat is about to open ITS OWN thread or polls screen over itself —
     * which is not leaving it (S4 lists the back button, another chat, the
     * rail and a notification tap; on the iPhone both are sheets over a
     * conversation that stays). Its screen will leave composition, but this
     * ViewModel stays and its composer comes back as it was: a voice message
     * in review stays in review, with the photos, the words and the reply
     * beside it. A recording still stops and is kept (ON_STOP), since nobody
     * can see it.
     */
    fun coverWithOwnScreen() {
        coveredByOwnScreen = true
    }

    /**
     * The screen left composition. Not for a configuration change — a
     * rotation, a fold, a theme change rebuild the activity and the
     * recording carries on (S4) — but for anything else the composer is
     * gone: the chat was left, or another chat opened over it. Under the
     * chat's own thread or polls ([coverWithOwnScreen]) only a recording
     * stops; the chat goes on the covered list, and is left the moment
     * another chat comes on (CoveredChats).
     */
    fun screenDetached(changingConfigurations: Boolean) {
        screens = (screens - 1).coerceAtLeast(0)
        if (!changingConfigurations) {
            if (coveredByOwnScreen) {
                interruptRecording()
                covers.cover(coveredChat)
            } else {
                leave(duringCall = calls.state.value is CallState.Live)
            }
            return
        }
        // A rebuilt activity brings its screen straight back. One that does
        // not — a tablet window narrowed past the two-pane width, which
        // leaves this chat out of the new layout — must not leave a
        // recording running where nobody can see it.
        if (screens == 0) {
            orphanCheck?.cancel()
            orphanCheck = viewModelScope.launch {
                delay(ORPHAN_GRACE_MS)
                if (screens == 0) leave(duringCall = calls.state.value is CallState.Live)
            }
        }
    }

    /**
     * The screen stopped: the app went to the background, the screen locked,
     * the call screen or another app came over it (ON_STOP). A recording stops
     * and is kept; what is in review stays in review (S4).
     */
    fun screenStopped(changingConfigurations: Boolean) {
        if (!changingConfigurations) interruptRecording()
    }

    private var screens = 0
    private var orphanCheck: Job? = null

    /** The chat's own thread or polls screen is over it ([coverWithOwnScreen]); cleared once it is back. */
    private var coveredByOwnScreen = false

    /** This chat on the covered list: leaving it is what leaving the chat does. */
    private val coveredChat = CoveredChats.Covered {
        coveredByOwnScreen = false
        leave(duringCall = calls.state.value is CallState.Live)
    }

    override fun onCleared() {
        // The last word, and the fix for the microphone left open after Back:
        // nothing that cannot be made again dies with this ViewModel. Its own
        // scope is already cancelled; the writes go out on the app's.
        covers.uncover(coveredChat)
        leave(duringCall = false)
        super.onCleared()
    }

    /** Send a "Voice message not sent" — with ITS reply and caption, and nothing else (S2.8). */
    fun sendNotSent(id: String) {
        val entry = notSent.value.firstOrNull { it.id == id } ?: return
        if (id in _notSentInFlight.value) return
        // Not while the composer is busy with another attachment: its strip's
        // words are that one's live progress, and its state the busy guard.
        if (_mediaState.value.isBusy) return
        viewModelScope.launch {
            // The questions a typed message is asked, of THIS message's words:
            // in the member's own `ai` chat a voice message reaches the model
            // too, and a caption may say `@ai` in the family chat.
            val settingsState = settings.state.first()
            val kind = chat.value?.kind
            if (AssistantConsent.isWithheldFromAnUnnamedAssistant(
                    chatKind = kind,
                    body = entry.caption,
                    hasAssistant = settingsState.assistantUserId != null,
                    processor = settingsState.assistantProcessor,
                )
            ) {
                return@launch
            }
            if (AssistantConsent.isRequired(
                    chatKind = kind,
                    body = entry.caption,
                    processor = settingsState.assistantProcessor,
                    agreedAt = settingsState.assistantConsentAt,
                )
            ) {
                notSentAwaitingConsent = id
                _assistantConsentAsked.value = true
                return@launch
            }
            dispatchNotSent(entry)
        }
    }

    /**
     * Its ✕. Ten seconds or more asks first (S2.8): a recording cannot be
     * made again, and one tap must not be what loses a long one.
     */
    fun deleteNotSent(id: String) {
        val entry = notSent.value.firstOrNull { it.id == id } ?: return
        if (VoiceNoteRules.deleteAsks(entry.durationMs)) {
            _deleteAsk.value = id
            return
        }
        discardNotSent(entry)
    }

    /** "Delete this recording?" answered: Delete, or Keep — which leaves it where it is. */
    fun answerDeleteAsk(delete: Boolean) {
        val id = _deleteAsk.getAndUpdate { null } ?: return
        if (!delete) return
        notSent.value.firstOrNull { it.id == id }?.let(::discardNotSent)
    }

    /** Where a not-sent message's bytes are — what its ▶ plays (S2.8). */
    fun notSentFile(entry: ParkedRecording): File = parked.file(entry)

    /** "Delete this recording?" about a staged voice note answered: Delete, or Keep. */
    fun answerStagedDelete(delete: Boolean) {
        val file = _stagedDeleteAsk.getAndUpdate { null } ?: return
        if (delete) {
            removeStaged(file)
            personChangedStrip()
        }
    }

    private fun discardNotSent(entry: ParkedRecording) {
        if (entry.id in _notSentInFlight.value) return
        // Deleting a recording never deletes words somebody typed: a caption
        // goes back to the field, and the reply with it when nothing else is
        // primed there.
        giveBack(entry.caption, entry.replyTo)
        appScope.launch { parked.remove(entry.id) }
    }

    /** Words and a quote a not-sent message held, back into the composer without clobbering it. */
    private fun giveBack(caption: String, quote: ReplyToDto?) {
        if (_editTarget.value != null) return
        if (caption.isNotBlank()) {
            when (val outcome = MessageBody.appending(caption, inputState.text)) {
                is MessageBody.Paste.Appended -> inputState.setTextAndPlaceCursorAtEnd(outcome.draft)
                is MessageBody.Paste.Truncated -> inputState.setTextAndPlaceCursorAtEnd(outcome.draft)
                MessageBody.Paste.Full -> Unit
            }
        }
        if (quote != null && _replyDraft.value == null) _replyDraft.value = quote
    }

    private fun dispatchNotSent(entry: ParkedRecording) {
        var claimed = false
        _notSentInFlight.update { inFlight ->
            claimed = entry.id !in inFlight
            inFlight + entry.id
        }
        if (!claimed) return
        _mediaState.value = MediaSendState.Preparing
        // App scope, like every media send here: leaving the chat must not
        // take the hand-off with it.
        appScope.launch {
            try {
                val prepared = try {
                    mediaPrep.prepareAudio(Uri.fromFile(parked.file(entry)), voiceNote = true)
                } catch (_: Exception) {
                    _mediaState.value =
                        MediaSendState.Failed(appContext.getString(R.string.e_prepare_failed))
                    return@launch
                }
                val note = prepared.withRecordedLength(entry.durationMs).withWaveform(entry.waveform)
                val queued = messageRepository.sendMedia(
                    listOf(note),
                    entry.caption,
                    chatId,
                    entry.replyTo,
                    resolvedMentions(entry.caption),
                )
                if (queued == null) {
                    note.file.delete()
                    _mediaState.value =
                        MediaSendState.Failed(appContext.getString(R.string.e_send_failed))
                } else {
                    // The outbox owns it from here — its row is written and its
                    // bytes are in the outbox's directory — so the waiting copy
                    // goes.
                    parked.remove(entry.id)
                    _mediaState.value = MediaSendState.Idle
                }
            } finally {
                _notSentInFlight.update { it - entry.id }
            }
        }
    }

    // -- The reducer, fed and carried out --------------------------------------

    /** One step of the shared reducer, and everything it says to do. */
    private fun step(event: RecordGesture.HoldEvent) {
        val before = _hold.value
        val (next, effects) = RecordGesture.holdStep(before, event, holdConstants)
        _hold.value = next
        // The slot's own activation set the guard: what the field held then
        // is what a Send must find for the guard to hold it (sendGuarded).
        if (next.guardUntilMs != before.guardUntilMs) draftAtGuard = inputState.text.toString()
        perform(effects)
    }

    /**
     * The effects, in the reducer's order: the thing, then what is shown, said
     * and felt — except that what the reducer says ABOUT a send or a review
     * (S2.5, S6) waits for the thing to have happened. "Voice message sent" is
     * said once the outbox has the note, never before a hand-off that may still
     * fail and put it back in review; and a recording that turns out to have
     * kept nothing says "too short" instead of "sent" or "Ready to review".
     */
    private fun perform(effects: List<RecordGesture.HoldEffect>) {
        var index = 0
        while (index < effects.size) {
            when (val effect = effects[index]) {
                RecordGesture.HoldEffect.Start -> startMicrophone()
                RecordGesture.HoldEffect.Delete -> {
                    discardRecording()
                    _voiceEffects.tryEmit(VoiceEffect.Ended)
                }
                RecordGesture.HoldEffect.Send -> {
                    val told = toldAbout(effects, index)
                    index += told.size
                    sendRecording(told)
                    _voiceEffects.tryEmit(VoiceEffect.Ended)
                }
                RecordGesture.HoldEffect.Review -> {
                    val told = toldAbout(effects, index)
                    index += told.size
                    // A line said with it lands once the note is staged: staging
                    // writes the strip's state, and would wipe a line written now.
                    val hint = (told.firstOrNull() as? RecordGesture.HoldEffect.Hint)?.hint
                    val rest = if (hint != null) told.drop(1) else told
                    reviewRecording(hint, rest)
                    _voiceEffects.tryEmit(VoiceEffect.Ended)
                }
                RecordGesture.HoldEffect.Park -> park(takeRecording())
                // The recording stops FIRST (S2.5): the question is about a finished one.
                RecordGesture.HoldEffect.AskDelete -> askedRecording = takeRecording()
                RecordGesture.HoldEffect.AskPermission -> _voiceEffects.tryEmit(VoiceEffect.AskPermission)
                // Open Settings only where asking again cannot work (S2.2).
                RecordGesture.HoldEffect.Denied -> failed(
                    R.string.e_microphone_permission,
                    opensSettings = denialPermanent || voiceEnv.permission == RecordGesture.Permission.DENIED,
                )
                is RecordGesture.HoldEffect.Explain -> notice(RecordStrings.of(effect.reason))
                is RecordGesture.HoldEffect.Hint -> showHint(effect.hint)
                is RecordGesture.HoldEffect.Announce -> announce(effect.announcement)
                is RecordGesture.HoldEffect.Haptic -> _voiceEffects.tryEmit(VoiceEffect.Haptic(effect.haptic))
            }
            index++
        }
    }

    /**
     * What the reducer says about [effects]`[index]` — the announcement,
     * haptic and hint it always places right after the thing they
     * describe — for that thing to say once it has happened.
     */
    private fun toldAbout(effects: List<RecordGesture.HoldEffect>, index: Int): List<RecordGesture.HoldEffect> =
        effects.drop(index + 1).takeWhile { it.describesTheEffectBefore() }

    private fun RecordGesture.HoldEffect.describesTheEffectBefore(): Boolean = when (this) {
        is RecordGesture.HoldEffect.Announce,
        is RecordGesture.HoldEffect.Haptic,
        is RecordGesture.HoldEffect.Hint,
        -> true
        else -> false
    }

    /** Whether there is a recording for an effect to take (see [takeRecording]). */
    private fun hasRecording(): Boolean = endedRecording != null || askedRecording != null || recordingHere

    /**
     * An effect meant to send or review a recording found that it kept
     * nothing — the recorder's floor, a recording that never got audio: what
     * really happened is said, in the reducer's own words, instead of what
     * was meant to. Nothing there at all (already gone) says nothing.
     */
    private fun keptNothing(wasThere: Boolean) {
        if (!wasThere) return
        perform(
            listOf(
                RecordGesture.HoldEffect.Hint(RecordGesture.Hint.TOO_SHORT),
                RecordGesture.HoldEffect.Announce(RecordGesture.Announcement.TooShort),
                RecordGesture.HoldEffect.Haptic(RecordGesture.Haptic.WARNING),
            ),
        )
    }

    /**
     * The outbox did not take a voice note. It is in review with [error], or
     * "not sent" — never lost — and that is what is said and felt, never
     * "Voice message sent".
     */
    private fun notQueued(@StringRes error: Int) {
        say(appContext.getString(error))
        _voiceEffects.tryEmit(VoiceEffect.Haptic(RecordGesture.Haptic.WARNING))
    }

    /** The facts a decision reads at the moment it is made. */
    private fun situation(): RecordGesture.Situation = RecordGesture.Situation(
        permission = voiceEnv.permission,
        blocked = blockedReason(),
    )

    /**
     * S1.3's dimmed rows, in their order — asked of the sources, not of
     * [callLive], which reaches this thread a dispatch later: a call placed a
     * moment ago is a call.
     */
    private fun blockedReason(): ComposerSlot.Dimmed? = when {
        calls.state.value is CallState.Live -> ComposerSlot.Dimmed.CALL
        _mediaState.value.isBusy -> ComposerSlot.Dimmed.BUSY
        notSent.value.isNotEmpty() -> ComposerSlot.Dimmed.NOT_SENT
        else -> null
    }

    /** The recorder's own clock: what the person sees, and what a decision reads. */
    private fun recordedNow(): Long =
        endedRecording?.durationMs
            ?: askedRecording?.durationMs
            ?: if (recorderOpen) voiceRecorder.elapsedMs else 0L

    /**
     * Open the microphone: the recording row is already the reducer's. Starting a recording pauses what plays — the screen's
     * players stop on [recordingMs], and the recorder's transient focus
     * pauses everybody else's (S1.7).
     */
    private fun startMicrophone() {
        heardSound = false
        recordingHere = true
        _recordingMs.value = 0
        _voiceLine.value = null
        _voiceLevel.value = 0
        _voiceLevels.value = emptyList()
        _voiceEffects.tryEmit(VoiceEffect.FocusSlot)
        if (voiceEnv.assistive) {
            // The app's own voice stays out of the note (S6): with TalkBack the
            // microphone opens once "Recording" has been spoken — a fixed second,
            // as Android cannot say when an announcement has finished — and the
            // timer starts with it.
            pendingStart = viewModelScope.launch {
                delay(SPEECH_LEAD_MS)
                pendingStart = null
                openRecorder()
            }
        } else {
            openRecorder()
        }
    }

    private fun openRecorder() {
        if (!voiceRecorder.start(recorderOwner)) {
            endRecordingState()
            // The reducer believed it started; nothing did.
            _hold.value = _hold.value.copy(phase = RecordGesture.Phase.Idle)
            failed(R.string.e_record_failed)
            return
        }
        recorderOpen = true
        recordingTicker?.cancel()
        recordingTicker = viewModelScope.launch {
            while (recordingHere) {
                tick()
                delay(VOICE_TICK_MS)
            }
        }
    }

    /** Every 200 ms: the timer, the level meter, and the row's line. */
    private fun tick() {
        val elapsed = voiceRecorder.elapsedMs
        _recordingMs.value = elapsed
        val amplitude = voiceRecorder.maxAmplitude()
        if (VoiceNoteRules.isHeard(amplitude)) heardSound = true
        _voiceLevel.value = VoiceNoteRules.levelBars(amplitude)
        _voiceLevels.value = (_voiceLevels.value + Waveform.level(Waveform.dbfs(amplitude))).takeLast(LIVE_LEVELS)
        refreshLine(elapsed)
    }

    /** The line in the level meter's place — each said once as it appears (S2.5, S2.9). */
    private fun refreshLine(elapsed: Long) {
        val line = when {
            VoiceNoteRules.showsTimeWarning(elapsed) -> VoiceLine.THIRTY_SECONDS_LEFT
            VoiceNoteRules.showsSilenceWarning(elapsed, heardSound) -> VoiceLine.CANT_HEAR
            else -> null
        }
        val before = _voiceLine.value
        _voiceLine.value = line
        if (line != before) {
            when (line) {
                VoiceLine.THIRTY_SECONDS_LEFT -> say(appContext.getString(R.string.s_thirty_seconds_left))
                VoiceLine.CANT_HEAR -> say(appContext.getString(R.string.s_cant_hear_microphone_muted))
                null -> Unit
            }
        }
    }

    /**
     * What a recording left, for the effect that ends it: what the recorder
     * kept when it ended it by itself, the one stopped to be asked about, or
     * this composer's running recording, stopped now.
     */
    private fun takeRecording(): VoiceRecorder.Recording? {
        endedRecording?.let {
            endedRecording = null
            return it
        }
        askedRecording?.let {
            askedRecording = null
            return it
        }
        if (!recordingHere) return null
        val kept = if (recorderOpen) voiceRecorder.stop() else null
        endRecordingState()
        return kept
    }

    /** Delete: stopped, then its file deleted — exactly the file it left. */
    private fun discardRecording() {
        takeRecording()?.file?.delete()
    }

    /**
     * The Send arrow (S2.5): prepared without re-encoding and handed to the
     * outbox, which writes its row before the first byte, with the primed
     * reply.
     */
    private fun sendRecording(told: List<RecordGesture.HoldEffect>) {
        val wasThere = hasRecording()
        val kept = takeRecording()
        if (kept == null) {
            keptNothing(wasThere)
            return
        }
        val quote = _replyDraft.getAndUpdate { null }
        val ticket = reviewTicket()
        appScope.launch {
            when (val handOff = dispatchRecording(kept.file, kept.durationMs, kept.waveform, quote, ticket)) {
                HandOff.Queued -> perform(told)
                is HandOff.Failed -> notQueued(handOff.error)
            }
        }
    }

    /**
     * Prepare a voice note and hand it to the outbox. If preparing fails, or
     * the outbox will not take it, the note lands in review with the error —
     * never lost (S2.5) — or, once the chat has been left, waits as "not sent".
     * Queued once the outbox has it; otherwise Failed with the error shown.
     */
    private suspend fun dispatchRecording(
        file: File,
        durationMs: Long,
        waveform: String?,
        quote: ReplyToDto?,
        ticket: Int?,
    ): HandOff {
        val prepared = try {
            mediaPrep.prepareAudio(Uri.fromFile(file), voiceNote = true)
        } catch (_: Exception) {
            // The recording itself, as recorded: it is the voice-note profile
            // already, which is all preparing would have checked.
            if (file.exists() &&
                backToReview(ticket, rawVoiceNote(file, durationMs, waveform), quote, R.string.e_prepare_failed)
            ) {
                return HandOff.Failed(R.string.e_prepare_failed)
            }
            failed(R.string.e_prepare_failed)
            parked.park(chatId, file, durationMs, quote, caption = "", session = session, waveform = waveform)
            return HandOff.Failed(R.string.e_prepare_failed)
        }
        file.delete()
        val note = prepared.withRecordedLength(durationMs).withWaveform(waveform)
        val queued = messageRepository.sendMedia(listOf(note), "", chatId, quote, null)
        if (queued == null) {
            if (note.file.exists() && backToReview(ticket, note, quote, R.string.e_send_failed)) {
                return HandOff.Failed(R.string.e_send_failed)
            }
            failed(R.string.e_send_failed)
            if (note.file.exists()) {
                parked.park(chatId, note.file, durationMs, quote, caption = "", session = session, waveform = waveform)
            }
            return HandOff.Failed(R.string.e_send_failed)
        }
        return HandOff.Queued
    }

    /**
     * Where a voice note Send could not get out may still land in review
     * (S2.5): the [leaves] it was sent under, or null when leaving is what
     * sent it — the review it would land in is then already "not sent".
     */
    private fun reviewTicket(): Int? = synchronized(reviewLock) { if (leaving) null else leaves }

    /**
     * S2.5's "lands in review with the error": [note] staged beside the
     * field, its reply primed again, and [error] in the notice line — true
     * when it is. False, with nothing done, when the chat has been left since
     * [ticket] (leaving made review "not sent", so that is where it belongs)
     * or the strip is full; the caller then keeps it as "not sent". Under
     * [reviewLock], so leaving either finds it staged or has already counted.
     */
    private fun backToReview(ticket: Int?, note: MediaPrep.Prepared, quote: ReplyToDto?, @StringRes error: Int): Boolean {
        val staged = synchronized(reviewLock) {
            if (ticket == null || ticket != leaves) return false
            stage(note, keepRefused = true).also { staged ->
                if (staged && _editTarget.value == null) _replyDraft.compareAndSet(null, quote)
            }
        }
        if (staged) failed(error)
        return staged
    }

    /** A recording staged as it was recorded — what review holds when preparing it failed. */
    private fun rawVoiceNote(file: File, durationMs: Long, waveform: String?): MediaPrep.Prepared = MediaPrep.Prepared(
        file = file,
        mime = "audio/mp4",
        kind = AttachmentDto.KIND_AUDIO,
        width = null,
        height = null,
        durationMs = durationMs.toInt(),
        previewJpeg = null,
        voiceNote = true,
        waveform = waveform,
    )

    /**
     * Stop, the Stop square, the cap, Keep: staged
     * beside the field, and [told] said — unless the recording kept nothing,
     * which is "too short", never "Ready to review".
     */
    private fun reviewRecording(hint: RecordGesture.Hint?, told: List<RecordGesture.HoldEffect>) {
        val wasThere = hasRecording()
        val kept = takeRecording()
        if (kept == null) {
            keptNothing(wasThere)
            return
        }
        stageRecording(kept, notice = hint?.let(RecordStrings::of))
        perform(told)
    }

    /** Where a voice note handed to the outbox went ([dispatchRecording]). */
    private sealed interface HandOff {
        /** The outbox has it. */
        data object Queued : HandOff

        /** In review with [error], or "not sent" — never lost. */
        data class Failed(@param:StringRes val error: Int) : HandOff
    }

    /**
     * Where a voice note on its way into review ended up. Stopping (S2.5)
     * PREPARES the note on the app's scope before it can be staged, and leaving the chat in that window must still make it "not
     * sent" (S2.8, S4) — so leaving claims it, and it is parked instead.
     */
    private sealed interface Arrival {
        /** In the strip, for review. */
        data object Staged : Arrival

        /** The strip was full (ten attachments): kept as "not sent" instead. */
        data object Refused : Arrival

        /** The chat was left while it was on its way: "not sent", with these. */
        data class Left(val caption: String, val quote: ReplyToDto?) : Arrival
    }

    /** A voice note being prepared for review. [left] is set, under [reviewLock], by leaving. */
    private class ReviewInFlight {
        var left: Arrival.Left? = null
    }

    private fun onItsWayToReview(): ReviewInFlight =
        ReviewInFlight().also { flight -> synchronized(reviewLock) { reviewsInFlight += flight } }

    /** It arrived: [stageIt] — unless the chat was left meanwhile, which says how to park it instead. */
    private fun arrive(flight: ReviewInFlight, stageIt: () -> Boolean): Arrival = synchronized(reviewLock) {
        reviewsInFlight -= flight
        flight.left ?: if (stageIt()) Arrival.Staged else Arrival.Refused
    }

    /** It will not be staged after all (nothing came back, or it could not be prepared): what leaving left it. */
    private fun arrivedNowhere(flight: ReviewInFlight): Arrival.Left? = synchronized(reviewLock) {
        reviewsInFlight -= flight
        flight.left
    }

    /** A line the reducer shows: in the row, or in the composer's notice line. */
    private fun showHint(hint: RecordGesture.Hint) {
        when (hint) {
            RecordGesture.Hint.TOO_SHORT -> failed(RecordStrings.of(hint))
            RecordGesture.Hint.STOPPED_AT_FIVE_MINUTES -> notice(RecordStrings.of(hint))
        }
    }

    private fun announce(announcement: RecordGesture.Announcement) {
        val text = when (announcement) {
            is RecordGesture.Announcement.ReadyToReview -> appContext.getString(
                R.string.s_announce_ready_to_review,
                VoiceNoteRules.clock(announcement.recordedMs),
            )
            else -> appContext.getString(RecordStrings.of(announcement))
        }
        say(text)
    }

    private fun say(text: String) {
        _announcement.value = VoiceAnnouncement(text, ++announcementSerial)
    }

    /** The recorder ended a recording on its own (see [VoiceRecorder.Ending]). */
    private fun onRecorderEnded(ending: VoiceRecorder.Ending, kept: VoiceRecorder.Recording?) {
        // It has stopped by itself: what it kept is what the effects take,
        // rather than a stop() that would find nothing.
        recorderOpen = false
        endRecordingState()
        endedRecording = kept
        val at = uptime.now()
        when (ending) {
            // Into review with "Recording stopped at five minutes.", never sent
            // (S2.5) — unless nothing came back, which is no cap at all.
            VoiceRecorder.Ending.CAP ->
                if (kept != null) {
                    step(RecordGesture.HoldEvent.Cap(at))
                } else {
                    step(RecordGesture.HoldEvent.Interruption(at, recordedMs = 0))
                    failed(R.string.e_recording_stopped_unexpectedly)
                }
            // What is readable becomes "not sent", with the sentence (S4).
            VoiceRecorder.Ending.FAILED -> {
                step(RecordGesture.HoldEvent.Interruption(at, kept?.durationMs ?: 0L))
                failed(R.string.e_recording_stopped_unexpectedly)
            }
            // Another app has the audio, or another chat started a recording.
            VoiceRecorder.Ending.INTERRUPTED,
            VoiceRecorder.Ending.SUPERSEDED,
            -> step(RecordGesture.HoldEvent.Interruption(at, kept?.durationMs ?: 0L))
        }
        // Whatever no effect took is kept: never lost.
        endedRecording?.let { leftover ->
            endedRecording = null
            park(leftover)
        }
    }

    /**
     * Something other than the person stopped it (S4): a recording is kept
     * as "not sent" (under a second, deleted) — never sent — and a pending
     * prompt is let go.
     */
    private fun interruptRecording() {
        step(RecordGesture.HoldEvent.Interruption(uptime.now(), recordedNow()))
    }

    /**
     * The composer is going away — the chat left, another chat opened over
     * it or come on while its own thread covered it, the screen gone for good
     * (S4, "Leaving the chat"). A recording stops into "not sent"; and a voice
     * message still in review — or on its way there — becomes one too, taking
     * the words in the field as its caption (S2.8) — except while a call is
     * live, which keeps what is in review where it is (S4's call column; the
     * call screen coming over the chat is not leaving it). [onCleared] always
     * parks: nothing would keep them after it.
     */
    private fun leave(duringCall: Boolean) {
        if (duringCall) {
            interruptRecording()
            return
        }
        // Counted first: a note Send could not get out after this belongs
        // with "not sent" now, not in a review nobody sees (backToReview).
        synchronized(reviewLock) {
            leaves++
            leaving = true
        }
        try {
            interruptRecording()
        } finally {
            synchronized(reviewLock) { leaving = false }
        }
        parkStagedVoiceNotes()
    }

    /** Park a recording as "not sent", with the reply it was recorded under. */
    private fun park(kept: VoiceRecorder.Recording?) {
        if (kept == null) return
        // Under a second there is nothing worth keeping: deleted, silently.
        if (!VoiceNoteRules.isWorthKeeping(kept.durationMs)) {
            kept.file.delete()
            return
        }
        // Whatever is recorded carries the primed reply (S1.2), and takes it:
        // left primed, it would quote the next text as well.
        val quote = _replyDraft.getAndUpdate { null }
        appScope.launch {
            parked.park(chatId, kept.file, kept.durationMs, quote, caption = "", session = session, waveform = kept.waveform)
        }
    }

    /**
     * Leaving: the voice messages in review become "not sent", the first
     * taking the words and the primed reply — those already staged, and those
     * still on their way into review (a Stop's or a Keep's preparation runs
     * on the app's scope), which leaving claims so that they are parked when
     * they arrive rather than staged into a composer nobody shows again.
     */
    private fun parkStagedVoiceNotes() {
        val (notes, quote, caption) = synchronized(reviewLock) {
            val notes = takeStaged { it.voiceNote }
            val onTheirWay = reviewsInFlight.filter { it.left == null }
            if (notes.isEmpty() && onTheirWay.isEmpty()) return
            val quote = _replyDraft.getAndUpdate { null }
            // Not while an edit has borrowed the field: those words are the
            // message being rewritten, not a caption.
            val words = if (_editTarget.value == null) inputState.text.toString() else ""
            val caption = words.takeIf { it.isNotBlank() } ?: ""
            if (caption.isNotEmpty()) inputState.clearText()
            // The first one takes the words and the reply: a staged note if
            // there is one, else the first still on its way.
            onTheirWay.forEachIndexed { index, flight ->
                val first = notes.isEmpty() && index == 0
                flight.left = Arrival.Left(caption = if (first) caption else "", quote = if (first) quote else null)
            }
            Triple(notes, quote, caption)
        }
        if (notes.isEmpty()) return
        appScope.launch {
            notes.forEachIndexed { index, note ->
                parked.park(
                    chatId = chatId,
                    source = note.file,
                    durationMs = note.durationMs?.toLong() ?: 0L,
                    replyTo = if (index == 0) quote else null,
                    caption = if (index == 0) caption else "",
                    session = session,
                    waveform = note.waveform,
                )
            }
        }
    }

    /** Take the staged items [predicate] picks out, atomically (see [stage]). */
    private fun takeStaged(predicate: (MediaPrep.Prepared) -> Boolean): List<MediaPrep.Prepared> {
        while (true) {
            val current = _staged.value
            val taken = current.filter(predicate)
            if (taken.isEmpty()) return emptyList()
            if (_staged.compareAndSet(current, current.filterNot(predicate))) return taken
        }
    }

    /**
     * Prepare a stopped recording without re-encoding it, and stage it beside
     * the field — or, if the chat is left while it is being prepared, keep it
     * as the "not sent" row leaving makes of a note in review (S2.8, S4).
     */
    private fun stageRecording(kept: VoiceRecorder.Recording, @StringRes notice: Int?) {
        _mediaState.value = MediaSendState.Preparing
        val flight = onItsWayToReview()
        appScope.launch {
            val prepared = try {
                // A voice note is already the protocol's voice-note row; the
                // audio rules that re-encode are for picked files only.
                mediaPrep.prepareAudio(Uri.fromFile(kept.file), voiceNote = true)
            } catch (_: Exception) {
                // Never lost: what could not be staged waits as "not sent".
                val left = arrivedNowhere(flight)
                _mediaState.value =
                    MediaSendState.Failed(appContext.getString(R.string.e_prepare_failed))
                if (left == null) {
                    park(kept)
                } else {
                    parked.park(
                        chatId, kept.file, kept.durationMs, left.quote, left.caption, session = session,
                        waveform = kept.waveform,
                    )
                }
                return@launch
            }
            kept.file.delete()
            val note = prepared.withRecordedLength(kept.durationMs).withWaveform(kept.waveform)
            when (val arrival = arrive(flight) { stage(note, keepRefused = true) }) {
                Arrival.Staged -> if (notice != null) {
                    _mediaState.value = MediaSendState.Notice(appContext.getString(notice))
                }
                // A full strip (ten attachments) does not get to delete it.
                Arrival.Refused -> park(VoiceRecorder.Recording(note.file, kept.durationMs, kept.waveform))
                is Arrival.Left -> {
                    // The strip is not busy with it any more, whoever comes back.
                    _mediaState.compareAndSet(MediaSendState.Preparing, MediaSendState.Idle)
                    parked.park(
                        chatId, note.file, kept.durationMs, arrival.quote, arrival.caption, session = session,
                        waveform = kept.waveform,
                    )
                }
            }
        }
    }

    private fun endRecordingState() {
        recordingHere = false
        recorderOpen = false
        recordingTicker?.cancel()
        recordingTicker = null
        pendingStart?.cancel()
        pendingStart = null
        _recordingMs.value = null
        _voiceLine.value = null
        _voiceLevel.value = 0
        _voiceLevels.value = emptyList()
    }

    /**
     * A sentence in the composer's notice line (S1.3's "says why") — or, while
     * the strip is busy with an attachment's live progress, a toast, so the
     * progress is not overwritten and the sentence is still said.
     */
    private fun notice(@StringRes message: Int) {
        val text = appContext.getString(message)
        if (_mediaState.value.isBusy) {
            _transientMessages.tryEmit(text)
            return
        }
        _mediaState.value = MediaSendState.Notice(text)
    }

    /** An error in the notice line, likewise; [opensSettings] adds Open Settings. */
    private fun failed(@StringRes message: Int, opensSettings: Boolean = false) {
        val text = appContext.getString(message)
        if (_mediaState.value.isBusy) {
            _transientMessages.tryEmit(text)
            return
        }
        _mediaState.value = MediaSendState.Failed(text, opensSettings)
    }

    /** The recorder's own clock, where the file cannot say how long it is. */
    private fun MediaPrep.Prepared.withRecordedLength(recordedMs: Long): MediaPrep.Prepared =
        if (durationMs != null) this else copy(durationMs = recordedMs.toInt())

    /** The recorder's waveform (#79), which preparing the file cannot know: it is the meter's. */
    private fun MediaPrep.Prepared.withWaveform(waveform: String?): MediaPrep.Prepared =
        if (waveform == null) this else copy(waveform = waveform)

    /**
     * Prepare and send a picked document. Nothing is re-encoded — a file
     * goes as it is (protocol.md, "Files").
     */
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
                    // than a document row, and so does audio the audio rules
                    // re-encode into a type it knows (FLAC); anything else it
                    // would refuse falls through to the file path, where
                    // nothing is verified.
                    if (declared in MediaPrep.SENDABLE_AUDIO_TYPES ||
                        declared in MediaPrep.TRANSCODABLE_AUDIO_TYPES
                    ) {
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
                personChangedStrip()
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
                    personChangedStrip()
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

    /**
     * A voice message's Save (#79): download if needed, then copy into the
     * document the person just created through the system's own save screen
     * (ACTION_CREATE_DOCUMENT) — Android's "Save to Files". That screen is
     * the confirmation; a failure says so in the strip. True when saved.
     */
    suspend fun saveToDocument(attachment: AttachmentDto, destination: Uri): Boolean {
        _mediaState.value = MediaSendState.Working(appContext.getString(R.string.s_preparing))
        val file = attachments.fileFor(attachment)
        if (file == null) {
            // The save screen has already made the document: an empty file
            // with the recording's name must not be left where it was saved.
            discardDocument(destination)
            _mediaState.value = MediaSendState.Failed(appContext.getString(R.string.e_download_to_save_failed))
            return false
        }
        val copied = withContext(Dispatchers.IO) {
            runCatching {
                appContext.contentResolver.openOutputStream(destination, "w")?.use { output ->
                    file.inputStream().use { input -> input.copyTo(output) }
                } != null
            }.getOrDefault(false)
        }
        if (!copied) discardDocument(destination)
        _mediaState.value = if (copied) {
            MediaSendState.Idle
        } else {
            MediaSendState.Failed(appContext.getString(R.string.e_save_failed))
        }
        return copied
    }

    /**
     * Remove a document the system's save screen made but nothing was put
     * in — a failed Save, or one whose recording was lost. Best effort: a
     * provider that refuses keeps it, and there is nothing more to say.
     */
    suspend fun discardDocument(destination: Uri) {
        withContext(Dispatchers.IO) {
            runCatching {
                if (destination.scheme == android.content.ContentResolver.SCHEME_FILE) {
                    destination.path?.let { File(it).delete() }
                } else {
                    android.provider.DocumentsContract.deleteDocument(appContext.contentResolver, destination)
                }
            }
        }
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

        /**
         * How long a screen rebuilt by a configuration change has to come
         * back before this chat counts as left (see [screenDetached]).
         */
        const val ORPHAN_GRACE_MS = 5_000L

        /**
         * With TalkBack running, how long the microphone waits for "Recording"
         * to be spoken (#79, S6: "a fixed 1 s" where the platform cannot say
         * when an announcement has finished).
         */
        const val SPEECH_LEAD_MS = 1_000L

        /** The recording's timer, level meter and line refresh this often (S2.9's 200 ms tick). */
        const val VOICE_TICK_MS = 200L

        /** How many of the newest peaks the live waveform keeps: more than the widest row draws. */
        const val LIVE_LEVELS = 96

        const val READ_DEBOUNCE_MS = 500L
        const val TYPING_THROTTLE_MS = 3_000L
        const val TYPING_EXPIRY_MS = 5_000L
    }
}
