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
 * iOS counterpart: ios/FamilyConnect/Data/Repo/FamilyRepository.swift
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AuthApi
import me.nettrash.familyconnect.data.net.FamilyApi
import me.nettrash.familyconnect.data.net.dto.BirthdayDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.net.dto.JoinRequestsResponse
import me.nettrash.familyconnect.data.net.dto.JoinResponse
import me.nettrash.familyconnect.data.net.dto.RotateInviteCodeResponse
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import javax.inject.Inject
import javax.inject.Singleton
import me.nettrash.familyconnect.data.net.dto.ReportResponse

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
    @AppScope scope: CoroutineScope,
) {

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
                    is ServerFrame.MemberJoined -> memberDao.upsertLeavingBirthday(
                        MemberEntity(
                            userId = frame.user.id,
                            username = frame.user.username,
                            displayName = frame.user.displayName,
                            // Joiners are always plain members; the
                            // owner existed before the frame could.
                            role = "member",
                            avatarVersion = frame.user.avatarVersion,
                        ),
                    )
                    is ServerFrame.MemberLeft -> {
                        memberDao.markLeft(frame.userId)
                        val myId = settings.state.first().myUserId
                        if (frame.userId == myId) sessionRepository.onRemovedFromFamily()
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
                        userId = frame.member.id,
                        username = frame.member.username,
                        displayName = frame.member.displayName,
                    )
                    // Ownership moved because the owner deleted their
                    // account. The roster row moves with it, and if the
                    // new owner is US the stored status flips now rather
                    // than at the next GET /me — that is the whole point
                    // of the frame (protocol.md, "Server → client").
                    is ServerFrame.FamilyOwner -> {
                        memberDao.setOwner(frame.userId)
                        val state = settings.state.first()
                        if (state.familyStatus == FamilyStatus.MEMBER ||
                            state.familyStatus == FamilyStatus.OWNER
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
    suspend fun report(
        reportedUserId: Long,
        reason: String,
        messageId: Long?,
    ): ApiResult<ReportResponse> = familyApi.report(reportedUserId, reason, messageId)

    suspend fun unblock(userId: Long): ApiResult<Unit> =
        familyApi.unblockMember(userId).also {
            if (it is ApiResult.Ok) applyBlockLocally(userId, blocked = false)
        }

    /** Everyone ever seen — the name-resolution feed. */
    fun observeMembers(): Flow<List<MemberEntity>> = memberDao.observeMembers()

    /** Only those still in the family — for pickers and the admin list. */
    fun observeActiveMembers(): Flow<List<MemberEntity>> = memberDao.observeActiveMembers()

    /** GET /families/mine → roster upsert. invite_code present for owners only. */
    suspend fun refreshMine(): ApiResult<FamilyMineResponse> {
        val result = familyApi.mine()
        if (result is ApiResult.Ok) {
            // Upsert, then flag whoever the server no longer lists.
            //
            // NOT delete-then-insert: that dropped departed members
            // entirely, and every message they had ever sent lost its
            // name and face. The upsert also clears `hasLeft` for anyone
            // who rejoined, since the row is replaced wholesale.
            val roster = result.value.members.map {
                MemberEntity(
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
                memberDao.markLeftExcept(roster.map { it.userId })
            }
            settings.setFamilyName(result.value.family.name)
            // The assistant is NOT upserted as a member — it belongs to no
            // family, so it appears in no roster. Kept aside purely so the
            // family chat can put a name on its messages and the composer
            // knows whether to offer `@ai`.
            settings.setAssistant(
                result.value.assistant?.userId,
                result.value.assistant?.displayName,
            )
            // The second apply of the same complete state-set. Idempotent
            // and last-writer-wins, which is why the fixed resync order
            // (/me, then /families/mine) needs no coordination — and why
            // this runs harmlessly on the many foreground refreshes that
            // also call this method.
            settings.setBlockedUserIds(result.value.blockedUserIds)
        }
        return result
    }

    suspend fun create(name: String): ApiResult<FamilyResponse> {
        val result = familyApi.create(name)
        if (result is ApiResult.Ok) {
            settings.setFamilyStatus(FamilyStatus.OWNER)
            settings.setFamilyName(result.value.family.name)
            // The assistant comes from `GET /families/mine`, which the call
            // below makes anyway — `POST /families` answers with the family
            // alone.
            refreshMine()
        }
        return result
    }

    suspend fun join(inviteCode: String): ApiResult<JoinResponse> {
        val result = familyApi.join(inviteCode)
        if (result is ApiResult.Ok) {
            when (result.value.status) {
                // Open policy — membership immediate; refreshMe picks up
                // family + role and flips the status.
                "joined" -> sessionRepository.refreshMe()
                else -> settings.setFamilyStatus(FamilyStatus.PENDING)
            }
        }
        return result
    }

    suspend fun rotateInviteCode(): ApiResult<RotateInviteCodeResponse> =
        familyApi.rotateInviteCode()

    suspend fun setJoinPolicy(policy: String): ApiResult<FamilyResponse> =
        familyApi.setJoinPolicy(policy)

    /** Owner-only. A null tag CLEARS the language back to unset. */
    suspend fun setLanguage(tag: String?): ApiResult<FamilyResponse> =
        familyApi.setLanguage(tag)

    /** Owner-only. */
    suspend fun setAiHistory(enabled: Boolean): ApiResult<FamilyResponse> =
        familyApi.setAiHistory(enabled)

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
        val result = familyApi.setMemberBirthday(userId, month, day)
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
        val result = familyApi.clearMemberBirthday(userId)
        if (result is ApiResult.Ok) memberDao.setBirthday(userId, null, null)
        return result
    }

    suspend fun joinRequests(): ApiResult<JoinRequestsResponse> =
        familyApi.joinRequests()

    suspend fun approve(requestId: Long): ApiResult<Unit> {
        val result = familyApi.approve(requestId)
        return when (result) {
            is ApiResult.Ok -> {
                memberDao.upsertAll(
                    listOf(
                        MemberEntity(
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
        familyApi.reject(requestId)

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
     */
    suspend fun leave(): ApiResult<String?> {
        return when (val result = familyApi.leave()) {
            is ApiResult.Ok -> {
                val name = result.value?.let { memberDao.displayName(it) }
                // History is retained server-side and resurfaces on rejoin
                // (protocol) — but locally these chats aren't ours to show.
                sessionRepository.onRemovedFromFamily()
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
        familyApi.resetMemberPassword(userId, newPassword)

    suspend fun removeMember(userId: Long): ApiResult<Unit> {
        val result = familyApi.removeMember(userId)
        if (result is ApiResult.Ok) memberDao.markLeft(userId)
        return result
    }
}
