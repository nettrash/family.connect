/*
 * FamilyApi.kt
 * Family Connect (Android)
 *
 * Suspend wrappers over the Families endpoint table of docs/protocol.md.
 * Interface + impl split for testability (scripted fakes).
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/FamilyApi.swift
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.ResetPasswordRequest
import me.nettrash.familyconnect.data.net.dto.ApproveResponse
import me.nettrash.familyconnect.data.net.dto.CreateFamilyRequest
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.net.dto.JoinFamilyRequest
import me.nettrash.familyconnect.data.net.dto.JoinRequestsResponse
import me.nettrash.familyconnect.data.net.dto.JoinResponse
import me.nettrash.familyconnect.data.net.dto.PatchFamilyRequest
import me.nettrash.familyconnect.data.net.dto.RotateInviteCodeResponse
import javax.inject.Inject
import javax.inject.Singleton

interface FamilyApi {
    suspend fun create(name: String): ApiResult<FamilyResponse>
    suspend fun join(inviteCode: String): ApiResult<JoinResponse>
    suspend fun mine(): ApiResult<FamilyMineResponse>
    suspend fun rotateInviteCode(): ApiResult<RotateInviteCodeResponse>
    suspend fun setJoinPolicy(policy: String): ApiResult<FamilyResponse>
    suspend fun joinRequests(): ApiResult<JoinRequestsResponse>
    suspend fun approve(requestId: Long): ApiResult<ApproveResponse>
    suspend fun reject(requestId: Long): ApiResult<Unit>
    suspend fun leave(): ApiResult<Unit>
    suspend fun removeMember(userId: Long): ApiResult<Unit>

    /**
     * Owner-only: set a member's password without knowing their old one.
     * Every session that member has is revoked, so their devices return to
     * login — which is what makes this a recovery rather than a courtesy.
     */
    suspend fun resetMemberPassword(userId: Long, newPassword: String): ApiResult<Unit>
}

@Singleton
class DefaultFamilyApi @Inject constructor(
    private val client: ApiClient,
) : FamilyApi {

    override suspend fun create(name: String): ApiResult<FamilyResponse> =
        client.post("/families", CreateFamilyRequest(name))

    override suspend fun join(inviteCode: String): ApiResult<JoinResponse> =
        client.post("/families/join", JoinFamilyRequest(inviteCode))

    override suspend fun mine(): ApiResult<FamilyMineResponse> =
        client.get("/families/mine")

    override suspend fun rotateInviteCode(): ApiResult<RotateInviteCodeResponse> =
        client.postEmpty("/families/invite-code/rotate")

    override suspend fun setJoinPolicy(policy: String): ApiResult<FamilyResponse> =
        client.patch("/families/mine", PatchFamilyRequest(policy))

    override suspend fun joinRequests(): ApiResult<JoinRequestsResponse> =
        client.get("/families/join-requests")

    override suspend fun approve(requestId: Long): ApiResult<ApproveResponse> =
        client.postEmpty("/families/join-requests/$requestId/approve")

    override suspend fun reject(requestId: Long): ApiResult<Unit> =
        client.postEmpty("/families/join-requests/$requestId/reject")

    override suspend fun leave(): ApiResult<Unit> =
        client.postEmpty("/families/leave")

    override suspend fun removeMember(userId: Long): ApiResult<Unit> =
        client.delete("/families/members/$userId")

    override suspend fun resetMemberPassword(
        userId: Long,
        newPassword: String,
    ): ApiResult<Unit> =
        client.post("/families/members/$userId/password", ResetPasswordRequest(newPassword))
}
