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
import me.nettrash.familyconnect.calls.WebRtcClientFactory
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.util.resolvedDisplayNames
import org.webrtc.EglBase
import org.webrtc.VideoSink
import javax.inject.Inject

/** The other person, as the screen draws them. */
data class CallPeer(val userId: Long, val name: String, val avatarVersion: Long)

@HiltViewModel
class CallViewModel @Inject constructor(
    @param:ApplicationContext private val appContext: Context,
    private val callManager: CallManager,
    memberDao: MemberDao,
    /**
     * The concrete factory, for the ONE EglBase context every
     * SurfaceViewRenderer must be initialized with — the sole reason a
     * screen file knows the WebRTC factory by name.
     */
    private val webRtcFactory: WebRtcClientFactory,
) : ViewModel() {

    val state: StateFlow<CallState> = callManager.state
    val isMuted: StateFlow<Boolean> = callManager.isMuted
    val isSpeaker: StateFlow<Boolean> = callManager.isSpeaker
    val isCameraOn: StateFlow<Boolean> = callManager.isCameraOn
    val isFrontCamera: StateFlow<Boolean> = callManager.isFrontCamera
    val remoteVideoActive: StateFlow<Boolean> = callManager.remoteVideoActive
    val answerRequested: StateFlow<Boolean> = callManager.answerRequested

    /** What the screen's renderers are initialized with. */
    val eglBaseContext: EglBase.Context get() = webRtcFactory.eglBaseContext

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

    /**
     * [cameraGranted] is the answer dialog's camera verdict on a video
     * call: granted starts the camera ON (parity with iOS), denied
     * answers camera-off; null (grant already held, or a voice call)
     * leaves the camera state as it is.
     */
    fun accept(cameraGranted: Boolean? = null) = callManager.accept(cameraGranted)
    fun decline() = callManager.decline()
    fun hangUp() = callManager.hangUp()
    fun toggleMute() = callManager.toggleMute()
    fun toggleSpeaker() = callManager.toggleSpeaker()
    fun toggleCamera() = callManager.toggleCamera()
    fun flipCamera() = callManager.flipCamera()

    /** The camera-permission-denied path: the call goes on, camera off (protocol.md, "Video"). */
    fun setCameraEnabled(on: Boolean) = callManager.setCameraEnabled(on)

    fun setLocalVideoSink(sink: VideoSink?) = callManager.setLocalVideoSink(sink)
    fun setRemoteVideoSink(sink: VideoSink?) = callManager.setRemoteVideoSink(sink)

    /**
     * The screen's surfaces go away by NAME, not by null: two
     * compositions can overlap (an Activity recreated mid-call builds the
     * new tree before it disposes the old one), and a blind detach from
     * the dying one would unhook the live surface for the rest of the
     * call. The manager ignores a release that is not the current sink.
     */
    fun detachLocalVideoSink(sink: VideoSink) = callManager.detachLocalVideoSink(sink)
    fun detachRemoteVideoSink(sink: VideoSink) = callManager.detachRemoteVideoSink(sink)
}
