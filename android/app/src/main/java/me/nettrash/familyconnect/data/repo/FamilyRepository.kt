/*
 * FamilyRepository.kt
 * Family Connect (Android)
 *
 * Family membership: roster (Room `members` table), invite code, join
 * policy, join requests, leave/remove. Also the sink for the
 * member_joined / member_left WebSocket frames — applied to the roster
 * live, and a member_left carrying *my* user id escalates to
 * SessionRepository (we've been kicked).
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
import me.nettrash.familyconnect.data.net.FamilyApi
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

@Singleton
class FamilyRepository @Inject constructor(
    private val familyApi: FamilyApi,
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
                    is ServerFrame.MemberJoined -> memberDao.upsertAll(
                        listOf(
                            MemberEntity(
                                userId = frame.user.id,
                                username = frame.user.username,
                                displayName = frame.user.displayName,
                                // Joiners are always plain members; the
                                // owner existed before the frame could.
                                role = "member",
                                avatarVersion = frame.user.avatarVersion,
                            ),
                        ),
                    )
                    is ServerFrame.MemberLeft -> {
                        memberDao.markLeft(frame.userId)
                        val myId = settings.state.first().myUserId
                        if (frame.userId == myId) sessionRepository.onRemovedFromFamily()
                    }
                    else -> Unit
                }
            }
        }
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
                    role = it.role,
                    avatarVersion = it.avatarVersion,
                )
            }
            memberDao.upsertAll(roster)
            if (roster.isNotEmpty()) {
                // Guarded: `NOT IN ()` is a syntax error in SQLite, and an
                // empty roster cannot happen anyway — the caller is in the
                // family they just read.
                memberDao.markLeftExcept(roster.map { it.userId })
            }
            settings.setFamilyName(result.value.family.name)
        }
        return result
    }

    suspend fun create(name: String): ApiResult<FamilyResponse> {
        val result = familyApi.create(name)
        if (result is ApiResult.Ok) {
            settings.setFamilyStatus(FamilyStatus.OWNER)
            settings.setFamilyName(result.value.family.name)
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
                            role = result.value.member.role,
                            avatarVersion = result.value.member.avatarVersion,
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

    suspend fun leave(): ApiResult<Unit> {
        val result = familyApi.leave()
        if (result is ApiResult.Ok) {
            // History is retained server-side and resurfaces on rejoin
            // (protocol) — but locally these chats aren't ours to show.
            sessionRepository.onRemovedFromFamily()
        }
        return result
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
