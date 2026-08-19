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

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
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
    private val chatRepository: ChatRepository,
    familyRepository: FamilyRepository,
    settings: SettingsRepository,
    connectivity: ConnectivityObserver,
    socket: ChatSocket,
) : ViewModel() {

    val chats: StateFlow<List<ChatEntity>> = chatRepository.observeChats()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

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
                    _error.value = result.message ?: "Couldn't open the chat"
                is ApiResult.NetworkError ->
                    _error.value = "Can't reach the server"
            }
        }
    }

    fun dismissError() {
        _error.value = null
    }
}
