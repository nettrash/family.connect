/*
 * ChatListViewModel.kt
 * Family Connect (Android)
 *
 * Renders straight from Room (ChatDao.observeChats already orders:
 * family chat pinned, then most recent activity). Network work is
 * refresh-on-resume plus whatever the socket manager's resyncs pull in.
 * The new-chat picker lists family members minus me; picking one runs
 * the idempotent POST /chats/direct and emits the chat id to navigate.
 *
 * iOS counterpart: ios/FamilyConnect/UI/ChatList/ChatListViewModel.swift
 */

package me.nettrash.familyconnect.ui.chatlist

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import me.nettrash.familyconnect.R
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.db.NoteDao
import kotlinx.coroutines.flow.first
import me.nettrash.familyconnect.data.repo.ChatRepository
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import javax.inject.Inject

@HiltViewModel
class ChatListViewModel @Inject constructor(
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
    private val chatRepository: ChatRepository,
    familyRepository: FamilyRepository,
    // The DAO rather than BoardRepository: the badge needs one flow of
    // notes, not the board's whole sync machinery, and the narrower
    // dependency keeps this screen's tests free of board plumbing.
    private val noteDao: NoteDao,
    private val settingsRepository: SettingsRepository,
    settings: SettingsRepository,
    connectivity: ConnectivityObserver,
    socket: ChatSocket,
) : ViewModel() {

    // Seeded with null, not emptyList(): null means "Room hasn't answered
    // yet" so the screen can show skeleton rows instead of flashing the
    // "No chats yet" empty state on cold entry.
    val chats: StateFlow<List<ChatEntity>?> = chatRepository.observeChats()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * userId → profile-picture version, so a direct-chat row can name its
     * peer's picture. One flow for the whole list rather than a lookup
     * per row.
     */
    val avatarVersions: StateFlow<Map<Long, Long>> = familyRepository.observeMembers()
        .map { members -> members.associate { it.userId to it.avatarVersion } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

    /**
     * Picker candidates: everyone still in the family but me.
     *
     * ACTIVE members — you cannot start a chat with somebody who left, and
     * offering them would be offering a request the server refuses.
     * `avatarVersions` above deliberately reads the FULL roster instead:
     * it is drawing faces on existing rows, including a direct chat with
     * somebody who has since gone.
     */
    /**
     * Who the reader has blocked, and their own id — the chat list needs
     * both to decide whether a row's preview is somebody else's hidden
     * message.
     */
    val blockedUserIds: StateFlow<Set<Long>> = settings.state.map { it.blockedUserIds }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    val myUserId: StateFlow<Long?> = settings.state.map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    val pickableMembers: StateFlow<List<MemberEntity>> =
        combine(familyRepository.observeActiveMembers(), settings.state) { members, s ->
            // Blocked members are left OUT rather than left in to have the
            // tap answered `blocked` — the protocol's own steer for what a
            // client does with a complete roster (docs/protocol.md,
            // "Blocking a member"). The roster itself is untouched: they
            // are still nameable everywhere a name is needed.
            members.filter { it.userId != s.myUserId && it.userId !in s.blockedUserIds }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /**
     * Notes pinned since this device last showed the board.
     *
     * Counted from the note IDS, against a high-water mark that only moves
     * when the board is opened — never from `boardCursor`, which a
     * background resync advances and would silently clear the badge.
     */
    val newNoteCount: StateFlow<Int> =
        combine(noteDao.observeNotes(), settings.state) { notes, s ->
            notes.count { it.id > s.boardSeenNoteId }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), 0)

    /** The board is on screen, so everything pinned to it has been seen. */
    fun markBoardSeen() {
        viewModelScope.launch {
            val highest = noteDao.observeNotes().first().maxOfOrNull { it.id } ?: 0L
            settingsRepository.setBoardSeenNoteId(highest)
        }
    }

    val isOnline: StateFlow<Boolean> = connectivity.isOnline
    val socketState: StateFlow<SocketState> = socket.state

    private val _navigateToChat = MutableSharedFlow<Long>(extraBufferCapacity = 1)
    val navigateToChat: SharedFlow<Long> = _navigateToChat

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error

    fun refresh() {
        viewModelScope.launch {
            chatRepository.refreshChats()
        }
    }

    fun openDirectChat(userId: Long) {
        viewModelScope.launch {
            when (val result = chatRepository.createDirect(userId)) {
                is ApiResult.Ok -> _navigateToChat.tryEmit(result.value.id)
                is ApiResult.HttpError ->
                    _error.value = result.message ?: appContext.getString(R.string.e_open_chat_failed)
                is ApiResult.NetworkError ->
                    _error.value = appContext.getString(R.string.e_unreachable)
            }
        }
    }

    fun dismissError() {
        _error.value = null
    }
}
