/*
 * MemberDao.kt
 * Family Connect (Android)
 *
 * Family roster — drives sender names in the family chat, the new-chat
 * member picker, and the admin member list. Owner sorts first, then
 * alphabetical.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Db/MemberStore.swift
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.Transaction
import androidx.room.Upsert
import kotlinx.coroutines.flow.Flow

@Dao
interface MemberDao {

    /**
     * EVERY member ever seen, including those who have left.
     *
     * This is the name-resolution feed: a bubble from somebody who left
     * last year still has to say who wrote it. Screens that offer an
     * ACTION on a member — the picker, the admin list — want
     * [observeActiveMembers] instead.
     */
    @Query(
        """
        SELECT * FROM members
        ORDER BY (role = 'owner') DESC, displayName COLLATE NOCASE ASC
        """,
    )
    fun observeMembers(): Flow<List<MemberEntity>>

    /** Only members still in the family. */
    @Query(
        """
        SELECT * FROM members WHERE hasLeft = 0
        ORDER BY (role = 'owner') DESC, displayName COLLATE NOCASE ASC
        """,
    )
    fun observeActiveMembers(): Flow<List<MemberEntity>>

    @Upsert
    suspend fun upsertAll(members: List<MemberEntity>)

    /**
     * Upsert one member from a frame that says NOTHING about a birthday,
     * leaving whatever is stored exactly where it is.
     *
     * `member_joined.user` is a `UserBrief` (docs/protocol.md,
     * "Server → client") — id, username, display_name, avatar_version and
     * no birthday field at all — so the decoded DTO carries null on every
     * one of these frames, whoever it is about. Through [upsertAll] that
     * null is WRITTEN, and a member who leaves and is re-approved loses
     * their birthday on every other device until the next roster refresh
     * happens to put it back.
     *
     * So: an absent field never wipes a stored one. Insert-if-new, then
     * update only the columns the frame actually carries. iOS spells the
     * same rule with a double optional (`.none` = never told,
     * `.some(nil)` = told there is none) in ChatSyncCoordinator; this is
     * the shape the same rule takes against a table.
     *
     * `hasLeft` still clears — the frame is precisely the news that they
     * are back — and the birthday columns are not in the SET list at all,
     * which is what makes "not told" and "told it is empty" different
     * things here.
     */
    @Transaction
    suspend fun upsertLeavingBirthday(member: MemberEntity) {
        if (insertIfAbsent(member) == -1L) {
            updateLeavingBirthday(
                userId = member.userId,
                username = member.username,
                displayName = member.displayName,
                role = member.role,
                avatarVersion = member.avatarVersion,
            )
        }
    }

    /** -1 when a row for this member is already there. See [upsertLeavingBirthday]. */
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun insertIfAbsent(member: MemberEntity): Long

    /** Every column a `member_joined` frame carries, and not one more. */
    @Query(
        """
        UPDATE members
        SET username = :username, displayName = :displayName,
            role = :role, avatarVersion = :avatarVersion, hasLeft = 0
        WHERE userId = :userId
        """,
    )
    suspend fun updateLeavingBirthday(
        userId: Long,
        username: String,
        displayName: String,
        role: String,
        avatarVersion: Long,
    )

    /**
     * Write (or clear, with two nulls) one member's birthday.
     *
     * A targeted UPDATE rather than an upsert of the whole row: the two
     * writers are a PUT that answers with the member and a DELETE that
     * answers with nothing, and the DELETE has no row to upsert. Going
     * through one statement also means neither path can quietly reset
     * `hasLeft` or `avatarVersion` from a shape assembled at the call site.
     *
     * This is the only way the value lands locally at all — a birthday
     * change raises no WebSocket frame and no push (docs/protocol.md,
     * "Birthdays"), so the writing device has its own response and
     * everyone else waits for the next roster refresh.
     */
    @Query("UPDATE members SET birthdayMonth = :month, birthdayDay = :day WHERE userId = :userId")
    suspend fun setBirthday(userId: Long, month: Int?, day: Int?)

    /**
     * They left; the row stays so their old messages keep a name and a
     * face. Deleting it is what orphaned them.
     */
    @Query("UPDATE members SET hasLeft = 1 WHERE userId = :userId")
    suspend fun markLeft(userId: Long)

    /**
     * Everyone the server no longer lists has left. Called after a roster
     * upsert, which is what replaces the old delete-then-insert: that
     * threw away departed members entirely.
     */
    @Query("UPDATE members SET hasLeft = 1 WHERE userId NOT IN (:presentUserIds)")
    suspend fun markLeftExcept(presentUserIds: List<Long>)

    /** Session teardown only — logout wipes local data outright. */
    @Query("DELETE FROM members")
    suspend fun deleteAll()
}
