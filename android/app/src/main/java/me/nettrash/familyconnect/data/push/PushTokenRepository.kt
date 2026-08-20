/*
 * PushTokenRepository.kt
 * Family Connect (Android)
 *
 * Owns the device-registration half of the protocol's push lifecycle
 * (docs/protocol.md, "Push notifications"):
 *
 *   POST /devices {platform: "android", push_token} → {device_id}
 *
 * Rules encoded here:
 *   - register only while a session exists (token + server URL) — when
 *     logged out the FCM token is just cached so the next login can
 *     register immediately;
 *   - re-POST only on change: same token + a stored device_id = no-op
 *     (the server upserts by token, but there's no point chattering);
 *   - a failed POST caches the token WITHOUT a device_id, so the next
 *     login/resync retries instead of short-circuiting;
 *   - DELETE /devices/{id} on logout lives in SessionRepository (it owns
 *     teardown ordering); the id is dropped by resetKeepingServerUrl,
 *     the token survives it (device-scoped, not account-scoped).
 *
 * Deliberately depends on TokenStore + SettingsRepository, NOT on
 * SessionRepository — SessionRepository calls *into* this class on login,
 * so depending back on it would be a cycle.
 *
 * iOS counterpart: none yet (push is not ported to ios/ at this time).
 */

package me.nettrash.familyconnect.data.push

import kotlinx.coroutines.flow.first
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AuthApi
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.TokenStore
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class PushTokenRepository @Inject constructor(
    private val authApi: AuthApi,
    private val settings: SettingsRepository,
    private val tokenStore: TokenStore,
    private val pushTokenProvider: PushTokenProvider,
) {

    /** FCM delivered a fresh (possibly rotated) token — FcPushService.onNewToken. */
    suspend fun onNewToken(token: String) {
        register(token)
    }

    /**
     * Login / resync hook: ask Firebase for the current token (null when
     * the build has no google-services.json), fall back to the cached one
     * (an onNewToken that arrived while logged out), and (re)register.
     * A null token still registers the device row — the protocol's v1
     * push hook — it just can't be pushed to.
     */
    suspend fun registerCurrentToken() {
        register(pushTokenProvider.currentToken() ?: settings.state.first().pushToken)
    }

    private suspend fun register(token: String?) {
        val state = settings.state.first()
        val loggedIn = tokenStore.load() != null && state.serverUrl != null
        if (!loggedIn) {
            // No session to attach a device to — cache the token; the
            // next login's registerCurrentToken() picks it up.
            token?.let { settings.setPushToken(it) }
            return
        }
        // Re-POST only on change: an existing registration for this exact
        // token needs no refresh. A missing device_id means the last POST
        // failed (or never happened) — always retry then.
        if (state.pushDeviceId != null && state.pushToken == token) return
        when (val result = authApi.registerDevice(token)) {
            is ApiResult.Ok -> {
                settings.setPushToken(token)
                settings.setPushDeviceId(result.value.deviceId)
            }
            is ApiResult.HttpError, is ApiResult.NetworkError -> {
                // Best-effort by design: cache the token, leave device_id
                // unset so the next login/resync retries the POST.
                token?.let { settings.setPushToken(it) }
            }
        }
    }
}
