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
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.net.LinkPreviewRepository
import me.nettrash.familyconnect.data.net.LinkPreviewState
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.repo.ChatRepository
import android.content.Context
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.data.repo.GallerySaver
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.Clock
import java.io.File
import javax.inject.Inject

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
    private val settings: SettingsRepository,
    private val socket: ChatSocket,
    private val clock: Clock,
    private val linkPreviewRepository: LinkPreviewRepository,
    private val mediaPrep: MediaPrep,
    private val attachmentApi: AttachmentApi,
    private val attachments: AttachmentRepository,
    private val gallerySaver: GallerySaver,
    @param:AppScope private val appScope: CoroutineScope,
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

    /** userId → profile-picture version, for the avatars beside reactors. */
    val memberAvatars: StateFlow<Map<Long, Long>> = memberDao.observeMembers()
        .map { members -> members.associate { it.userId to it.avatarVersion } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

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

    /** Composition asks for a link the first time a bubble renders it. */
    fun requestLinkPreview(url: String) {
        if (!linkPreviewsEnabled.value) return
        linkPreviewRepository.request(url)
    }

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
    }

    private val _mediaState = MutableStateFlow<MediaSendState>(MediaSendState.Idle)

    /** Drives the input bar's busy strip. */
    val mediaState: StateFlow<MediaSendState> = _mediaState

    /** Media prepared and waiting for Send; null when there is none. */
    private val _staged = MutableStateFlow<MediaPrep.Prepared?>(null)
    val staged: StateFlow<MediaPrep.Prepared?> = _staged

    /** Screen calls this from a LifecycleResumeEffect. */
    fun setResumed(isResumed: Boolean) {
        resumed.value = isResumed
        chatRepository.setOpenChat(if (isResumed) chatId else null)
    }

    fun send() {
        val body = inputState.text.toString()
        if (body.isBlank()) return
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
    fun stageMedia(uri: Uri, isVideo: Boolean) {
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
            val prepared = try {
                if (isVideo) mediaPrep.prepareVideo(uri) else mediaPrep.preparePhoto(uri)
            } catch (_: MediaPrep.TooLargeAfterCompression) {
                // The one failure the user can act on, so it says what
                // would help rather than just refusing.
                _mediaState.value = MediaSendState.Failed(
                    appContext.getString(R.string.e_still_too_large),
                )
                return@launch
            } catch (_: Exception) {
                _mediaState.value = MediaSendState.Failed(appContext.getString(R.string.e_prepare_failed))
                return@launch
            }

            stage(prepared)
        }
    }

    /**
     * Hold prepared media in the composer until the user presses Send.
     *
     * Picking used to send immediately, so a caption had to be typed BEFORE
     * choosing the photo and there was no way to back out once picked. One
     * attachment per message, so a second pick replaces the first rather
     * than queueing behind it.
     */
    private fun stage(prepared: MediaPrep.Prepared) {
        discardStaged()
        _staged.value = prepared
        _mediaState.value = MediaSendState.Idle
    }

    /**
     * A destination for the camera to write into, as a FileProvider Uri.
     *
     * The capture intents write into a Uri the CALLER provides — which is
     * what keeps this app off `android.permission.CAMERA` entirely: it never
     * touches the camera, it hands over a file and gets a result back.
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

    /** Throw away staged media and the temp file nothing else will clean up. */
    fun discardStaged() {
        _staged.value?.file?.delete()
        _staged.value = null
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
    fun stageFile(uri: Uri) {
        if (_mediaState.value == MediaSendState.Preparing ||
            _mediaState.value == MediaSendState.Uploading
        ) {
            return
        }
        _mediaState.value = MediaSendState.Preparing
        // App scope for the same reason as sendMedia.
        appScope.launch {
            val prepared = try {
                mediaPrep.prepareFile(uri)
            } catch (_: MediaPrep.TooLargeAfterCompression) {
                // A document cannot be compressed the way a video can, so
                // the advice is different: there is nothing to try.
                _mediaState.value = MediaSendState.Failed(appContext.getString(R.string.e_file_too_large))
                return@launch
            } catch (_: Exception) {
                _mediaState.value = MediaSendState.Failed(appContext.getString(R.string.e_read_file_failed))
                return@launch
            }

            stage(prepared)
        }
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
