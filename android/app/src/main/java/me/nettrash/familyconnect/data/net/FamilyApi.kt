/*
 * FamilyApi.kt
 * Family Connect (Android)
 *
 * Suspend wrappers over the Families endpoint table of docs/protocol.md.
 * Interface + impl split for testability (scripted fakes).
 *
 * Every family-level call names its family (docs/protocol.md, "Several
 * families"): `familyId` is put into the path — `/families/3/…` — and NULL
 * keeps the form without one, which means the home family and is the only
 * form a server from before several families knows ([FamilyPaths]).
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/FamilyApi.swift
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.AssistantReportResponse
import me.nettrash.familyconnect.data.net.dto.CreateAssistantReportRequest
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
import me.nettrash.familyconnect.data.net.dto.InvitationResponse
import me.nettrash.familyconnect.data.net.dto.InvitationsResponse
import me.nettrash.familyconnect.data.net.dto.InviteRequest
import javax.inject.Inject
import javax.inject.Singleton

/**
 * How a family is named in a path (docs/protocol.md, "Several families" —
 * "Naming a family"). Null is the home family in both spellings.
 */
object FamilyPaths {
    /** `/families/3` — or `/families/mine` for null: the reads and PATCH that always had a `mine`. */
    fun root(familyId: Long?): String = "/families/" + (familyId?.toString() ?: "mine")

    /** `/families/3` — or bare `/families` for null: the forms that never had a `mine` (`/families/leave`). */
    fun scoped(familyId: Long?): String = if (familyId == null) "/families" else "/families/$familyId"
}

interface FamilyApi {
    suspend fun create(name: String): ApiResult<FamilyResponse>
    suspend fun join(inviteCode: String): ApiResult<JoinResponse>
    suspend fun family(familyId: Long?): ApiResult<FamilyMineResponse>

    /**
     * What the family has sent. Visible to EVERY member, not just the owner
     * (protocol.md, "Family statistics").
     */
    suspend fun stats(familyId: Long?): ApiResult<FamilyStatsDto>
    suspend fun rotateInviteCode(familyId: Long?): ApiResult<RotateInviteCodeResponse>
    suspend fun setJoinPolicy(familyId: Long?, policy: String): ApiResult<FamilyResponse>

    /**
     * Owner-only: the ONE language the family speaks, or null to clear it
     * back to unset — which is not the same as choosing English
     * (protocol.md, "The family's language"). Null here becomes a real
     * JSON null on the wire; see PatchFamilyRequest.
     */
    suspend fun setLanguage(familyId: Long?, tag: String?): ApiResult<FamilyResponse>

    /**
     * Owner-only: whether a mention of the assistant in the family chat
     * carries the recent history of that chat. One family-wide switch,
     * no third state and no per-member override.
     */
    suspend fun setAiHistory(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse>

    /**
     * Owner-only: whether a photograph a member attaches in their OWN
     * assistant chat may be shown to the model (docs/protocol.md,
     * "Pictures"). Family-wide, no per-member override, and OFF unless
     * the owner turned it on — the opposite default to [setAiHistory],
     * which is the point rather than an inconsistency.
     *
     * It is one of THREE locks, and the only one an owner holds: the
     * operator must also have configured a vision deployment, and the
     * member must attach the picture to the question themselves. That
     * third one is never a remembered setting.
     */
    suspend fun setAiVision(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse>

    /**
     * Owner-only: whether an `@ai` mention in the family chat may ALSO be
     * shown that chat's most recent photographs — pictures nobody pointed
     * the assistant at (docs/protocol.md, "Recent photos from the family
     * chat"). Family-wide, OFF by default, and only openable while
     * `ai_vision` is on: the server answers `validation` to `true`
     * otherwise, and turns this off in the same write whenever
     * `ai_vision` goes off. Inert, not refused, while `ai_history` is
     * off — there is no transcript for it to widen.
     */
    suspend fun setAiHistoryPhotos(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse>

    /**
     * Owner-only: whether the assistant posts one unprompted good-morning
     * message a day into the family chat (docs/protocol.md, "The daily
     * greeting"). Family-wide, OFF by default, and — unlike
     * [setAiHistoryPhotos] — bound to none of the other switches, so it can
     * never be refused for the state of one of them.
     *
     * What it cannot promise is that a greeting arrives: that also needs the
     * operator's half, `MeResponse.greetingsEnabled`.
     */
    suspend fun setAiGreeting(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse>

    /**
     * Owner-only: the fifth switch — whether a mention may be shown the
     * profile pictures of the members named in its transcript
     * (docs/protocol.md, "Profile pictures of members"). Family-wide, OFF by
     * default, and only openable while `ai_vision` is on, exactly as
     * [setAiHistoryPhotos] is.
     */
    suspend fun setAiFaces(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse>

    /**
     * Owner-only: whether members may ask for the text of OTHER members'
     * recordings in the family chat (docs/protocol.md, "Transcripts on
     * request"). Family-wide, OFF by default, bound to no other switch.
     */
    suspend fun setAiTranscripts(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse>

    /**
     * Owner-only: whether the assistant may look things up for this family
     * (docs/protocol.md, "Looking things up"). Family-wide, OFF by default,
     * bound to no other switch, and inert on a server whose
     * `assistant.lookups` is absent.
     */
    suspend fun setAiLookups(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse>

    /**
     * Owner-only: the places whose weather the daily greeting mentions
     * (docs/protocol.md, "Today's weather, for places the owner chose").
     * The list REPLACES the stored one; `[]` clears it. The answer carries
     * the list as the server KEPT it — possibly shorter, or spelt with less
     * whitespace — and a server that predates the field ignores it, so the
     * answer, never the request, is what a caller shows.
     */
    suspend fun setGreetingPlaces(familyId: Long?, places: List<String>): ApiResult<FamilyResponse>
    suspend fun joinRequests(familyId: Long?): ApiResult<JoinRequestsResponse>
    suspend fun approve(familyId: Long?, requestId: Long): ApiResult<ApproveResponse>
    suspend fun reject(familyId: Long?, requestId: Long): ApiResult<Unit>
    /**
     * Leave the family, and learn who inherited it.
     *
     * Two answers from one endpoint: `204` when nothing passed on (an
     * ordinary member, or the last one — the family goes with them) and
     * `200 {new_owner_user_id}` when the caller was the owner and somebody
     * remained. An owner is NEVER refused; `owner_cannot_leave` is retired
     * and no endpoint raises it (docs/protocol.md, `POST /families/leave`).
     */
    suspend fun leave(familyId: Long?): ApiResult<Long?>
    suspend fun removeMember(familyId: Long?, userId: Long): ApiResult<Unit>

    /**
     * The most members this family admits; null CLEARS the cap, which is
     * NOT the same as setting it to the operator's ceiling.
     *
     * Its own method rather than a parameter on [setJoinPolicy], so a
     * patch carries exactly one field — a request that sent two would make
     * "which of these did the user actually change" a question the server
     * has to guess at.
     */
    suspend fun setMemberCap(familyId: Long?, cap: Int?): ApiResult<FamilyResponse>

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
    suspend fun report(familyId: Long?, reportedUserId: Long, reason: String, messageId: Long?): ApiResult<ReportResponse>

    /**
     * What the ASSISTANT got wrong (docs/protocol.md, "Reporting the
     * assistant"). A separate endpoint from [report], and deliberately not
     * under `/families`: the assistant belongs to no family, so the
     * member-report endpoint refuses it, and the people who run the server
     * read this one rather than the family owner. Reporting the same reply
     * twice answers 200 with the stored row and creates nothing.
     */
    suspend fun reportAssistant(
        messageId: Long,
        reason: String,
        note: String?,
    ): ApiResult<AssistantReportResponse>

    /** Owner-only: the open reports, oldest first. */
    suspend fun reports(familyId: Long?): ApiResult<ReportsResponse>

    /** Owner-only: mark one report handled. */
    suspend fun resolveReport(familyId: Long?, reportId: Long): ApiResult<Unit>

    /**
     * Owner-only: set a member's password without knowing their old one.
     * Every session that member has is revoked, so their devices return to
     * login — which is what makes this a recovery rather than a courtesy.
     */
    suspend fun resetMemberPassword(familyId: Long?, userId: Long, newPassword: String): ApiResult<Unit>

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
    suspend fun setMemberBirthday(familyId: Long?, userId: Long, month: Int, day: Int): ApiResult<MemberBirthdayResponse>

    /** Idempotent, like the profile one. */
    suspend fun clearMemberBirthday(familyId: Long?, userId: Long): ApiResult<Unit>

    /**
     * Owner-only: invite somebody who shares ANOTHER family with the owner
     * (docs/protocol.md, "Several families"). `201` with the new invitation,
     * or `200` with the pending one when they are already invited.
     */
    suspend fun invite(familyId: Long?, userId: Long): ApiResult<InvitationResponse>

    /** Owner-only: the family's pending invitations, oldest first. */
    suspend fun invitations(familyId: Long?): ApiResult<InvitationsResponse>

    /** Owner-only: withdraw one. Idempotent. */
    suspend fun withdrawInvitation(familyId: Long?, invitationId: Long): ApiResult<Unit>

    /** The invited person says yes: membership is immediate, and the answer is the family. */
    suspend fun acceptInvitation(invitationId: Long): ApiResult<FamilyResponse>

    /** The invited person says no. Idempotent; the owner is not pushed. */
    suspend fun declineInvitation(invitationId: Long): ApiResult<Unit>
}

@Singleton
class DefaultFamilyApi @Inject constructor(
    private val client: ApiClient,
) : FamilyApi {

    override suspend fun create(name: String): ApiResult<FamilyResponse> =
        client.post("/families", CreateFamilyRequest(name))

    override suspend fun join(inviteCode: String): ApiResult<JoinResponse> =
        client.post("/families/join", JoinFamilyRequest(inviteCode))

    override suspend fun family(familyId: Long?): ApiResult<FamilyMineResponse> =
        client.get(FamilyPaths.root(familyId))

    override suspend fun stats(familyId: Long?): ApiResult<FamilyStatsDto> =
        client.get(FamilyPaths.root(familyId) + "/stats")

    override suspend fun rotateInviteCode(familyId: Long?): ApiResult<RotateInviteCodeResponse> =
        client.postEmpty(FamilyPaths.scoped(familyId) + "/invite-code/rotate")

    override suspend fun setJoinPolicy(familyId: Long?, policy: String): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.joinPolicy(policy))

    override suspend fun setLanguage(familyId: Long?, tag: String?): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.language(tag))

    override suspend fun setAiHistory(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.aiHistory(enabled))

    override suspend fun setAiVision(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.aiVision(enabled))

    override suspend fun setAiHistoryPhotos(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.aiHistoryPhotos(enabled))

    override suspend fun setAiGreeting(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.aiGreeting(enabled))

    override suspend fun setAiFaces(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.aiFaces(enabled))

    override suspend fun setAiTranscripts(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.aiTranscripts(enabled))

    override suspend fun setAiLookups(familyId: Long?, enabled: Boolean): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.aiLookups(enabled))

    override suspend fun setGreetingPlaces(familyId: Long?, places: List<String>): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.greetingPlaces(places))

    override suspend fun joinRequests(familyId: Long?): ApiResult<JoinRequestsResponse> =
        client.get(FamilyPaths.scoped(familyId) + "/join-requests")

    override suspend fun approve(familyId: Long?, requestId: Long): ApiResult<ApproveResponse> =
        client.postEmpty(FamilyPaths.scoped(familyId) + "/join-requests/$requestId/approve")

    override suspend fun reject(familyId: Long?, requestId: Long): ApiResult<Unit> =
        client.postEmpty(FamilyPaths.scoped(familyId) + "/join-requests/$requestId/reject")

    override suspend fun leave(familyId: Long?): ApiResult<Long?> =
        when (val result = client.raw("POST", FamilyPaths.scoped(familyId) + "/leave", null)) {
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

    override suspend fun setMemberCap(familyId: Long?, cap: Int?): ApiResult<FamilyResponse> =
        client.patch(FamilyPaths.root(familyId), PatchFamilyRequest.maxMembers(cap))

    override suspend fun blockMember(userId: Long): ApiResult<Unit> =
        client.put("/families/members/$userId/block", Unit)

    override suspend fun unblockMember(userId: Long): ApiResult<Unit> =
        client.delete("/families/members/$userId/block")

    override suspend fun report(
        familyId: Long?,
        reportedUserId: Long,
        reason: String,
        messageId: Long?,
    ): ApiResult<ReportResponse> =
        client.post(FamilyPaths.scoped(familyId) + "/reports", CreateReportRequest(reportedUserId, reason, messageId))

    override suspend fun reportAssistant(
        messageId: Long,
        reason: String,
        note: String?,
    ): ApiResult<AssistantReportResponse> =
        client.post(
            "/reports/assistant",
            CreateAssistantReportRequest(messageId, reason, note?.trim()?.ifEmpty { null }),
        )

    override suspend fun reports(familyId: Long?): ApiResult<ReportsResponse> =
        client.get(FamilyPaths.scoped(familyId) + "/reports")

    override suspend fun resolveReport(familyId: Long?, reportId: Long): ApiResult<Unit> =
        client.postEmpty(FamilyPaths.scoped(familyId) + "/reports/$reportId/resolve")

    override suspend fun removeMember(familyId: Long?, userId: Long): ApiResult<Unit> =
        client.delete(FamilyPaths.scoped(familyId) + "/members/$userId")

    override suspend fun resetMemberPassword(
        familyId: Long?,
        userId: Long,
        newPassword: String,
    ): ApiResult<Unit> =
        client.post(FamilyPaths.scoped(familyId) + "/members/$userId/password", ResetPasswordRequest(newPassword))

    override suspend fun setMemberBirthday(
        familyId: Long?,
        userId: Long,
        month: Int,
        day: Int,
    ): ApiResult<MemberBirthdayResponse> =
        client.put(FamilyPaths.scoped(familyId) + "/members/$userId/birthday", BirthdayRequest(month, day))

    override suspend fun clearMemberBirthday(familyId: Long?, userId: Long): ApiResult<Unit> =
        client.delete(FamilyPaths.scoped(familyId) + "/members/$userId/birthday")

    override suspend fun invite(familyId: Long?, userId: Long): ApiResult<InvitationResponse> =
        client.post(FamilyPaths.root(familyId) + "/invitations", InviteRequest(userId))

    override suspend fun invitations(familyId: Long?): ApiResult<InvitationsResponse> =
        client.get(FamilyPaths.root(familyId) + "/invitations")

    override suspend fun withdrawInvitation(familyId: Long?, invitationId: Long): ApiResult<Unit> =
        client.delete(FamilyPaths.root(familyId) + "/invitations/$invitationId")

    override suspend fun acceptInvitation(invitationId: Long): ApiResult<FamilyResponse> =
        client.postEmpty("/invitations/$invitationId/accept")

    override suspend fun declineInvitation(invitationId: Long): ApiResult<Unit> =
        client.postEmpty("/invitations/$invitationId/decline")
}
