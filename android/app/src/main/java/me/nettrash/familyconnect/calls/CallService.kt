/*
 * CallService.kt
 * Family Connect (Android)
 *
 * The foreground service a call lives behind. It exists for three
 * platform reasons and one product one: a process woken by the call push
 * has to be allowed to keep running while the socket connects and the
 * phone rings; the microphone may only be used from the foreground; a
 * notification with CallStyle has to belong to an ongoing thing; and a
 * person who switches apps mid-call must not have the call killed under
 * them.
 *
 * It owns no call state — CallManager does — and merely mirrors it: the
 * ringing notification while Incoming, the ongoing one from Connecting on,
 * and stopSelf the moment the manager is Idle again. Declined and Hang up
 * from the notification land here as intent actions.
 */

package me.nettrash.familyconnect.calls

import android.Manifest
import android.app.Service
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.util.resolvedDisplayNames
import javax.inject.Inject

@AndroidEntryPoint
class CallService : Service() {

    @Inject
    lateinit var callManager: CallManager

    @Inject
    lateinit var memberDao: MemberDao

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var foregroundId = 0

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        CallNotifications.ensureChannels(this)
        scope.launch {
            callManager.state.collect { state -> render(state) }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_DECLINE -> callManager.decline()
            ACTION_HANG_UP -> callManager.hangUp()
        }
        // startForegroundService demands a startForeground within seconds,
        // whatever the state; the collector above also runs on the first
        // emission, and a second call with the same notification is a
        // harmless update.
        scope.launch { render(callManager.state.value) }
        return START_NOT_STICKY
    }

    private suspend fun render(state: CallState) {
        when (state) {
            is CallState.Idle -> {
                ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
                CallNotifications.cancelAll(this)
                stopSelf()
            }
            is CallState.Incoming -> {
                val name = state.callerName ?: nameOf(state.peerUserId)
                promote(
                    CallNotifications.INCOMING_ID,
                    CallNotifications.incoming(this, name, state.peerUserId, state.video),
                )
            }
            is CallState.Live -> {
                val status = when (state) {
                    is CallState.Active -> getString(CallNotifications.ongoingTextRes(state.video))
                    else -> getString(R.string.s_connecting)
                }
                promote(
                    CallNotifications.ONGOING_ID,
                    CallNotifications.ongoing(this, nameOf(state.peerUserId), state.peerUserId, status),
                )
            }
            is CallState.Ended -> {
                // Keep whatever is showing until the manager goes Idle; a
                // flash of a third notification for two seconds is noise.
                if (foregroundId == 0) {
                    promote(
                        CallNotifications.ONGOING_ID,
                        CallNotifications.ongoing(
                            this,
                            state.peerUserId?.let { nameOf(it) } ?: getString(R.string.s_voice_call),
                            state.peerUserId ?: 0L,
                            getString(R.string.s_call_ended),
                        ),
                    )
                }
            }
        }
    }

    private fun promote(id: Int, notification: android.app.Notification) {
        if (foregroundId != 0 && foregroundId != id) {
            NotificationManagerCompat.from(this).cancel(foregroundId)
        }
        foregroundId = id
        ServiceCompat.startForeground(this, id, notification, foregroundType())
    }

    /**
     * phoneCall always; microphone and camera only once their runtime
     * permissions are held — declaring a type without its permission is a
     * SecurityException on 14+. The camera bit rides on the grant alone,
     * exactly like the microphone bit: harmless on a voice call, and the
     * one thing that lets a video call keep capturing in the background.
     */
    private fun foregroundType(): Int {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return 0
        var type = ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL
        val micHeld = ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        if (micHeld && Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            type = type or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        }
        val cameraHeld = ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        if (cameraHeld && Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            type = type or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        }
        return type
    }

    private suspend fun nameOf(userId: Long): String =
        memberDao.observeMembers().first().resolvedDisplayNames(this)[userId]
            ?: getString(R.string.s_voice_call)

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        const val ACTION_DECLINE = "me.nettrash.familyconnect.call.DECLINE"
        const val ACTION_HANG_UP = "me.nettrash.familyconnect.call.HANG_UP"

        fun intent(context: android.content.Context): Intent = Intent(context, CallService::class.java)
    }
}
