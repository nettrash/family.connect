/*
 * Fakes.kt
 * Family Connect (Android) — test fixtures
 *
 * Hand-rolled fakes for every seam the production code exposes as an
 * interface. No mocking library: the fakes are tiny, the behavior under
 * test is interaction *order* (frames, acks, retries), and recorded
 * lists read better in assertions than verify() DSLs.
 */

package me.nettrash.familyconnect.testutil

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import me.nettrash.familyconnect.data.db.LocalDataWiper
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AuthApi
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.AvatarApi
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.net.FamilyApi
import me.nettrash.familyconnect.data.net.dto.ApproveResponse
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentResponse
import me.nettrash.familyconnect.data.net.dto.AuthResponse
import me.nettrash.familyconnect.data.net.dto.AvatarResponse
import me.nettrash.familyconnect.data.net.dto.ChatResponse
import me.nettrash.familyconnect.data.net.dto.ChatsResponse
import me.nettrash.familyconnect.data.net.dto.DeviceResponse
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.net.dto.FamilyStatsDto
import me.nettrash.familyconnect.data.net.dto.JoinRequestsResponse
import me.nettrash.familyconnect.data.net.dto.JoinResponse
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.net.dto.MessageReactionStateDto
import me.nettrash.familyconnect.data.net.dto.MessageResponse
import me.nettrash.familyconnect.data.net.dto.MessagesResponse
import me.nettrash.familyconnect.data.net.BoardApi
import me.nettrash.familyconnect.data.net.dto.BoardChangesResponse
import me.nettrash.familyconnect.data.net.dto.BoardResponse
import me.nettrash.familyconnect.data.net.dto.NoteDto
import me.nettrash.familyconnect.data.net.dto.NoteResponse
import me.nettrash.familyconnect.data.net.dto.PatchNoteRequest
import me.nettrash.familyconnect.data.net.dto.ReactionDto
import me.nettrash.familyconnect.data.net.dto.ReactionsCatchUpResponse
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.net.dto.RotateInviteCodeResponse
import me.nettrash.familyconnect.data.net.dto.UserDto
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.data.settings.TokenStore
import java.io.File

class FakeTokenStore(initial: String? = null) : TokenStore {
    var token: String? = initial
        private set

    override fun load(): String? = token

    override fun save(token: String) {
        this.token = token
    }

    override fun clear() {
        token = null
    }
}

class FakeSettingsRepository(initial: SettingsState = SettingsState()) : SettingsRepository {
    private val _state = MutableStateFlow(initial)
    override val state: Flow<SettingsState> = _state

    val current: SettingsState get() = _state.value

    override suspend fun setServerUrl(url: String) {
        _state.value = _state.value.copy(serverUrl = url)
    }

    override suspend fun setFamilyStatus(status: FamilyStatus) {
        _state.value = _state.value.copy(familyStatus = status)
    }

    override suspend fun setProfile(
        userId: Long,
        username: String,
        displayName: String,
        avatarVersion: Long,
    ) {
        _state.value = _state.value.copy(
            myUserId = userId,
            myUsername = username,
            myDisplayName = displayName,
            myAvatarVersion = avatarVersion,
        )
    }

    override suspend fun setMyAvatarVersion(version: Long) {
        _state.value = _state.value.copy(myAvatarVersion = version)
    }

    override suspend fun setFamilyName(name: String?) {
        _state.value = _state.value.copy(familyName = name)
    }

    override suspend fun setPushToken(token: String?) {
        _state.value = _state.value.copy(pushToken = token)
    }

    override suspend fun setPushDeviceId(deviceId: Long?) {
        _state.value = _state.value.copy(pushDeviceId = deviceId)
    }

    override suspend fun setLinkPreviewsEnabled(enabled: Boolean) {
        _state.value = _state.value.copy(linkPreviewsEnabled = enabled)
    }

    override suspend fun setBoardCursor(seq: Long) {
        _state.value = _state.value.copy(boardCursor = seq)
    }

    override suspend fun setBoardSeenNoteId(noteId: Long) {
        // Never backwards, matching the real store.
        if (noteId > _state.value.boardSeenNoteId) {
            _state.value = _state.value.copy(boardSeenNoteId = noteId)
        }
    }

    override suspend fun setMapPreviewsEnabled(enabled: Boolean) {
        _state.value = _state.value.copy(mapPreviewsEnabled = enabled)
    }

    override suspend fun setAssistant(userId: Long?, displayName: String?) {
        _state.value = _state.value.copy(
            assistantUserId = userId,
            assistantName = displayName,
        )
    }

    override suspend fun resetKeepingServerUrl() {
        // Mirrors production: server URL AND the device-scoped FCM token
        // survive; the account-scoped device id does not.
        _state.value = SettingsState(
            serverUrl = _state.value.serverUrl,
            pushToken = _state.value.pushToken,
        )
    }
}

class RecordingWiper : LocalDataWiper {
    var wipeCount = 0
        private set

    override suspend fun wipeAll() {
        wipeCount += 1
    }
}

class FakeChatSocket : ChatSocket {
    private val _frames = MutableSharedFlow<ServerFrame>(
        extraBufferCapacity = 64,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    override val frames: SharedFlow<ServerFrame> = _frames

    private val _state = MutableStateFlow(SocketState.Disconnected)
    override val state: StateFlow<SocketState> = _state

    val sent = mutableListOf<ClientFrame>()

    /** When false, trySend reports failure even while Open. */
    var sendSucceeds = true

    override fun connect(wsUrl: String, token: String) {
        _state.value = SocketState.Open
    }

    override fun close(code: Int, reason: String) {
        _state.value = SocketState.Disconnected
    }

    override fun trySend(frame: ClientFrame): Boolean {
        if (_state.value != SocketState.Open || !sendSucceeds) return false
        sent += frame
        return true
    }

    fun setOpen(open: Boolean) {
        _state.value = if (open) SocketState.Open else SocketState.Disconnected
    }

    fun emit(frame: ServerFrame) {
        check(_frames.tryEmit(frame)) { "frame buffer full" }
    }
}

class FakeConnectivityObserver(online: Boolean = true) : ConnectivityObserver {
    override val isOnline = MutableStateFlow(online)
    override val onAvailable = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
}

/** Scripted AuthApi — assign the result fields per test. */
class FakeAuthApi : AuthApi {
    var registerResult: ApiResult<AuthResponse> = ApiResult.NetworkError(IllegalStateException("unscripted"))
    var loginResult: ApiResult<AuthResponse> = ApiResult.NetworkError(IllegalStateException("unscripted"))
    var meResult: ApiResult<MeResponse> = ApiResult.NetworkError(IllegalStateException("unscripted"))
    var probeResult: ApiResult<MeResponse> = ApiResult.NetworkError(IllegalStateException("unscripted"))
    var deviceResult: ApiResult<DeviceResponse> = ApiResult.Ok(DeviceResponse(deviceId = 1))
    var deleteDeviceResult: ApiResult<Unit> = ApiResult.Ok(Unit)

    var registerCalls = 0
    var loginCalls = 0
    var meCalls = 0
    var deviceCalls = 0
    var logoutCalls = 0

    /** Every push_token handed to POST /devices, in call order. */
    val deviceRegistrations = mutableListOf<String?>()

    /** Every id handed to DELETE /devices/{id}, in call order. */
    val deletedDeviceIds = mutableListOf<Long>()

    override suspend fun register(
        username: String,
        displayName: String,
        password: String,
    ): ApiResult<AuthResponse> {
        registerCalls += 1
        return registerResult
    }

    override suspend fun login(username: String, password: String): ApiResult<AuthResponse> {
        loginCalls += 1
        return loginResult
    }

    /** Every password change this fake saw: (current, new). */
    val passwordChanges = mutableListOf<Pair<String, String>>()
    var changePasswordHandler: (String, String) -> ApiResult<Unit> = { _, _ -> ApiResult.Ok(Unit) }

    override suspend fun changePassword(current: String, new: String): ApiResult<Unit> {
        passwordChanges += current to new
        return changePasswordHandler(current, new)
    }

    override suspend fun logout(): ApiResult<Unit> {
        logoutCalls += 1
        return ApiResult.Ok(Unit)
    }

    override suspend fun me(): ApiResult<MeResponse> {
        meCalls += 1
        return meResult
    }

    override suspend fun probe(candidateServerUrl: String): ApiResult<MeResponse> = probeResult

    override suspend fun registerDevice(pushToken: String?): ApiResult<DeviceResponse> {
        deviceCalls += 1
        deviceRegistrations += pushToken
        return deviceResult
    }

    override suspend fun deleteDevice(deviceId: Long): ApiResult<Unit> {
        deletedDeviceIds += deviceId
        return deleteDeviceResult
    }
}

/** Scripted ChatApi. `postMessageHandler` lets tests vary per call. */
class FakeChatApi : ChatApi {
    var chatsResult: ApiResult<ChatsResponse> = ApiResult.Ok(ChatsResponse(emptyList()))
    var createDirectResult: ApiResult<ChatResponse> = ApiResult.NetworkError(IllegalStateException("unscripted"))
    var messagesHandler: (chatId: Long, beforeId: Long?, afterId: Long?, limit: Int) -> ApiResult<MessagesResponse> =
        { _, _, _, _ -> ApiResult.Ok(MessagesResponse(emptyList())) }
    var postMessageHandler: (chatId: Long, clientMsgId: String, body: String) -> ApiResult<MessageResponse> =
        { _, _, _ -> ApiResult.NetworkError(IllegalStateException("unscripted")) }

    // Suspend-typed so a test can park the REST call on a gate and
    // observe the optimistic state mid-flight.
    var putReactionHandler: suspend (chatId: Long, messageId: Long, emoji: String) -> ApiResult<MessageReactionStateDto> =
        { _, _, _ -> ApiResult.NetworkError(IllegalStateException("unscripted")) }
    var deleteReactionHandler: suspend (chatId: Long, messageId: Long) -> ApiResult<MessageReactionStateDto> =
        { _, _ -> ApiResult.NetworkError(IllegalStateException("unscripted")) }
    var reactionsHandler: (chatId: Long, afterSeq: Long, limit: Int) -> ApiResult<ReactionsCatchUpResponse> =
        { _, _, _ -> ApiResult.Ok(ReactionsCatchUpResponse(emptyList())) }

    val postedMessages = mutableListOf<Triple<Long, String, String>>()
    val postedReads = mutableListOf<Pair<Long, Long>>()
    val putReactions = mutableListOf<Triple<Long, Long, String>>()
    val deletedReactions = mutableListOf<Pair<Long, Long>>()
    var messagesCalls = 0
    var reactionsCalls = 0

    override suspend fun chats(): ApiResult<ChatsResponse> = chatsResult

    override suspend fun createDirect(userId: Long): ApiResult<ChatResponse> = createDirectResult

    override suspend fun messages(
        chatId: Long,
        beforeId: Long?,
        afterId: Long?,
        limit: Int,
    ): ApiResult<MessagesResponse> {
        messagesCalls += 1
        return messagesHandler(chatId, beforeId, afterId, limit)
    }

    /** Every reply target a REST send carried, in order. */
    val postedReplyTargets = mutableListOf<Long?>()

    /** Every attachment id a REST send carried, in order. */
    val postedAttachmentIds = mutableListOf<Long?>()

    override suspend fun postMessage(
        chatId: Long,
        clientMsgId: String,
        body: String,
        replyToMessageId: Long?,
        attachmentId: Long?,
    ): ApiResult<MessageResponse> {
        postedMessages += Triple(chatId, clientMsgId, body)
        postedReplyTargets += replyToMessageId
        postedAttachmentIds += attachmentId
        return postMessageHandler(chatId, clientMsgId, body)
    }

    override suspend fun postRead(chatId: Long, lastReadMessageId: Long): ApiResult<Unit> {
        postedReads += chatId to lastReadMessageId
        return ApiResult.Ok(Unit)
    }

    /** Every (messageId, body) an edit was attempted with, in order. */
    val edits = mutableListOf<Pair<Long, String>>()
    var editHandler: (Long, String) -> ApiResult<MessageResponse> = { id, body ->
        ApiResult.Ok(MessageResponse(messageDto(id = id, body = body, editSeq = 1)))
    }

    override suspend fun editMessage(
        chatId: Long,
        messageId: Long,
        body: String,
    ): ApiResult<MessageResponse> {
        edits += messageId to body
        return editHandler(messageId, body)
    }

    /** Pages the edit catch-up will serve, oldest first. */
    var editPages: MutableList<List<MessageDto>> = mutableListOf()

    override suspend fun getEdits(
        chatId: Long,
        afterSeq: Long,
        limit: Int,
    ): ApiResult<MessagesResponse> =
        ApiResult.Ok(MessagesResponse(if (editPages.isEmpty()) emptyList() else editPages.removeAt(0)))

    override suspend fun putReaction(
        chatId: Long,
        messageId: Long,
        emoji: String,
    ): ApiResult<MessageReactionStateDto> {
        putReactions += Triple(chatId, messageId, emoji)
        return putReactionHandler(chatId, messageId, emoji)
    }

    override suspend fun deleteReaction(
        chatId: Long,
        messageId: Long,
    ): ApiResult<MessageReactionStateDto> {
        deletedReactions += chatId to messageId
        return deleteReactionHandler(chatId, messageId)
    }

    override suspend fun getReactions(
        chatId: Long,
        afterSeq: Long,
        limit: Int,
    ): ApiResult<ReactionsCatchUpResponse> {
        reactionsCalls += 1
        return reactionsHandler(chatId, afterSeq, limit)
    }
}

class FakeFamilyApi : FamilyApi {
    var mineResult: ApiResult<FamilyMineResponse> = ApiResult.NetworkError(IllegalStateException("unscripted"))
    var createResult: ApiResult<FamilyResponse> = ApiResult.NetworkError(IllegalStateException("unscripted"))
    var joinResult: ApiResult<JoinResponse> = ApiResult.NetworkError(IllegalStateException("unscripted"))
    var joinRequestsResult: ApiResult<JoinRequestsResponse> = ApiResult.Ok(JoinRequestsResponse(emptyList()))

    override suspend fun create(name: String): ApiResult<FamilyResponse> = createResult
    override suspend fun join(inviteCode: String): ApiResult<JoinResponse> = joinResult
    override suspend fun mine(): ApiResult<FamilyMineResponse> = mineResult

    var statsResult: ApiResult<FamilyStatsDto> =
        ApiResult.NetworkError(IllegalStateException("unscripted"))

    /** How many times the statistics were fetched — they are never cached. */
    var statsCalls = 0

    override suspend fun stats(): ApiResult<FamilyStatsDto> {
        statsCalls += 1
        return statsResult
    }
    override suspend fun rotateInviteCode(): ApiResult<RotateInviteCodeResponse> =
        ApiResult.Ok(RotateInviteCodeResponse("NEWCODE1"))

    override suspend fun setJoinPolicy(policy: String): ApiResult<FamilyResponse> = createResult
    override suspend fun joinRequests(): ApiResult<JoinRequestsResponse> = joinRequestsResult
    override suspend fun approve(requestId: Long): ApiResult<ApproveResponse> =
        ApiResult.NetworkError(IllegalStateException("unscripted"))

    override suspend fun reject(requestId: Long): ApiResult<Unit> = ApiResult.Ok(Unit)
    override suspend fun leave(): ApiResult<Unit> = ApiResult.Ok(Unit)
    override suspend fun removeMember(userId: Long): ApiResult<Unit> = ApiResult.Ok(Unit)

    /** Every reset this fake saw: (userId, newPassword). */
    val passwordResets = mutableListOf<Pair<Long, String>>()

    override suspend fun resetMemberPassword(
        userId: Long,
        newPassword: String,
    ): ApiResult<Unit> {
        passwordResets += userId to newPassword
        return ApiResult.Ok(Unit)
    }
}

class FakeAvatarApi : AvatarApi {
    /** Every (userId, version) asked for, in order — the dedup assertions read this. */
    val fetches = mutableListOf<Pair<Long, Long>>()
    val uploads = mutableListOf<ByteArray>()
    var deletes = 0

    var uploadResult: ApiResult<AvatarResponse> = ApiResult.Ok(AvatarResponse(userDto(1).copy(avatarVersion = 1)))
    var deleteResult: ApiResult<Unit> = ApiResult.Ok(Unit)

    /** Scripted per call so a test can 404 one user and serve another. */
    var onFetch: (Long, Long) -> ApiResult<ByteArray> = { _, _ -> ApiResult.Ok(ByteArray(8)) }

    /**
     * Set to hold every fetch open until the test completes it — the way
     * to have a request still in flight when something else happens
     * (the caller is cancelled, the session ends).
     */
    var gate: CompletableDeferred<Unit>? = null

    override suspend fun upload(jpeg: ByteArray): ApiResult<AvatarResponse> {
        uploads += jpeg
        return uploadResult
    }

    override suspend fun delete(): ApiResult<Unit> {
        deletes++
        return deleteResult
    }

    override suspend fun fetch(userId: Long, version: Long): ApiResult<ByteArray> {
        fetches += userId to version
        gate?.await()
        return onFetch(userId, version)
    }
}

/** Scripted BoardApi — assign the result fields per test. */
class FakeBoardApi : BoardApi {
    var board: BoardResponse = BoardResponse(emptyList(), 0)
    /** Pages the catch-up will serve, oldest first. */
    var changePages: MutableList<List<NoteDto>> = mutableListOf()
    val created = mutableListOf<Triple<String, String, Pair<Double, Double>>>()
    val patched = mutableListOf<Pair<Long, PatchNoteRequest>>()
    val deleted = mutableListOf<Long>()

    var createResult: ((NoteDto) -> ApiResult<NoteResponse>)? = null
    var nextSeq = 1L

    override suspend fun getBoard(): ApiResult<BoardResponse> = ApiResult.Ok(board)

    override suspend fun getBoardChanges(
        afterSeq: Long,
        limit: Int,
    ): ApiResult<BoardChangesResponse> =
        ApiResult.Ok(
            BoardChangesResponse(if (changePages.isEmpty()) emptyList() else changePages.removeAt(0)),
        )

    override suspend fun createNote(
        text: String,
        color: String,
        x: Double,
        y: Double,
    ): ApiResult<NoteResponse> {
        created += Triple(text, color, x to y)
        val note = noteDto(id = nextSeq, text = text, color = color, x = x, y = y, boardSeq = nextSeq)
        nextSeq++
        return createResult?.invoke(note) ?: ApiResult.Ok(NoteResponse(note))
    }

    override suspend fun patchNote(
        id: Long,
        text: String?,
        color: String?,
        x: Double?,
        y: Double?,
    ): ApiResult<NoteResponse> {
        patched += id to PatchNoteRequest(text, color, x, y)
        val note = noteDto(
            id = id,
            text = text ?: "note $id",
            color = color ?: "yellow",
            x = x ?: 0.0,
            y = y ?: 0.0,
            boardSeq = nextSeq++,
        )
        return ApiResult.Ok(NoteResponse(note))
    }

    override suspend fun deleteNote(id: Long): ApiResult<Unit> {
        deleted += id
        return ApiResult.Ok(Unit)
    }
}

// -- DTO builders ----------------------------------------------------------

fun noteDto(
    id: Long,
    authorId: Long = 7L,
    text: String = "note $id",
    color: String = "yellow",
    x: Double = 0.2,
    y: Double = 0.3,
    boardSeq: Long,
    deleted: Boolean? = null,
) = NoteDto(
    id = id,
    authorId = if (deleted == true) null else authorId,
    text = if (deleted == true) null else text,
    color = if (deleted == true) null else color,
    x = if (deleted == true) null else x,
    y = if (deleted == true) null else y,
    createdAt = if (deleted == true) null else "2026-08-22T12:00:00Z",
    updatedAt = if (deleted == true) null else "2026-08-22T12:00:00Z",
    boardSeq = boardSeq,
    deleted = deleted,
)

/** A tombstone: id and seq only, which is all the server sends. */
fun noteTombstone(id: Long, boardSeq: Long) = noteDto(id = id, boardSeq = boardSeq, deleted = true)

fun userDto(id: Long, name: String = "user$id") = UserDto(
    id = id,
    username = name,
    displayName = name.replaceFirstChar { it.uppercase() },
    createdAt = "2026-08-01T10:00:00Z",
)

fun messageDto(
    id: Long,
    chatId: Long = 42L,
    senderId: Long = 9L,
    clientMsgId: String = "srv-$id",
    body: String = "msg $id",
    createdAt: String = "2026-08-19T17:03:12Z",
    reactions: List<ReactionDto>? = null,
    reactionSeq: Long? = null,
    replyTo: ReplyToDto? = null,
    editSeq: Long? = null,
    editedAt: String? = null,
) = MessageDto(
    id = id,
    chatId = chatId,
    senderId = senderId,
    clientMsgId = clientMsgId,
    body = body,
    createdAt = createdAt,
    reactions = reactions,
    reactionSeq = reactionSeq,
    replyTo = replyTo,
    editSeq = editSeq,
    editedAt = editedAt,
)

fun reactionState(
    messageId: Long,
    reactionSeq: Long,
    reactions: List<ReactionDto> = emptyList(),
) = MessageReactionStateDto(
    messageId = messageId,
    reactionSeq = reactionSeq,
    reactions = reactions,
)

/**
 * Scriptable AttachmentApi. Records what was uploaded, in order, so the
 * send-order invariant (bytes, then preview, then the message) can be
 * asserted without a server.
 */
class FakeAttachmentApi : AttachmentApi {

    /** What `upload` answers with; a failure by default would hide bugs. */
    var uploadHandler: (File, String, String) -> ApiResult<AttachmentResponse> = { _, _, _ ->
        ApiResult.Ok(AttachmentResponse(attachment(id = 34)))
    }
    var previewHandler: (Long, ByteArray) -> ApiResult<Unit> = { _, _ -> ApiResult.Ok(Unit) }

    /** Every call this fake saw, in order: "upload", "preview", "download". */
    val calls = mutableListOf<String>()
    val uploadedFiles = mutableListOf<File>()
    val uploadedMetadata = mutableListOf<Triple<String, Int?, Int?>>()
    /** Every name a file upload carried, in order. */
    val uploadedNames = mutableListOf<String?>()
    val uploadedPreviews = mutableListOf<Pair<Long, Int>>()

    override suspend fun upload(
        file: File,
        mime: String,
        kind: String,
        width: Int?,
        height: Int?,
        durationMs: Int?,
        name: String?,
    ): ApiResult<AttachmentResponse> {
        calls += "upload"
        uploadedFiles += file
        uploadedMetadata += Triple(kind, width, height)
        uploadedNames += name
        return uploadHandler(file, mime, kind)
    }

    /** Every location this fake was asked to send, in order. */
    val uploadedLocations = mutableListOf<Triple<Double, Double, Int?>>()
    var locationHandler: (Double, Double) -> ApiResult<AttachmentResponse> = { lat, lon ->
        ApiResult.Ok(
            AttachmentResponse(
                attachment(id = 61).copy(
                    kind = AttachmentDto.KIND_LOCATION,
                    mime = "application/vnd.family-connect.location",
                    size = 0,
                    latitude = lat,
                    longitude = lon,
                ),
            ),
        )
    }

    override suspend fun uploadLocation(
        latitude: Double,
        longitude: Double,
        accuracyM: Int?,
        name: String?,
    ): ApiResult<AttachmentResponse> {
        calls += "uploadLocation"
        uploadedLocations += Triple(latitude, longitude, accuracyM)
        uploadedNames += name
        return locationHandler(latitude, longitude)
    }

    override suspend fun uploadPreview(attachmentId: Long, jpeg: ByteArray): ApiResult<Unit> {
        calls += "preview"
        uploadedPreviews += attachmentId to jpeg.size
        return previewHandler(attachmentId, jpeg)
    }

    override suspend fun download(
        attachmentId: Long,
        preview: Boolean,
        destination: File,
    ): ApiResult<Unit> {
        calls += "download"
        return ApiResult.HttpError(404, "attachment_not_found", "no")
    }

    override suspend fun streamUrl(attachmentId: Long): Pair<String, Map<String, String>>? =
        "https://home.example/api/v1/attachments/$attachmentId" to
            mapOf("Authorization" to "Bearer test")

    companion object {
        fun attachment(
            id: Long = 34,
            kind: String = "photo",
            hasPreview: Boolean = false,
            name: String? = null,
        ) = AttachmentDto(
            id = id,
            kind = kind,
            mime = when (kind) {
                "video" -> "video/mp4"
                "file" -> "application/pdf"
                else -> "image/jpeg"
            },
            size = 4096,
            // A file has no shape and no preview to reserve one from.
            width = if (kind == "file") null else 1600,
            height = if (kind == "file") null else 1200,
            durationMs = if (kind == "video") 8400 else null,
            hasPreview = hasPreview,
            name = name ?: if (kind == "file") "receipts.pdf" else null,
        )
    }
}
