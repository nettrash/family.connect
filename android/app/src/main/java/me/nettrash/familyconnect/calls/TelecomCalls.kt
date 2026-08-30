/*
 * TelecomCalls.kt
 * Family Connect (Android)
 *
 * Family's calls, registered with the Android Telecom framework through
 * the Jetpack core-telecom library — the platform's own way of knowing
 * that a call is in progress. Once it knows, everything that answers or
 * ends a call from OUTSIDE the app works: the button on a Bluetooth
 * headset, a Wear OS watch's call card, Android Auto; a cellular call
 * arriving mid-call is arbitrated instead of the two fighting over the
 * microphone; the audio route (earpiece, speaker, headset) is the
 * system's to choose and follows the headset the person actually put
 * on; and on Android 16.1+ the Phone app's call log lists the call and
 * can call back (CallBackRegistry). iOS has all of this through CallKit
 * (CallKitController); this is the Android twin.
 *
 * The shape, and why: CallManager stays the state machine and knows
 * nothing about Telecom. This mirrors its state INTO a Telecom session —
 * addCall when a call begins, answer / setActive / disconnect as the
 * state moves (TelecomMirror, the pure mapping) — and the session's
 * callbacks OUT of it: a remote surface answering or hanging up is
 * accept() / hangUp(); Telecom holding the call for a SIM call is
 * setHeld(); the route it chose is onSystemRoute(). core-telecom's
 * addCall does not return until the session ends, so every call is one
 * coroutine on the app scope that lives exactly as long as the call.
 *
 * Audio: under Telecom the app must not set the communication mode,
 * take audio focus or pick a device itself — that is the platform's job
 * now, and doing both is documented to break audio. AndroidCallAudio asks
 * this (CallRouteOwner) whether Telecom owns the CURRENT call's audio
 * and routes the speaker toggle through requestEndpointChange; the old
 * hand-rolled path stays as the fallback for a call Telecom did not take.
 * Ownership is decided per call and forgotten when the call ends, so one
 * refusal never colours the next call.
 *
 * Refusals: Telecom refuses an OUTGOING call while a cellular call is up
 * — correct, and now said out loud (the call ends "busy") rather than
 * failing with no sound. But the library's "not permitted" code also
 * means "no phone account" or "another setup in flight", so busy is only
 * declared when the device really is in a call; every other refusal
 * keeps the call on the fallback audio path. An INCOMING call is not
 * offered to Telecom at all while the microphone permission is missing:
 * a watch or a headset would answer it, Telecom would count it active,
 * and the phone would still be waiting to ask.
 */

package me.nettrash.familyconnect.calls

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioManager
import android.net.Uri
import android.telecom.DisconnectCause
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.core.telecom.CallAttributesCompat
import androidx.core.telecom.CallControlResult
import androidx.core.telecom.CallControlScope
import androidx.core.telecom.CallEndpointCompat
import androidx.core.telecom.CallException
import androidx.core.telecom.CallsManager
import dagger.hilt.android.qualifiers.ApplicationContext
import java.util.Collections
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.resolvedDisplayNames
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class TelecomCalls @Inject constructor(
    @param:ApplicationContext private val context: Context,
    private val callManager: CallManager,
    private val audio: AndroidCallAudio,
    private val memberDao: MemberDao,
    private val settings: SettingsRepository,
    private val callBacks: CallBackRegistry,
    @param:AppScope private val scope: CoroutineScope,
) : CallRouteOwner {

    init {
        // The system dialer's call log outlives the block: it keeps
        // offering to ring somebody through Family, and the app would then
        // place a call the server refuses. Watched rather than hooked to
        // one action, so a block made on ANOTHER device is honoured here
        // too — the same reason ChatRepository watches this set to prune
        // the direct chat (docs/protocol.md, "Calls").
        scope.launch {
            settings.state
                .map { it.blockedUserIds }
                .distinctUntilChanged()
                .collect { blocked -> blocked.forEach(callBacks::forget) }
        }
    }

    private val manager: CallsManager by lazy { CallsManager(context) }

    /** registerAppWithTelecom succeeded: calls are Telecom's from here on. */
    @Volatile
    private var registered = false

    /** Telecom did not take the CURRENT call; its audio is the fallback path's. Reset when the call ends. */
    @Volatile
    private var fellBack = false

    private class Session(val callId: String, val control: CallControlScope, val mirror: TelecomMirror) {
        @Volatile
        var endpoints: List<CallEndpointCompat> = emptyList()

        /** What the route is now, so a failed request can put the toggle back. */
        @Volatile
        var speakerNow = false

        /** Everything launched on the scope — all cancelled after disconnect, or addCall never returns. */
        val jobs: MutableList<Job> = Collections.synchronizedList(mutableListOf())

        /** Disconnected: nothing may be launched on the scope any more. */
        @Volatile
        var over = false
    }

    @Volatile
    private var session: Session? = null

    /** A route asked for before the session had endpoints. */
    @Volatile
    private var pendingSpeaker: Boolean? = null

    override val ownsAudio: Boolean
        get() = registered && !fellBack

    /**
     * Register once, at app start — off the main thread, it is two binder
     * calls — and follow the manager from then on. A device that will not
     * register (no Telecom at all) simply keeps the hand-rolled audio
     * path, which is what every call used before.
     */
    fun start() {
        scope.launch {
            try {
                manager.registerAppWithTelecom(
                    CallsManager.CAPABILITY_BASELINE or CallsManager.CAPABILITY_SUPPORTS_VIDEO_CALLING,
                )
            } catch (error: Exception) {
                Log.w(TAG, "Telecom registration failed; calls stay app-managed: $error")
                return@launch
            }
            registered = true
            audio.routeOwner = this@TelecomCalls
            var tracked: String? = null
            callManager.state.collect { state ->
                val live = state as? CallState.Live
                if (live != null && live.callId != tracked) {
                    tracked = live.callId
                    open(live)
                }
                if (state !is CallState.Live) {
                    // Ownership is per call: whatever happened to this one
                    // says nothing about the next.
                    fellBack = false
                    pendingSpeaker = null
                    if (state is CallState.Idle) tracked = null
                }
            }
        }
        // The call-back index names members of ONE account's family: a
        // logout or a different login empties it. Changes only — the first
        // value is merely who is signed in now.
        scope.launch {
            settings.state.map { it.myUserId }.distinctUntilChanged().drop(1).collect { callBacks.clear() }
        }
    }

    // -- CallRouteOwner -------------------------------------------------------------

    override fun requestSpeaker(on: Boolean): Boolean {
        if (!ownsAudio) return false
        val current = session
        if (current == null || !route(current, on)) {
            // No session, or no endpoints yet: applied when they arrive.
            pendingSpeaker = on
        }
        return true
    }

    /**
     * True when an endpoint of the wanted kind exists and was asked for.
     * Off the speaker means the earpiece (or the wired headset that is
     * plugged in) — never a Bluetooth device by preference: a paired
     * watch is one of those, and the platform's own default is the right
     * judge of headsets.
     */
    private fun route(current: Session, speaker: Boolean): Boolean {
        val endpoints = current.endpoints
        val target = if (speaker) {
            endpoints.firstOrNull { it.type == CallEndpointCompat.TYPE_SPEAKER }
        } else {
            endpoints.firstOrNull { it.type == CallEndpointCompat.TYPE_WIRED_HEADSET }
                ?: endpoints.firstOrNull { it.type == CallEndpointCompat.TYPE_EARPIECE }
        } ?: return false
        if (current.over) return false
        current.jobs += current.control.launch {
            val result = withTimeoutOrNull(ROUTE_TIMEOUT_MILLIS) { current.control.requestEndpointChange(target) }
            if (result !is CallControlResult.Success) {
                Log.w(TAG, "route → ${target.name} refused: $result")
                // The route did not change; the toggle must not claim it did.
                callManager.onSystemRoute(speaker = current.speakerNow)
            }
        }
        return true
    }

    // -- The session ------------------------------------------------------------------

    private fun open(live: CallState.Live) {
        val incoming = live is CallState.Incoming || (live is CallState.Connecting && live.incoming)
        if (incoming && !granted(Manifest.permission.RECORD_AUDIO)) {
            Log.i(TAG, "Incoming call kept off Telecom: no microphone permission yet")
            fellBack = true
            return
        }
        scope.launch {
            val name = memberDao.observeMembers().first().resolvedDisplayNames(context)[live.peerUserId]
                ?: context.getString(CallNotifications.kindTextRes(live.video))
            // A video call starts on the speaker, and the library has a
            // word for that — better than switching after the fact.
            val startOn = if (live.video) {
                runCatching {
                    withTimeoutOrNull(ENDPOINTS_TIMEOUT_MILLIS) { manager.getAvailableStartingCallEndpoints().first() }
                }.getOrNull()?.firstOrNull { it.type == CallEndpointCompat.TYPE_SPEAKER }
            } else {
                null
            }
            // Unless the person already touched the toggle while the
            // lookup was out — their choice stands.
            if (live.video && startOn == null && pendingSpeaker == null) pendingSpeaker = true
            val attributes = CallAttributesCompat(
                displayName = name,
                // The member, in the app's own namespace — what the call
                // log shows as the "number" when it cannot show the name.
                address = Uri.parse("$ADDRESS_SCHEME:${live.peerUserId}"),
                direction = if (incoming) CallAttributesCompat.DIRECTION_INCOMING else CallAttributesCompat.DIRECTION_OUTGOING,
                callType = if (live.video) CallAttributesCompat.CALL_TYPE_VIDEO_CALL else CallAttributesCompat.CALL_TYPE_AUDIO_CALL,
                callCapabilities = CallAttributesCompat.SUPPORTS_SET_INACTIVE,
                preferredStartingCallEndpoint = startOn,
            )
            val mirror = TelecomMirror(live.callId, incoming)
            var started = false
            try {
                manager.addCall(
                    attributes,
                    onAnswer = { callType -> answerFromSystem(live.callId, callType, mirror) },
                    onDisconnect = { endFromSystem(live.callId) },
                    onSetActive = { callManager.setHeld(false) },
                    onSetInactive = { callManager.setHeld(true) },
                ) {
                    started = true
                    runSession(live.callId, this, mirror)
                }
            } catch (error: Exception) {
                if (started) {
                    // The session died AFTER it was up (a callback threw
                    // past Telecom's timeout): Telecom has let go of the
                    // call, but the audio it configured is what the call
                    // is running on — leave it be.
                    Log.w(TAG, "Telecom session for ${live.callId} ended with $error")
                } else {
                    refused(live, error)
                }
            } finally {
                if (session?.callId == live.callId) session = null
            }
        }
    }

    /** Inside the CallControlScope: mirror the manager until the call ends. */
    private fun runSession(callId: String, control: CallControlScope, mirror: TelecomMirror) {
        val current = Session(callId, control, mirror)
        session = current
        (callManager.state.value as? CallState.Live)?.let { live ->
            if (live.callId == callId) {
                callBacks.remember(
                    CallBackEntry(control.getCallId().uuid.toString(), live.chatId, live.peerUserId, live.video),
                )
            }
        }
        current.jobs += control.launch {
            control.currentCallEndpoint.collect { endpoint ->
                val speaker = endpoint.type == CallEndpointCompat.TYPE_SPEAKER
                current.speakerNow = speaker
                callManager.onSystemRoute(speaker = speaker)
            }
        }
        current.jobs += control.launch {
            control.availableEndpoints.collect { endpoints ->
                current.endpoints = endpoints
                pendingSpeaker?.let { on -> if (route(current, on)) pendingSpeaker = null }
            }
        }
        current.jobs += control.launch {
            // Every value is a change the person made somewhere else (a
            // headset's button): the flow is not seeded with a current
            // state, so nothing is dropped.
            control.isMuted.distinctUntilChanged().collect { muted -> callManager.setMuted(muted) }
        }
        current.jobs += control.launch {
            callManager.state.collect { next ->
                for (command in mirror.next(next)) {
                    when (command) {
                        is TelecomTransitions.Command.Answer -> control.answer(
                            if (command.video) CallAttributesCompat.CALL_TYPE_VIDEO_CALL else CallAttributesCompat.CALL_TYPE_AUDIO_CALL,
                        )
                        TelecomTransitions.Command.SetActive -> control.setActive()
                        is TelecomTransitions.Command.Disconnect -> {
                            current.over = true
                            control.disconnect(DisconnectCause(code(command.cause)))
                            // The block's coroutines are what keep addCall
                            // from returning; the session is over.
                            synchronized(current.jobs) { current.jobs.toList() }.forEach { it.cancel() }
                        }
                    }
                }
            }
        }
    }

    /**
     * Telecom could not take the call. An outgoing one while the device is
     * genuinely in a (cellular) call is refused by design — it ends here,
     * as "busy", instead of ringing the far side with no audio route.
     * Every other refusal — no phone account, a setup still in flight, a
     * timeout — keeps the call, on the fallback audio path.
     */
    private fun refused(live: CallState.Live, error: Exception) {
        Log.w(TAG, "Telecom did not take call ${live.callId}: $error")
        val still = callManager.state.value as? CallState.Live ?: return
        if (still.callId != live.callId) return
        fellBack = true
        pendingSpeaker = null
        val notPermitted = error is CallException && error.code == CallException.ERROR_CALL_NOT_PERMITTED_AT_PRESENT_TIME
        val inCall = context.getSystemService(AudioManager::class.java)?.mode == AudioManager.MODE_IN_CALL
        if (notPermitted && inCall && still is CallState.Outgoing) {
            callManager.systemRefused(live.callId)
            return
        }
        audio.takeOverAudio(live.video)
    }

    // -- Telecom's requests ------------------------------------------------------------

    /**
     * A watch, a headset, Android Auto answered. Telecom counts the call
     * active the moment this returns, so it waits — within the 5-second
     * budget — for the manager to actually leave Incoming: the answer is
     * immediate when the offer is here, and a beat later when the socket
     * is still replaying it. The microphone permission is already held
     * (a call without it is never offered to Telecom, see open()).
     */
    private suspend fun answerFromSystem(callId: String, callType: Int, mirror: TelecomMirror) {
        val incoming = callManager.state.value as? CallState.Incoming ?: return
        if (incoming.callId != callId) return
        val cameraGranted = when {
            !incoming.video -> null
            callType != CallAttributesCompat.CALL_TYPE_VIDEO_CALL -> false
            else -> granted(Manifest.permission.CAMERA)
        }
        mirror.systemAnswered()
        callManager.accept(cameraGranted)
        withTimeoutOrNull(ANSWER_WAIT_MILLIS) {
            callManager.state.first { it !is CallState.Incoming || it.callId != callId }
        } ?: throw IllegalStateException("the call could not be answered in time")
        // Throwing is how Telecom is told the answer failed: it tears the
        // session down rather than showing an active call with no audio.
    }

    private fun endFromSystem(callId: String) {
        val live = callManager.state.value as? CallState.Live ?: return
        if (live.callId != callId) return
        if (live is CallState.Incoming) callManager.decline() else callManager.hangUp()
    }

    private fun granted(permission: String): Boolean =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

    private fun code(cause: TelecomTransitions.Cause): Int = when (cause) {
        TelecomTransitions.Cause.LOCAL -> DisconnectCause.LOCAL
        TelecomTransitions.Cause.REMOTE -> DisconnectCause.REMOTE
        TelecomTransitions.Cause.REJECTED -> DisconnectCause.REJECTED
        TelecomTransitions.Cause.MISSED -> DisconnectCause.MISSED
        TelecomTransitions.Cause.BUSY -> DisconnectCause.BUSY
        TelecomTransitions.Cause.ERROR -> DisconnectCause.ERROR
    }

    companion object {
        private const val TAG = "TelecomCalls"

        /** The app's URL scheme, doubling as the call address namespace — as on iOS (CallHandle). */
        const val ADDRESS_SCHEME = "familyconnect"

        /** The Phone app's call-back intent (TelecomManager.ACTION_CALL_BACK, Android 16.1+). */
        const val ACTION_CALL_BACK = "android.telecom.action.CALL_BACK"

        /** Its extra: the call's Telecom UUID (TelecomManager.EXTRA_UUID). */
        const val EXTRA_UUID = "android.telecom.extra.UUID"

        /** Under Telecom's own 5-second transaction budget. */
        private const val ANSWER_WAIT_MILLIS = 4_000L
        private const val ROUTE_TIMEOUT_MILLIS = 3_000L
        private const val ENDPOINTS_TIMEOUT_MILLIS = 1_000L
    }
}
