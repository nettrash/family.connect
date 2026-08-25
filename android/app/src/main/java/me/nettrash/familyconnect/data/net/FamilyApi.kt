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
import me.nettrash.familyconnect.data.net.dto.BirthdayRequest
import me.nettrash.familyconnect.data.net.dto.CreateFamilyRequest
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.net.dto.JoinFamilyRequest
import me.nettrash.familyconnect.data.net.dto.JoinRequestsResponse
import me.nettrash.familyconnect.data.net.dto.JoinResponse
import me.nettrash.familyconnect.data.net.dto.MemberBirthdayResponse
import me.nettrash.familyconnect.data.net.dto.PatchFamilyRequest
import me.nettrash.familyconnect.data.net.dto.FamilyStatsDto
import me.nettrash.familyconnect.data.net.dto.RotateInviteCodeResponse
import javax.inject.Inject
import javax.inject.Singleton

interface FamilyApi {
    suspend fun create(name: String): ApiResult<FamilyResponse>
    suspend fun join(inviteCode: String): ApiResult<JoinResponse>
    suspend fun mine(): ApiResult<FamilyMineResponse>

    /**
     * What the family has sent. Visible to EVERY member, not just the owner
     * (protocol.md, "Family statistics").
     */
    suspend fun stats(): ApiResult<FamilyStatsDto>
    suspend fun rotateInviteCode(): ApiResult<RotateInviteCodeResponse>
    suspend fun setJoinPolicy(policy: String): ApiResult<FamilyResponse>

    /**
     * Owner-only: the ONE language the family speaks, or null to clear it
     * back to unset — which is not the same as choosing English
     * (protocol.md, "The family's language"). Null here becomes a real
     * JSON null on the wire; see PatchFamilyRequest.
     */
    suspend fun setLanguage(tag: String?): ApiResult<FamilyResponse>

    /**
     * Owner-only: whether a mention of the assistant in the family chat
     * carries the recent history of that chat. One family-wide switch,
     * no third state and no per-member override.
     */
    suspend fun setAiHistory(enabled: Boolean): ApiResult<FamilyResponse>
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

    /**
     * Owner-only: fill in a birthday for somebody in the family — a
     * parent for a child, typically, since the child is never going to
     * open a settings screen to type it.
     *
     * Unlike the password reset, the owner MAY name THEMSELVES here: there
     * is no proof being skipped, and refusing would make every roster
     * screen carry a special case for exactly one row (protocol.md,
     * "Birthdays").
     */
    suspend fun setMemberBirthday(userId: Long, month: Int, day: Int): ApiResult<MemberBirthdayResponse>

    /** Idempotent, like the profile one. */
    suspend fun clearMemberBirthday(userId: Long): ApiResult<Unit>
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

    override suspend fun stats(): ApiResult<FamilyStatsDto> =
        client.get("/families/mine/stats")

    override suspend fun rotateInviteCode(): ApiResult<RotateInviteCodeResponse> =
        client.postEmpty("/families/invite-code/rotate")

    override suspend fun setJoinPolicy(policy: String): ApiResult<FamilyResponse> =
        client.patch("/families/mine", PatchFamilyRequest.joinPolicy(policy))

    override suspend fun setLanguage(tag: String?): ApiResult<FamilyResponse> =
        client.patch("/families/mine", PatchFamilyRequest.language(tag))

    override suspend fun setAiHistory(enabled: Boolean): ApiResult<FamilyResponse> =
        client.patch("/families/mine", PatchFamilyRequest.aiHistory(enabled))

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

    override suspend fun setMemberBirthday(
        userId: Long,
        month: Int,
        day: Int,
    ): ApiResult<MemberBirthdayResponse> =
        client.put("/families/members/$userId/birthday", BirthdayRequest(month, day))

    override suspend fun clearMemberBirthday(userId: Long): ApiResult<Unit> =
        client.delete("/families/members/$userId/birthday")
}
