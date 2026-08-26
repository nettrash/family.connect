/*
 * CallViewModel.kt
 * Family Connect (Android)
 *
 * The call screen's view of CallManager: the state, who the other person
 * is (name and picture from the roster), the two toggles, and the four
 * actions. Thin on purpose — the machine is the manager's, and this only
 * resolves the peer for drawing.
 */

package me.nettrash.familyconnect.ui.call

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import me.nettrash.familyconnect.calls.CallManager
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.util.resolvedDisplayNames
import javax.inject.Inject

/** The other person, as the screen draws them. */
data class CallPeer(val userId: Long, val name: String, val avatarVersion: Long)

@HiltViewModel
class CallViewModel @Inject constructor(
    @param:ApplicationContext private val appContext: Context,
    private val callManager: CallManager,
    memberDao: MemberDao,
) : ViewModel() {

    val state: StateFlow<CallState> = callManager.state
    val isMuted: StateFlow<Boolean> = callManager.isMuted
    val isSpeaker: StateFlow<Boolean> = callManager.isSpeaker
    val answerRequested: StateFlow<Boolean> = callManager.answerRequested

    val peer: StateFlow<CallPeer?> = combine(callManager.state, memberDao.observeMembers()) { state, members ->
        val userId = when (state) {
            is CallState.Live -> state.peerUserId
            is CallState.Ended -> state.peerUserId
            CallState.Idle -> null
        } ?: return@combine null
        val names = members.resolvedDisplayNames(appContext)
        CallPeer(
            userId = userId,
            // The push's caller name bridges the gap until the roster is
            // loaded — a phone woken from dead has nothing else yet.
            name = names[userId] ?: (state as? CallState.Incoming)?.callerName ?: "",
            avatarVersion = members.firstOrNull { it.userId == userId }?.avatarVersion ?: 0L,
        )
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    val isIncomingRinging: StateFlow<Boolean> = callManager.state.map { it is CallState.Incoming }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

    fun accept() = callManager.accept()
    fun decline() = callManager.decline()
    fun hangUp() = callManager.hangUp()
    fun toggleMute() = callManager.toggleMute()
    fun toggleSpeaker() = callManager.toggleSpeaker()
}
