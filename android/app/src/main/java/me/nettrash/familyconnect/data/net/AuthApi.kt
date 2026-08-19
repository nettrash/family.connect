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
    suspend fun me(): ApiResult<MeResponse>

    /**
     * Server-setup probe: unauthenticated GET /me against a *candidate*
     * URL (not yet saved). A live Family Connect server answers 401 with
     * the protocol error body — that 401 is the success signal.
     */
    suspend fun probe(candidateServerUrl: String): ApiResult<MeResponse>

    /** POST /devices — push hook; v1 sends a null token, delivery comes later. */
    suspend fun registerDevice(): ApiResult<DeviceResponse>
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

    override suspend fun registerDevice(): ApiResult<DeviceResponse> =
        client.post("/devices", DeviceRequest(platform = "android", pushToken = null))
}
