/*
 * CallServiceLauncher.kt
 * Family Connect (Android)
 *
 * Starts CallService whenever CallManager leaves Idle. The push path
 * starts the service itself, synchronously, inside the FCM handler
 * (that is the window in which a background start is allowed); this
 * covers the other way a call begins — the person tapped Call, or an
 * offer arrived over a socket that was already open — both of which
 * happen with the app on screen, where starting a foreground service is
 * always permitted. If it is not (an offer on a socket held open past
 * onStop), the ringing notification is posted directly instead, so the
 * call still rings.
 */

package me.nettrash.familyconnect.calls

import android.content.Context
import androidx.core.content.ContextCompat
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.resolvedDisplayNames
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class CallServiceLauncher @Inject constructor(
    @param:ApplicationContext private val context: Context,
    private val callManager: CallManager,
    private val memberDao: MemberDao,
    @param:AppScope private val scope: CoroutineScope,
) {
    fun start() {
        scope.launch {
            var wasIdle = true
            callManager.state.collect { state ->
                val idle = state is CallState.Idle
                if (wasIdle && !idle) launchService(state)
                wasIdle = idle
            }
        }
    }

    private suspend fun launchService(state: CallState) {
        try {
            ContextCompat.startForegroundService(context, CallService.intent(context))
        } catch (error: Exception) {
            // Android 12+: not allowed from the background. Ring anyway.
            val incoming = state as? CallState.Incoming ?: return
            val name = incoming.callerName
                ?: memberDao.observeMembers().first().resolvedDisplayNames(context)[incoming.peerUserId]
                ?: ""
            CallNotifications.showIncoming(context, name, incoming.peerUserId, incoming.video)
        }
    }
}
