/*
 * FamilyRepository.kt
 * Family Connect (Android)
 *
 * Family membership: roster (Room `members` table), invite code, join
 * policy, join requests, leave/remove. Also the sink for the
 * member_joined / member_left / member_deleted / family_owner WebSocket
 * frames — applied to the roster live, and a member_left carrying *my*
 * user id escalates to SessionRepository (we've been kicked).
 *
 * The roster holds BOTH arrays `GET /families/mine` answers with:
 * `members` and `former_members`, the second flagged deleted. One table,
 * because a stored message has to be able to name a sender whose account
 * is gone; one filter (MemberDao.observeActiveMembers), because nothing
 * that offers an action on a person may include them.
 *
 * Several families (#88; docs/protocol.md, "Several families"): every
 * family-level call names the CURRENT family, every frame is applied to
 * the family its `family_id` names — never to "my family" — and the roster
 * of EVERY family the account is in is kept, one row per (family, member).
 * Creating another family, joining one with a code and accepting an
 * invitation all add a family and switch to it; leaving drops that family
 * alone. Invitations to this account live in settings; the owner's side of
 * them (invite, the pending list, withdraw) is here too.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Repo/FamilyRepository.swift
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AuthApi
import me.nettrash.familyconnect.data.net.FamilyApi
import me.nettrash.familyconnect.data.net.dto.AssistantReportResponse
import me.nettrash.familyconnect.data.net.dto.BirthdayDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.net.dto.InvitationDto
import me.nettrash.familyconnect.data.net.dto.JoinRequestsResponse
import me.nettrash.familyconnect.data.net.dto.JoinResponse
import me.nettrash.familyconnect.data.net.dto.RotateInviteCodeResponse
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.data.settings.familyKey
import me.nettrash.familyconnect.data.settings.localFamilyKey
import me.nettrash.familyconnect.data.settings.mirrorFamilySwitches
import me.nettrash.familyconnect.data.settings.pathFamily
import me.nettrash.familyconnect.di.AppScope
import javax.inject.Inject
import javax.inject.Singleton
import me.nettrash.familyconnect.data.net.dto.ReportResponse
import me.nettrash.familyconnect.data.net.dto.ReportsResponse

@Singleton
class FamilyRepository @Inject constructor(
    private val familyApi: FamilyApi,
    // My OWN birthday is a profile endpoint, not a family one — the same
    // split the two password endpoints already live on.
    private val authApi: AuthApi,
    private val memberDao: MemberDao,
    private val settings: SettingsRepository,
    private val sessionRepository: SessionRepository,
    socket: ChatSocket,
    @param:AppScope private val scope: CoroutineScope,
) {

    private val _familiesAdded = MutableSharedFlow<Long>(extraBufferCapacity = 4)

    /**
     * A family this account just came into — created, joined with a code, an
     * invitation accepted, or a `member_joined` naming us from another
     * device. Its chats, board and pack are a resync away; ChatSocketManager
     * runs one off this.
     */
    val familiesAdded: SharedFlow<Long> = _familiesAdded

    private val _invitationsChanged = MutableSharedFlow<Unit>(extraBufferCapacity = 4)

    /**
     * An `invitation` or `invitation_removed` frame arrived — the owner's
     * pending list re-reads off this (docs/protocol.md, "Several families").
     */
    val invitationsChanged: SharedFlow<Unit> = _invitationsChanged

    init {
        scope.launch {
            socket.frames.collect { frame ->
                when (frame) {
                    // NOT `upsertAll`: `member_joined.user` is a
                    // `UserBrief` (docs/protocol.md, "Server → client")
                    // with no birthday field on it at all, so the DTO
                    // always decodes with null there — and a wholesale
                    // upsert would write that null over a birthday the
                    // roster already knew, every time somebody leaves and
                    // is re-approved. An absent field never wipes a
                    // stored one; [MemberDao.upsertLeavingBirthday] is
                    // where that rule is spelled out.
                    //
                    // Applied to the family the frame names. One naming ME
                    // for a family this device does not hold means "you are
                    // in a new family" (an approval, an accepted invitation,
                    // a family created on another device): `/me` and that
                    // family are read again (docs/protocol.md, "Several
                    // families").
                    //
                    // One naming ME for a family this device DOES hold is
                    // the echo of a door I just went through here — most
                    // often a family I created, of which I am the owner —
                    // and changes nothing about my role in it.
                    is ServerFrame.MemberJoined -> {
                        val state = settings.state.first()
                        val held = state.families.isEmpty() || state.families.any { it.id == frame.familyId }
                        if (frame.user.id == state.myUserId && !held) {
                            sessionRepository.refreshMe()
                            refreshFamily(frame.familyId)
                            _familiesAdded.tryEmit(frame.familyId)
                        } else if (held) {
                            memberDao.upsertLeavingBirthday(
                                MemberEntity(
                                    familyId = state.localFamilyKey(frame.familyId),
                                    userId = frame.user.id,
                                    username = frame.user.username,
                                    displayName = frame.user.displayName,
                                    // A NEW row's role, and a returning
                                    // one's: joiners are plain members. It
                                    // never lands on a row still live
                                    // (MemberDao.updateLeavingBirthday) —
                                    // the frame naming ME for a family this
                                    // device just created arrives after
                                    // `create` read the roster with me as
                                    // its owner, and must not demote me.
                                    // My own row, if the frame wins the race
                                    // with that read, takes the role my
                                    // entry already says (the creator's is
                                    // "owner"), so a failed read leaves no
                                    // "member" behind either.
                                    role = if (frame.user.id == state.myUserId) {
                                        state.families.firstOrNull { it.id == frame.familyId }?.role ?: "member"
                                    } else {
                                        "member"
                                    },
                                    avatarVersion = frame.user.avatarVersion,
                                    // On the owner's copy only (docs/protocol.md,
                                    // "Several families"). A newcomer who came
                                    // by approval, invitation or open join and
                                    // is also in a family this owner does not
                                    // own arrives with false, and is not
                                    // offered a reset the server would refuse.
                                    // Null — not told — keeps what is stored.
                                    passwordReset = frame.passwordReset,
                                ),
                            )
                        }
                    }
                    // Out of THAT family. Naming me: only that family's
                    // chats, board, pack, roster and cursors go.
                    is ServerFrame.MemberLeft -> {
                        memberDao.markLeft(settings.state.first().localFamilyKey(frame.familyId), frame.userId)
                        val myId = settings.state.first().myUserId
                        if (frame.userId == myId) sessionRepository.leftFamily(frame.familyId)
                    }
                    // The ONE frame whose job is to wipe stored fields, so
                    // it is written deliberately rather than upserted —
                    // see MemberDao.writeTombstone. The row STAYS: their
                    // messages, notes and reactions are still in the
                    // family's history and still have to be named.
                    //
                    // This is the ROSTER half of the frame only. The same
                    // frame also takes the direct chat with them away —
                    // that half is ChatRepository's, because the chat
                    // store is not this repository's to write.
                    is ServerFrame.MemberDeleted -> memberDao.writeTombstone(
                        // The account is gone from every family at once; the
                        // row this device may lack is filed under the family
                        // the frame names, or the one shown when it names none.
                        familyId = settings.state.first().localFamilyKey(frame.familyId),
                        userId = frame.member.id,
                        username = frame.member.username,
                        displayName = frame.member.displayName,
                    )
                    // Ownership moved because the owner deleted their
                    // account. The roster row moves with it, and if the
                    // new owner is US the stored status flips now rather
                    // than at the next GET /me — that is the whole point
                    // of the frame (protocol.md, "Server → client").
                    // Since several families: THAT family's owner, and the
                    // role on that family's entry, which is the status only
                    // when it is the family shown.
                    is ServerFrame.FamilyOwner -> {
                        val state = settings.state.first()
                        memberDao.setOwner(state.localFamilyKey(frame.familyId), frame.userId)
                        val entry = state.families.firstOrNull { it.id == frame.familyId }
                        if (entry != null) {
                            when {
                                frame.userId == state.myUserId ->
                                    sessionRepository.setFamilyRole(frame.familyId, FamilyList.OWNER)
                                entry.role == FamilyList.OWNER ->
                                    sessionRepository.setFamilyRole(frame.familyId, "member")
                            }
                        } else if (state.families.isEmpty() &&
                            (state.familyStatus == FamilyStatus.MEMBER || state.familyStatus == FamilyStatus.OWNER)
                        ) {
                            settings.setFamilyStatus(
                                if (frame.userId == state.myUserId) {
                                    FamilyStatus.OWNER
                                } else {
                                    FamilyStatus.MEMBER
                                },
                            )
                        }
                    }
                    // Reaches only the blocker's own connections, so
                    // there is nobody else's state to consider. A
                    // state-set, not an event: an unblock is this same
                    // frame with `false` (docs/protocol.md, "Blocking a
                    // member"). A latency optimisation over the full list
                    // on `/me`, so it goes through the same store.
                    is ServerFrame.MemberBlocked ->
                        applyBlockLocally(frame.userId, frame.blocked)
                    is ServerFrame.Invitation -> onInvitation(frame.invitation)
                    is ServerFrame.InvitationRemoved -> onInvitationRemoved(frame.invitationId)
                    else -> Unit
                }
            }
        }
    }

    /**
     * Fold one id into the stored block list.
     *
     * Read-modify-write of the whole set, because [SettingsRepository]
     * owns it as a complete state-set and there is exactly one function in
     * the app that writes it.
     */
    private suspend fun applyBlockLocally(userId: Long, blocked: Boolean) {
        val current = settings.state.first().blockedUserIds
        val next = if (blocked) current + userId else current - userId
        if (next != current) settings.setBlockedUserIds(next)
    }

    // -- Which family -----------------------------------------------------------

    private suspend fun state(): SettingsState = settings.state.first()

    /** The current family's local key (0 before there is one). */
    private suspend fun currentFamily(): Long = state().familyKey

    /** The current family as a request path names it — null on a server from before. */
    private suspend fun currentPath(): Long? = state().let { it.pathFamily(it.familyKey) }

    /**
     * `family_not_found` means the family is gone for this account — or was
     * never there (docs/protocol.md, "Several families"): re-read `/me`,
     * which reconciles the list and drops what is no longer ours.
     */
    private fun <T> ApiResult<T>.resyncIfGone(): ApiResult<T> {
        if (this is ApiResult.HttpError && code == FAMILY_NOT_FOUND) {
            scope.launch { sessionRepository.refreshMe() }
        }
        return this
    }

    /** Show [familyId] — the switcher's tap. */
    suspend fun switchTo(familyId: Long) = sessionRepository.switchFamily(familyId)

    /**
     * Open a push about [familyId] — or, for a direct chat's push that names
     * none, about the families [chatFamilyIds] lists — in its family
     * (docs/protocol.md, "Several families" — "Pushes").
     */
    suspend fun switchForPush(familyId: Long?, chatFamilyIds: List<Long>?) {
        val state = state()
        val target = familyId?.takeIf { id -> state.families.any { it.id == id } }
            ?: FamilyList.familyForChat(chatFamilyIds, state.currentFamilyId, state.families)
            ?: return
        if (target != state.currentFamilyId) sessionRepository.switchFamily(target)
    }

    /**
     * A family this account just came into: on the list, shown, its roster
     * read, and a resync asked for its chats, board and pack.
     */
    private suspend fun adopt(family: FamilyDto, role: String) {
        sessionRepository.addFamily(family.copy(role = family.role ?: role))
        refreshFamily(family.id)
        _familiesAdded.tryEmit(family.id)
    }

    // -- Invitations ------------------------------------------------------------

    /**
     * An invitation frame. Mine (I am the one invited): onto the banner.
     * The owner's copy only tells the owner's screen to re-read its list.
     */
    private suspend fun onInvitation(invitation: InvitationDto) {
        val state = state()
        if (invitation.user.id == state.myUserId) {
            settings.setInvitations(state.invitations.filter { it.id != invitation.id } + invitation)
        }
        _invitationsChanged.tryEmit(Unit)
    }

    private suspend fun onInvitationRemoved(invitationId: Long) {
        dropInvitation(invitationId)
        _invitationsChanged.tryEmit(Unit)
    }

    private suspend fun dropInvitation(invitationId: Long) {
        val held = state().invitations
        if (held.any { it.id == invitationId }) settings.setInvitations(held.filter { it.id != invitationId })
    }

    /**
     * "Join" on an invitation: membership is immediate, the family is added
     * and shown. `invitation_not_pending` means it is gone — accepted
     * elsewhere, withdrawn, declined — and it leaves the banner; the caller
     * says so.
     */
    suspend fun acceptInvitation(invitationId: Long): ApiResult<FamilyResponse> {
        val result = familyApi.acceptInvitation(invitationId)
        when (result) {
            is ApiResult.Ok -> {
                dropInvitation(invitationId)
                adopt(result.value.family, role = "member")
            }
            is ApiResult.HttpError -> if (result.code == INVITATION_NOT_PENDING) dropInvitation(invitationId)
            is ApiResult.NetworkError -> Unit
        }
        return result
    }

    /** "Decline". Idempotent on the server, so it leaves the banner either way it answers. */
    suspend fun declineInvitation(invitationId: Long): ApiResult<Unit> {
        val result = familyApi.declineInvitation(invitationId)
        if (result is ApiResult.Ok ||
            (result is ApiResult.HttpError && result.code == INVITATION_NOT_PENDING)
        ) {
            dropInvitation(invitationId)
        }
        return result
    }

    /** Owner: invite [userId] — a member of another of the owner's families — into the current one. */
    suspend fun invite(userId: Long): ApiResult<InvitationDto> =
        when (val result = familyApi.invite(currentPath(), userId).resyncIfGone()) {
            is ApiResult.Ok -> ApiResult.Ok(result.value.invitation)
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    /** Owner: the current family's pending invitations, oldest first. */
    suspend fun pendingInvitations(): ApiResult<List<InvitationDto>> =
        when (val result = familyApi.invitations(currentPath()).resyncIfGone()) {
            is ApiResult.Ok -> ApiResult.Ok(result.value.invitations)
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    /** Owner: "Withdraw Invitation". */
    suspend fun withdrawInvitation(invitationId: Long): ApiResult<Unit> =
        familyApi.withdrawInvitation(currentPath(), invitationId).resyncIfGone()

    /** Every live membership this device holds — the Invite picker's source. */
    fun observeAllMemberships(): Flow<List<MemberEntity>> = memberDao.observeAllActiveMemberships()

    /**
     * Block a member. ANY member may block any other, the OWNER INCLUDED.
     *
     * The request FIRST, then the local write — never optimistic. An
     * optimistic block that then failed would hide rows the reader does not
     * know are hidden, in a feature with no error surface and no badge to
     * notice it by.
     */
    suspend fun block(userId: Long): ApiResult<Unit> =
        familyApi.blockMember(userId).also {
            if (it is ApiResult.Ok) applyBlockLocally(userId, blocked = true)
        }

    /**
     * Report a member, optionally naming one of their messages.
     *
     * Nothing local changes: a report is a message to the family's owner,
     * not a state this device holds. Raising one that matches an OPEN
     * report returns that row and creates nothing, so a double tap is not
     * two rows in the owner's list (docs/protocol.md, "Reporting a
     * member").
     */
    /** Owner-only: the open reports, oldest first. */
    suspend fun reports(): ApiResult<ReportsResponse> = familyApi.reports(currentPath()).resyncIfGone()

    /** Owner-only: take one report off the list. */
    suspend fun resolveReport(reportId: Long): ApiResult<Unit> =
        familyApi.resolveReport(currentPath(), reportId).resyncIfGone()

    /** The most members this family admits; null CLEARS the cap. */
    suspend fun setMemberCap(cap: Int?): ApiResult<FamilyResponse> =
        familyApi.setMemberCap(currentPath(), cap).resyncIfGone().alsoMirror()

    /** To the CURRENT family's owner (docs/protocol.md, `POST /families/{id}/reports`). */
    suspend fun report(
        reportedUserId: Long,
        reason: String,
        messageId: Long?,
    ): ApiResult<ReportResponse> = familyApi.report(currentPath(), reportedUserId, reason, messageId).resyncIfGone()

    /** Statistics are the current family's, for every member (docs/protocol.md, "Family statistics"). */
    suspend fun stats() = familyApi.stats(currentPath()).resyncIfGone()

    /**
     * An owner's PATCH answered with the family as it now stands: its entry
     * on the list follows, and — when it is the family shown — the mirrored
     * switches with it.
     */
    private suspend fun ApiResult<FamilyResponse>.alsoMirror(): ApiResult<FamilyResponse> {
        if (this is ApiResult.Ok) {
            // With no list yet the one family is the one shown.
            if (state().families.isEmpty()) {
                settings.mirrorFamilySwitches(value.family)
            } else {
                sessionRepository.updateFamily(value.family)
            }
        }
        return this
    }

    /**
     * Report an ASSISTANT reply. A separate path from [report]: the assistant
     * belongs to no family, so that endpoint refuses it, and this one is read
     * by the people who run the server rather than by the family owner
     * (docs/protocol.md, "Reporting the assistant").
     */
    suspend fun reportAssistant(
        messageId: Long,
        reason: String,
        note: String?,
    ): ApiResult<AssistantReportResponse> = familyApi.reportAssistant(messageId, reason, note)

    /**
     * This member's own answer to the assistant question, and nobody
     * else's (docs/protocol.md, "Consenting to the assistant").
     *
     * Here beside [reportAssistant] for that call's reason: neither is the
     * family's business, but this is where the client's assistant state is
     * kept and where the composer reads it from. The stamp the server
     * answers with is written straight into settings, so the composer
     * stops asking — or starts again — without waiting for the next `/me`.
     *
     * Answers whether it was recorded: false leaves the composer asking,
     * which is the safe direction, and the caller says so.
     */
    suspend fun setAssistantConsent(granted: Boolean): Boolean =
        when (val result = authApi.setAssistantConsent(granted)) {
            is ApiResult.Ok -> {
                settings.setAssistantConsentAt(result.value.assistantConsentAt)
                // Withdrawing the assistant consent withdraws the lookup
                // consent with it, on the server, in the same write
                // (docs/protocol.md, "Consenting to the assistant", amended
                // 2026-10-03). The answer names only the first stamp, so
                // the second is cleared here rather than left showing
                // "Agreed" until the next `/me`.
                if (result.value.assistantConsentAt == null) settings.setAssistantLookupConsentAt(null)
                true
            }
            else -> false
        }

    /**
     * This member's own answer to the LOOKUP question — whether the
     * assistant may send a short query it wrote from their words to the
     * providers `assistant.lookups` names (docs/protocol.md, "Consenting to
     * the assistant", amended 2026-10-03). Never sent on anybody's behalf
     * and never inferred from [setAssistantConsent].
     *
     * Answers whether it was recorded. False leaves the lookup consent
     * where it was: for a grant that means no lookups, the safe direction;
     * for a withdrawal the caller has to say it failed.
     */
    suspend fun setAssistantLookupConsent(granted: Boolean): Boolean =
        when (val result = authApi.setAssistantLookupConsent(granted)) {
            is ApiResult.Ok -> {
                settings.setAssistantLookupConsentAt(result.value.assistantLookupConsentAt)
                true
            }
            else -> false
        }

    /**
     * "Agree With Lookups" / "Agree Without Lookups": the assistant consent,
     * and — only when asked for, and only once that one is recorded, since
     * the server refuses a lookup grant without it — the lookup consent.
     */
    suspend fun agreeToAssistant(withLookups: Boolean): AgreeOutcome {
        if (!setAssistantConsent(true)) return AgreeOutcome(assistant = false, lookups = false)
        val lookups = withLookups && setAssistantLookupConsent(true)
        return AgreeOutcome(assistant = true, lookups = lookups)
    }

    /** What [agreeToAssistant] managed to record. */
    data class AgreeOutcome(val assistant: Boolean, val lookups: Boolean)

    suspend fun unblock(userId: Long): ApiResult<Unit> =
        familyApi.unblockMember(userId).also {
            if (it is ApiResult.Ok) applyBlockLocally(userId, blocked = false)
        }

    /** Everyone ever seen — the name-resolution feed. */
    fun observeMembers(): Flow<List<MemberEntity>> = memberDao.observeMembers()

    /** Only those still in the CURRENT family — for pickers and the admin list. */
    fun observeActiveMembers(): Flow<List<MemberEntity>> = memberDao.observeCurrentActiveMembers(settings)

    /**
     * Every family this account is in, for the resync to read one by one —
     * or, on an install that has not read `/me` since several families, the
     * one family it has.
     */
    suspend fun heldFamilyIds(): List<Long> = state().let { s ->
        s.families.map { it.id }.ifEmpty { listOf(s.familyKey) }
    }

    /** The CURRENT family's roster read — what every screen that says "the family" means. */
    suspend fun refreshMine(): ApiResult<FamilyMineResponse> = refreshFamily(currentFamily())

    /**
     * `GET /families/{id}` → that family's roster upsert. invite_code present
     * for owners only. Resync reads EVERY family through here.
     */
    suspend fun refreshFamily(familyId: Long): ApiResult<FamilyMineResponse> {
        val asked = state()
        val result = familyApi.family(asked.pathFamily(familyId)).resyncIfGone()
        if (result is ApiResult.Ok) {
            // Filed under the id the answer carries — the one asked for on a
            // server with several families, the home family's on one from
            // before, which only ever answers `mine`. An install that has not
            // read `/me` since several families has no list yet, and files
            // under the key it asked with, which is what its screens read and
            // what the first `/me` claims for the home family.
            // A server from before answers `mine` for whatever was asked, and
            // that IS the family asked about — the one family it has.
            val key = if (asked.families.isEmpty() || asked.pathFamily(familyId) == null) {
                familyId
            } else {
                asked.localFamilyKey(result.value.family.id)
            }
            // Upsert, then flag whoever the server no longer lists.
            //
            // NOT delete-then-insert: that dropped departed members
            // entirely, and every message they had ever sent lost its
            // name and face. The upsert also clears `hasLeft` for anyone
            // who rejoined, since the row is replaced wholesale.
            val roster = result.value.members.map {
                MemberEntity(
                    familyId = key,
                    userId = it.id,
                    username = it.username,
                    displayName = it.displayName,
                    // Always present on a LIVE member; the fallback is
                    // only here because the field is optional on the wire
                    // for the tombstones below.
                    role = it.role ?: "member",
                    avatarVersion = it.avatarVersion,
                    // A birthday change raises no frame and no push
                    // (protocol.md, "Birthdays"), so this refresh is where
                    // every device except the one that made the change
                    // learns about it.
                    birthdayMonth = it.birthday?.month,
                    birthdayDay = it.birthday?.day,
                    // The owner's read says it for every member; any other
                    // read and a server from before say nothing, which is the
                    // old rule. The whole roster is the server's answer, so
                    // it is written as it reads.
                    passwordReset = it.passwordReset,
                )
            }
            // `former_members` goes into the SAME table, flagged — that
            // is what lets a stored message still name its sender
            // (protocol.md, "Deleting an account"). They are NOT members:
            // `deleted = 1` keeps them out of observeActiveMembers, which
            // is what every picker, admin list and roster reads.
            //
            // A tombstone carries no role, no picture and no birthday, and
            // the entity is written whole here on purpose: unlike a frame
            // that merely omits a field, the server is stating that those
            // values are gone.
            val tombstones = result.value.formerMembers.map {
                MemberEntity(
                    familyId = key,
                    userId = it.id,
                    username = it.username,
                    displayName = it.displayName,
                    role = "member",
                    avatarVersion = 0,
                    hasLeft = true,
                    deleted = true,
                )
            }
            // Through upsertRoster rather than upsertAll: a live entry
            // for somebody this device already holds a tombstone for is a
            // response the server computed before the deletion landed, and
            // writing it would resurrect them. See MemberDao.upsertRoster.
            memberDao.upsertRoster(roster, tombstones)
            if (roster.isNotEmpty()) {
                // Guarded: `NOT IN ()` is a syntax error in SQLite, and an
                // empty roster cannot happen anyway — the caller is in the
                // family they just read.
                memberDao.markLeftExcept(key, roster.map { it.userId })
            }
            // The family's entry on the list follows the read, and the
            // mirrored name and switches with it when it is the family
            // shown. An account whose list is not known yet (an install
            // that has not read `/me` since several families) has only the
            // one family, which is the one shown.
            val held = state()
            if (held.families.any { it.id == key }) {
                sessionRepository.updateFamily(result.value.family)
            } else if (held.families.isEmpty()) {
                settings.setFamilyName(result.value.family.name)
                settings.mirrorFamilySwitches(result.value.family)
            }
            // The sticker pack's two ceilings — and, by their ABSENCE, the
            // news that this server predates packs. Unconditional, null
            // included: it is the whole capability check, so a server that
            // stops saying it must take the sticker button away again
            // (docs/protocol.md, "Sticker pack").
            settings.setPackLimits(result.value.maxPackItems, result.value.maxPackItemBytes)
            // Video messages' limits, the same capability check by absence
            // (#79): a server without them is offered no video entry, and
            // one that stops saying them takes it away again.
            settings.setRoundVideoLimits(result.value.maxRoundVideoMs, result.value.maxRoundVideoBytes)
            // The assistant is NOT upserted as a member — it belongs to no
            // family, so it appears in no roster. Kept aside purely so the
            // family chat can put a name on its messages and the composer
            // knows whether to offer `@ai`.
            settings.setAssistant(
                result.value.assistant?.userId,
                result.value.assistant?.displayName,
                // What this SERVER can do with pictures. Kept beside the
                // assistant because it is part of the same capability
                // answer and goes with it on logout: another server may
                // have neither deployment (docs/protocol.md, "Pictures").
                vision = result.value.assistant?.vision == true,
                images = result.value.assistant?.images == true,
                // WHO answers, for the consent screen to name verbatim.
                // Null turns the assistant off in this client entirely: a
                // screen that cannot say where the words go cannot ask
                // the question (protocol.md, "Consenting to the
                // assistant").
                processor = result.value.assistant?.processor,
                // Whether a recording can be turned into text here, and
                // the largest stored copy the server will send
                // (docs/protocol.md, "Transcripts on request").
                transcribe = result.value.assistant?.transcribe == true,
                transcribeMaxBytes = result.value.assistant?.transcribeMaxBytes,
                // Who a lookup would go to, for the consent screen and the
                // owner's switch to name (docs/protocol.md, "Looking
                // things up"). Absent when the server has no source.
                lookups = result.value.assistant?.lookups,
            )
            // …and what this FAMILY allows is mirrored above, through the
            // family's entry: those switches are the current family's.
            // The second apply of the same complete state-set. Idempotent
            // and last-writer-wins, which is why the fixed resync order
            // (/me, then /families/mine) needs no coordination — and why
            // this runs harmlessly on the many foreground refreshes that
            // also call this method.
            settings.setBlockedUserIds(result.value.blockedUserIds)
        }
        return result
    }

    /**
     * `POST /families` — the first family from the gate, or ANOTHER one
     * ("New Family…", docs/protocol.md, "Several families"). Either way the
     * caller owns it, it goes on the list and it is shown. The assistant and
     * the roster come from `GET /families/{id}`, which [adopt] reads —
     * `POST /families` answers with the family alone.
     */
    suspend fun create(name: String): ApiResult<FamilyResponse> {
        val result = familyApi.create(name)
        if (result is ApiResult.Ok) {
            if (state().families.isEmpty()) {
                settings.setFamilyStatus(FamilyStatus.OWNER)
                settings.setFamilyName(result.value.family.name)
            }
            adopt(result.value.family, role = FamilyList.OWNER)
        }
        return result
    }

    /**
     * `POST /families/join` — from the gate, or "Join Another Family…".
     * `joined`: `/me` names the new family, which is then shown. `pending`:
     * with no family that is the waiting screen; with one, a pending row in
     * the switcher, which `/me` brings.
     */
    suspend fun join(inviteCode: String): ApiResult<JoinResponse> {
        val before = state().families.mapTo(HashSet()) { it.id }
        val result = familyApi.join(inviteCode)
        if (result is ApiResult.Ok) {
            when (result.value.status) {
                // Open policy — membership immediate; refreshMe picks up
                // family + role and flips the status.
                "joined" -> {
                    sessionRepository.refreshMe()
                    // The answer names the family; an older server's does
                    // not, and the new entry on `/me` is the one joined.
                    val families = state().families
                    val added = result.value.familyId?.let { id -> families.firstOrNull { it.id == id } }
                        ?: families.firstOrNull { it.id !in before }
                    if (added != null && before.isNotEmpty()) {
                        sessionRepository.switchFamily(added.id)
                    }
                    added?.let {
                        refreshFamily(it.id)
                        _familiesAdded.tryEmit(it.id)
                    }
                }
                else -> if (before.isEmpty()) {
                    settings.setFamilyStatus(FamilyStatus.PENDING)
                } else {
                    sessionRepository.refreshMe()
                }
            }
        }
        return result
    }

    suspend fun rotateInviteCode(): ApiResult<RotateInviteCodeResponse> =
        familyApi.rotateInviteCode(currentPath()).resyncIfGone()

    suspend fun setJoinPolicy(policy: String): ApiResult<FamilyResponse> =
        familyApi.setJoinPolicy(currentPath(), policy).resyncIfGone().alsoMirror()

    /** Owner-only. A null tag CLEARS the language back to unset. */
    suspend fun setLanguage(tag: String?): ApiResult<FamilyResponse> =
        familyApi.setLanguage(currentPath(), tag).resyncIfGone().alsoMirror()

    /**
     * Owner-only. Mirrored onto the settings for the strip's sake: with
     * history off there is no transcript, so the third switch is inert
     * and the family composer must stop announcing recent photos.
     */
    suspend fun setAiHistory(enabled: Boolean): ApiResult<FamilyResponse> =
        familyApi.setAiHistory(currentPath(), enabled).resyncIfGone().alsoMirror()

    /**
     * Owner-only: the family's half of the picture rule.
     *
     * Mirrored straight onto the account-scoped settings, exactly as
     * [refreshMine] does — the composer reads it from there to decide
     * whether to offer a picture at all, and without this mirror the
     * owner's own device would go on hiding the affordance until the
     * next `GET /families/mine`.
     */
    suspend fun setAiVision(enabled: Boolean): ApiResult<FamilyResponse> =
        // Turning `ai_vision` OFF turns the third switch off in the same
        // write on the server, asked or not — so the answer is the truth for
        // both, and the whole family is mirrored.
        familyApi.setAiVision(currentPath(), enabled).resyncIfGone().alsoMirror()

    /**
     * Owner-only: the third switch (docs/protocol.md, "Recent photos from
     * the family chat"). Mirrored for [setAiVision]'s reason — the family
     * composer reads it to say that a mention may carry the chat's recent
     * photos, and the owner's own device must say so at once.
     */
    suspend fun setAiHistoryPhotos(enabled: Boolean): ApiResult<FamilyResponse> =
        familyApi.setAiHistoryPhotos(currentPath(), enabled).resyncIfGone().alsoMirror()

    /**
     * Owner-only: the fourth switch (docs/protocol.md, "The daily
     * greeting"). Mirrored like the three above so the owner's own device
     * agrees with the server at once — no frame will tell it what it just
     * did itself.
     */
    suspend fun setAiGreeting(enabled: Boolean): ApiResult<FamilyResponse> =
        familyApi.setAiGreeting(currentPath(), enabled).resyncIfGone().alsoMirror()

    /**
     * Owner-only: the fifth switch (docs/protocol.md, "Profile pictures of
     * members"). Nothing is mirrored into settings: no composer strip reads
     * it — the protocol asks for the switch and its sentence, nothing more.
     */
    suspend fun setAiFaces(enabled: Boolean): ApiResult<FamilyResponse> =
        familyApi.setAiFaces(currentPath(), enabled).resyncIfGone().alsoMirror()

    /**
     * Owner-only: the transcripts switch (docs/protocol.md, "Transcripts on
     * request"). Mirrored like the others so the owner's own bubbles agree
     * with the server at once — no frame will tell this device what it
     * just did itself.
     */
    suspend fun setAiTranscripts(enabled: Boolean): ApiResult<FamilyResponse> =
        familyApi.setAiTranscripts(currentPath(), enabled).resyncIfGone().alsoMirror()

    /**
     * Owner-only: the lookups switch (docs/protocol.md, "Looking things
     * up"). Mirrored like the others so the owner's own device agrees with
     * the server at once.
     */
    /**
     * Owner-only: the greeting's weather places (docs/protocol.md, "Today's
     * weather, for places the owner chose"). Nothing is mirrored into
     * settings: only the owner's own screen reads the list, and it reads it
     * from this answer.
     */
    suspend fun setGreetingPlaces(places: List<String>): ApiResult<FamilyResponse> =
        familyApi.setGreetingPlaces(currentPath(), places).resyncIfGone().alsoMirror()

    suspend fun setAiLookups(enabled: Boolean): ApiResult<FamilyResponse> =
        familyApi.setAiLookups(currentPath(), enabled).resyncIfGone().alsoMirror()

    /**
     * My own birthday, mirrored onto my roster row.
     *
     * The mirror is the point: the roster is what every screen renders
     * from, and no frame will tell this device what it just did itself
     * (protocol.md, "Birthdays"). The value written back is the SERVER's,
     * not the one sent — the two agree today, and the response is the
     * authority if they ever stop.
     */
    suspend fun setMyBirthday(month: Int, day: Int): ApiResult<BirthdayDto?> {
        val result = authApi.setMyBirthday(month, day)
        return when (result) {
            is ApiResult.Ok -> {
                val birthday = result.value.user.birthday
                memberDao.setBirthday(result.value.user.id, birthday?.month, birthday?.day)
                ApiResult.Ok(birthday)
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }
    }

    /** The 204 carries no user, so the id to clear comes from settings. */
    suspend fun clearMyBirthday(): ApiResult<Unit> {
        val result = authApi.clearMyBirthday()
        if (result is ApiResult.Ok) {
            settings.state.first().myUserId?.let { memberDao.setBirthday(it, null, null) }
        }
        return result
    }

    /** Owner-only, and the owner may name themselves — see FamilyApi. */
    suspend fun setMemberBirthday(userId: Long, month: Int, day: Int): ApiResult<Unit> {
        val result = familyApi.setMemberBirthday(currentPath(), userId, month, day).resyncIfGone()
        return when (result) {
            is ApiResult.Ok -> {
                val birthday = result.value.member.birthday
                memberDao.setBirthday(userId, birthday?.month, birthday?.day)
                ApiResult.Ok(Unit)
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }
    }

    suspend fun clearMemberBirthday(userId: Long): ApiResult<Unit> {
        val result = familyApi.clearMemberBirthday(currentPath(), userId).resyncIfGone()
        if (result is ApiResult.Ok) memberDao.setBirthday(userId, null, null)
        return result
    }

    suspend fun joinRequests(): ApiResult<JoinRequestsResponse> =
        familyApi.joinRequests(currentPath()).resyncIfGone()

    suspend fun approve(requestId: Long): ApiResult<Unit> {
        val familyId = currentFamily()
        val result = familyApi.approve(state().pathFamily(familyId), requestId).resyncIfGone()
        return when (result) {
            is ApiResult.Ok -> {
                memberDao.upsertAll(
                    listOf(
                        MemberEntity(
                            familyId = familyId,
                            userId = result.value.member.id,
                            username = result.value.member.username,
                            displayName = result.value.member.displayName,
                            // An approved join request always names a
                            // live member; the fallback only exists
                            // because the field is optional on the wire.
                            role = result.value.member.role ?: "member",
                            avatarVersion = result.value.member.avatarVersion,
                            birthdayMonth = result.value.member.birthday?.month,
                            birthdayDay = result.value.member.birthday?.day,
                            // The answer is the OWNER's, so it says whether
                            // they may reset the newcomer's password — false
                            // when the member is also in a family this owner
                            // does not own (docs/protocol.md, "Several
                            // families"). Kept, or "Reset Password" would be
                            // offered until the next roster read.
                            passwordReset = result.value.member.passwordReset,
                        ),
                    ),
                )
                ApiResult.Ok(Unit)
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }
    }

    suspend fun reject(requestId: Long): ApiResult<Unit> =
        familyApi.reject(currentPath(), requestId).resyncIfGone()

    /**
     * Leave the family, and answer with the NAME of whoever inherited it —
     * or null when nobody did.
     *
     * The lookup happens BEFORE the local teardown, and that ordering is
     * the whole reason the resolution lives here rather than in the
     * caller: `onRemovedFromFamily` clears the roster the id has to be
     * looked up in, so resolving afterwards would always name nobody.
     * The protocol states it in the same order — the leaving owner
     * "resolves `new_owner_user_id` against the roster it still holds,
     * tells the user who inherited, and only then tears its family state
     * down" (docs/protocol.md, `POST /families/leave`).
     *
     * An owner is never refused; `owner_cannot_leave` is retired.
     *
     * Since several families it is the CURRENT family that is left, and only
     * it (`POST /families/{id}/leave`): the first remaining family is shown
     * afterwards, or the gate when none remain.
     */
    suspend fun leave(): ApiResult<String?> {
        val familyId = currentFamily()
        return when (val result = familyApi.leave(state().pathFamily(familyId)).resyncIfGone()) {
            is ApiResult.Ok -> {
                val name = result.value?.let { memberDao.displayName(it) }
                // History is retained server-side and resurfaces on rejoin
                // (protocol) — but locally these chats aren't ours to show.
                sessionRepository.leftFamily(familyId)
                ApiResult.Ok(name)
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }
    }

    /**
     * Owner-only password reset. Nothing local changes: the member stays in
     * the roster and keeps their history — only their sessions die, and
     * that happens on the server.
     */
    suspend fun resetMemberPassword(userId: Long, newPassword: String): ApiResult<Unit> =
        familyApi.resetMemberPassword(currentPath(), userId, newPassword).resyncIfGone()

    /** Out of the CURRENT family — their other families are not this owner's. */
    suspend fun removeMember(userId: Long): ApiResult<Unit> {
        val familyId = currentFamily()
        val result = familyApi.removeMember(state().pathFamily(familyId), userId).resyncIfGone()
        if (result is ApiResult.Ok) memberDao.markLeft(familyId, userId)
        return result
    }

    companion object {
        const val FAMILY_NOT_FOUND = "family_not_found"
        const val INVITATION_NOT_PENDING = "invitation_not_pending"
    }
}

/**
 * The CURRENT family's live roster, following the switcher: every screen
 * that offers something about a person reads the family it is showing.
 */
@OptIn(ExperimentalCoroutinesApi::class)
fun MemberDao.observeCurrentActiveMembers(settings: SettingsRepository): Flow<List<MemberEntity>> =
    settings.state
        .map { it.familyKey }
        .distinctUntilChanged()
        .flatMapLatest { observeActiveMembers(it) }
