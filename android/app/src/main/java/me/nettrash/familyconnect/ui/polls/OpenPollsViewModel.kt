/*
 * OpenPollsViewModel.kt
 * Family Connect (Android)
 *
 * The open-polls screen's state (docs/protocol.md, "Finding the open ones").
 *
 * Unlike the board's, this one does NOT read Room: `GET /chats/{id}/polls/open`
 * is a plain read that answers "what is still open right now", and the point
 * of the screen is to show polls the thread may never have paged back to.
 * Reading the local cache would show only what this device happens to hold,
 * which is exactly the problem the screen exists to solve.
 *
 * It is not a cursor and moves nothing: the poll catch-up feed
 * (`getPolls(afterSeq)`) remains what the sync loop runs, and this asks its
 * question when the screen opens and again after a vote.
 *
 * iOS counterpart: Views/OpenPollsView.swift
 */

package me.nettrash.familyconnect.ui.polls

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.dto.MessageDto
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.util.resolvedDisplayNames
import javax.inject.Inject

@HiltViewModel
class OpenPollsViewModel @Inject constructor(
    /** For `getString` only — see the note on SettingsViewModel. */
    @param:ApplicationContext private val appContext: Context,
    private val chatApi: ChatApi,
    memberDao: MemberDao,
    private val settings: SettingsRepository,
) : ViewModel() {

    data class State(
        /** The open polls, oldest first, each as the message that carries it. */
        val messages: List<MessageDto> = emptyList(),
        val loading: Boolean = true,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    val myUserId: StateFlow<Long?> = settings.state
        .map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    val blockedUserIds: StateFlow<Set<Long>> = settings.state
        .map { it.blockedUserIds }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    /** userId -> profile-picture version, so the faces are the chat's faces. */
    val memberAvatars: StateFlow<Map<Long, Long>> = memberDao.observeMembers()
        .map { members -> members.associate { it.userId to it.avatarVersion } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

    /** userId -> display name, for the byline and the voter faces. */
    val names: StateFlow<Map<Long, String>> = memberDao.observeMembers()
        .map { it.resolvedDisplayNames(appContext) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

    /**
     * How many people a poll could hear from — the live roster, neither the
     * people who have left nor the accounts that were deleted: a tally must
     * not go on counting somebody who no longer exists.
     */
    val memberCount: StateFlow<Int> = memberDao.observeActiveMembers()
        .map { it.size }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), 0)

    private var chatId: Long = 0

    fun start(chatId: Long) {
        this.chatId = chatId
        load()
    }

    fun load(keepError: Boolean = false) {
        viewModelScope.launch {
            if (!keepError) _state.update { it.copy(error = null) }
            when (val result = chatApi.getOpenPolls(chatId)) {
                is ApiResult.Ok ->
                    _state.update {
                        it.copy(
                            messages = result.value.messages,
                            loading = false,
                            error = if (keepError) it.error else null,
                        )
                    }
                is ApiResult.HttpError ->
                    _state.update {
                        it.copy(
                            loading = false,
                            error = result.message ?: appContext.getString(R.string.e_try_again),
                        )
                    }
                is ApiResult.NetworkError ->
                    _state.update {
                        it.copy(loading = false, error = appContext.getString(R.string.e_unreachable))
                    }
            }
        }
    }

    /**
     * Vote, then re-read.
     *
     * The vote endpoint answers with the poll's whole new state and the socket
     * fans the same state to every other device, so this could patch one row
     * in place — but a poll may have been CLOSED by its author while this list
     * was open, and a closed poll belongs off this list. Re-reading is one
     * request and gets both right.
     */
    fun vote(messageId: Long, optionId: Long) {
        // The tap on the option you already hold is a RETRACT, here as in the
        // chat (MessageRepository.toggleVote decides the same way). Always
        // PUTting was a silent no-op on this one surface: the server treats a
        // re-PUT of the held option as nothing, so nothing moved.
        val held = _state.value.messages
            .firstOrNull { it.id == messageId }
            ?.poll
            ?.optionHeldBy(myUserId.value ?: -1L)
            ?.id
        if (held == optionId) {
            retract(messageId)
            return
        }
        viewModelScope.launch { settle(chatApi.putVote(chatId, messageId, optionId)) }
    }

    fun retract(messageId: Long) {
        viewModelScope.launch { settle(chatApi.deleteVote(chatId, messageId)) }
    }

    /** Closing removes it from this list, which is the point of the list. */
    fun close(messageId: Long) {
        viewModelScope.launch { settle(chatApi.closePoll(chatId, messageId)) }
    }

    /**
     * After any change: re-read, whatever the answer was.
     *
     * On success the re-read is what drops a poll the author closed meanwhile.
     * On FAILURE it matters more — a poll closed under the reader answers
     * `poll_closed` to every tap, and without a re-read it would sit on the
     * list refusing them for ever, with the failure said nowhere. So the
     * reason is recorded AND the list is refreshed, and the screen draws the
     * reason above a non-empty list rather than only in the empty state.
     */
    private suspend fun settle(result: ApiResult<*>) {
        if (result !is ApiResult.Ok) {
            _state.update { it.copy(error = appContext.getString(R.string.e_try_again)) }
        }
        load(keepError = result !is ApiResult.Ok)
    }
}
