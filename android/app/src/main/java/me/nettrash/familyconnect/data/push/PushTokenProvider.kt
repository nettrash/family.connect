/*
 * PushTokenProvider.kt
 * Family Connect (Android)
 *
 * Seam for "what is this device's current FCM registration token?".
 * The Firebase-backed impl is the ONLY place the app actively calls into
 * Firebase, and it is built to be safely inert: source builds carry no
 * google-services.json, so FirebaseApp.initializeApp finds no config and
 * FirebaseMessaging.getInstance() throws IllegalStateException — the
 * runCatching turns that into "no token", and push simply stays off.
 *
 * The Task callback is bridged with suspendCancellableCoroutine instead
 * of pulling in kotlinx-coroutines-play-services for `.await()`: one
 * extension function is not worth another artifact in a deliberately
 * minimal dependency graph (see gradle/libs.versions.toml header).
 *
 * iOS counterpart: none yet (push is not ported to ios/ at this time).
 */

package me.nettrash.familyconnect.data.push

import com.google.firebase.messaging.FirebaseMessaging
import kotlinx.coroutines.suspendCancellableCoroutine
import javax.inject.Inject
import kotlin.coroutines.resume

fun interface PushTokenProvider {
    /** The current FCM token, or null when Firebase isn't configured /
     *  the token isn't available (no Play services, transient failure). */
    suspend fun currentToken(): String?
}

class FirebasePushTokenProvider @Inject constructor() : PushTokenProvider {

    // getInstance()/getToken() are deprecated in firebase-messaging 25.x
    // in favor of the new register()/onRegistered() installation-id flow —
    // deliberately NOT adopted: the server's FCM HTTP v1 sender
    // (docs/protocol.md) targets the classic registration token, which is
    // exactly what getToken() returns, and the classic API stays enabled
    // unless an app opts in via the firebase_messaging_installation_id_enabled
    // meta-data (we don't). Revisit together with the server.
    @Suppress("DEPRECATION")
    override suspend fun currentToken(): String? = runCatching {
        suspendCancellableCoroutine { continuation ->
            // getInstance() itself throws when no FirebaseApp exists —
            // caught by the runCatching before we ever suspend.
            FirebaseMessaging.getInstance().token
                .addOnSuccessListener { token -> continuation.resume(token) }
                .addOnFailureListener { continuation.resume(null) }
                .addOnCanceledListener { continuation.resume(null) }
        }
    }.getOrNull()
}
