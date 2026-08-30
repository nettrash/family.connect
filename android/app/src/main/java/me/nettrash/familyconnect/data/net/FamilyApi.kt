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
import me.nettrash.familyconnect.data.net.dto.CreateReportRequest
import me.nettrash.familyconnect.data.net.dto.LeaveFamilyResponse
import me.nettrash.familyconnect.data.net.dto.ReportResponse
import me.nettrash.familyconnect.data.net.dto.ReportsResponse
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
    /**
     * Leave the family, and learn who inherited it.
     *
     * Two answers from one endpoint: `204` when nothing passed on (an
     * ordinary member, or the last one — the family goes with them) and
     * `200 {new_owner_user_id}` when the caller was the owner and somebody
     * remained. An owner is NEVER refused; `owner_cannot_leave` is retired
     * and no endpoint raises it (docs/protocol.md, `POST /families/leave`).
     */
    suspend fun leave(): ApiResult<Long?>
    suspend fun removeMember(userId: Long): ApiResult<Unit>

    /**
     * The most members this family admits; null CLEARS the cap, which is
     * NOT the same as setting it to the operator's ceiling.
     *
     * Its own method rather than a parameter on [setJoinPolicy], so a
     * patch carries exactly one field — a request that sent two would make
     * "which of these did the user actually change" a question the server
     * has to guess at.
     */
    suspend fun setMemberCap(cap: Int?): ApiResult<FamilyResponse>

    /**
     * Block a member. ANY member may block any other, the OWNER INCLUDED —
     * there is no owner check here and that is the point
     * (docs/protocol.md, "Blocking a member").
     */
    suspend fun blockMember(userId: Long): ApiResult<Unit>
    suspend fun unblockMember(userId: Long): ApiResult<Unit>

    /**
     * Report a member, optionally naming one of their messages. Raising a
     * report that matches an OPEN row returns that row and creates
     * nothing, so a double tap is not two rows in the owner's list.
     */
    suspend fun report(reportedUserId: Long, reason: String, messageId: Long?): ApiResult<ReportResponse>

    /** Owner-only: the open reports, oldest first. */
    suspend fun reports(): ApiResult<ReportsResponse>

    /** Owner-only: mark one report handled. */
    suspend fun resolveReport(reportId: Long): ApiResult<Unit>

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

    override suspend fun leave(): ApiResult<Long?> =
        when (val result = client.raw("POST", "/families/leave", null)) {
            is ApiResult.Ok ->
                // A 204 has NO body, and that is the ordinary case here
                // rather than a malformed answer — decoding it as JSON
                // would turn every member's departure into an error.
                if (result.value.isBlank()) {
                    ApiResult.Ok(null)
                } else {
                    when (val decoded = client.decode<LeaveFamilyResponse>(result)) {
                        is ApiResult.Ok -> ApiResult.Ok(decoded.value.newOwnerUserId)
                        // A 200 whose body will not parse still LEFT the
                        // family; report it as "nobody inherited" rather
                        // than as a failure the caller would retry.
                        else -> ApiResult.Ok(null)
                    }
                }
            // `ApiResult` is covariant and both failures are
            // `ApiResult<Nothing>`, so they pass straight through.
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    override suspend fun setMemberCap(cap: Int?): ApiResult<FamilyResponse> =
        client.patch("/families/mine", PatchFamilyRequest.maxMembers(cap))

    override suspend fun blockMember(userId: Long): ApiResult<Unit> =
        client.put("/families/members/$userId/block", Unit)

    override suspend fun unblockMember(userId: Long): ApiResult<Unit> =
        client.delete("/families/members/$userId/block")

    override suspend fun report(
        reportedUserId: Long,
        reason: String,
        messageId: Long?,
    ): ApiResult<ReportResponse> =
        client.post("/families/reports", CreateReportRequest(reportedUserId, reason, messageId))

    override suspend fun reports(): ApiResult<ReportsResponse> =
        client.get("/families/reports")

    override suspend fun resolveReport(reportId: Long): ApiResult<Unit> =
        client.postEmpty("/families/reports/$reportId/resolve")

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
