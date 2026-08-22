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

    /** Picker candidates: everyone but me. */
    val pickableMembers: StateFlow<List<MemberEntity>> =
        combine(familyRepository.observeMembers(), settings.state) { members, s ->
            members.filter { it.userId != s.myUserId }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

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
