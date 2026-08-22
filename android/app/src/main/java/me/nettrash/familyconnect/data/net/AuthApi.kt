/*
 * AuthApi.kt
 * Family Connect (Android)
 *
 * Suspend wrappers over the Auth + Devices endpoint tables of
 * docs/protocol.md. Interface + impl split so ViewModel/repository tests
 * can substitute a scripted fake without an HTTP stack.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/AuthApi.swift
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.AuthResponse
import me.nettrash.familyconnect.data.net.dto.ChangePasswordRequest
import me.nettrash.familyconnect.data.net.dto.DeviceRequest
import me.nettrash.familyconnect.data.net.dto.DeviceResponse
import me.nettrash.familyconnect.data.net.dto.LoginRequest
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.RegisterRequest
import me.nettrash.familyconnect.data.settings.ServerUrlNormalizer
import javax.inject.Inject
import javax.inject.Singleton

interface AuthApi {
    suspend fun register(username: String, displayName: String, password: String): ApiResult<AuthResponse>
    suspend fun login(username: String, password: String): ApiResult<AuthResponse>
    suspend fun logout(): ApiResult<Unit>

    /**
     * Change my own password. The current one is required — a live session
     * is not proof of knowing it (protocol.md, "Auth"). Succeeding revokes
     * my OTHER sessions server-side; this one survives.
     */
    suspend fun changePassword(current: String, new: String): ApiResult<Unit>
    suspend fun me(): ApiResult<MeResponse>

    /**
     * Server-setup probe: unauthenticated GET /me against a *candidate*
     * URL (not yet saved). A live Family Connect server answers 401 with
     * the protocol error body — that 401 is the success signal.
     */
    suspend fun probe(candidateServerUrl: String): ApiResult<MeResponse>

    /**
     * POST /devices {platform: "android", push_token} → {device_id}.
     * Upserts by token when non-null; a null token still creates the
     * device row (the push hook without delivery — e.g. builds without
     * a google-services.json). PushTokenRepository owns when to call this.
     */
    suspend fun registerDevice(pushToken: String?): ApiResult<DeviceResponse>

    /** DELETE /devices/{id} — best-effort on logout so a logged-out phone
     *  stops receiving this account's pushes. */
    suspend fun deleteDevice(deviceId: Long): ApiResult<Unit>
}

@Singleton
class DefaultAuthApi @Inject constructor(
    private val client: ApiClient,
) : AuthApi {

    override suspend fun register(
        username: String,
        displayName: String,
        password: String,
    ): ApiResult<AuthResponse> =
        client.post("/auth/register", RegisterRequest(username, displayName, password), auth = false)

    override suspend fun login(username: String, password: String): ApiResult<AuthResponse> =
        client.post("/auth/login", LoginRequest(username, password), auth = false)

    override suspend fun changePassword(current: String, new: String): ApiResult<Unit> =
        client.post("/me/password", ChangePasswordRequest(current, new))

    override suspend fun logout(): ApiResult<Unit> =
        client.postEmpty("/auth/logout")

    override suspend fun me(): ApiResult<MeResponse> =
        client.get("/me")

    override suspend fun probe(candidateServerUrl: String): ApiResult<MeResponse> =
        client.get(
            "/me",
            auth = false,
            overrideBase = ServerUrlNormalizer.apiBase(candidateServerUrl),
        )

    override suspend fun registerDevice(pushToken: String?): ApiResult<DeviceResponse> =
        client.post("/devices", DeviceRequest(platform = "android", pushToken = pushToken))

    override suspend fun deleteDevice(deviceId: Long): ApiResult<Unit> =
        client.delete("/devices/$deviceId")
}
