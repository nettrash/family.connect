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
     * last year still has to say who wrote it, and so does one from
     * somebody whose account no longer exists at all. Screens that offer
     * an ACTION on a member — the picker, the admin list — want
     * [observeActiveMembers] instead.
     *
     * A DELETED row is in here, and its `displayName` is the server's
     * English placeholder: a reader draws its own translation off
     * [MemberEntity.deleted] rather than that text.
     */
    @Query(
        """
        SELECT * FROM members
        ORDER BY (role = 'owner') DESC, displayName COLLATE NOCASE ASC
        """,
    )
    fun observeMembers(): Flow<List<MemberEntity>>

    /**
     * Only members still in the family — and still in EXISTENCE.
     *
     * Both flags, not just `hasLeft`: a deleted account is never a member
     * (docs/protocol.md, "Deleting an account"), and the two facts arrive
     * from different places — `former_members` on a roster refresh, a
     * `member_deleted` frame on the wire — so a roster that filtered on
     * one of them would show a tombstone whenever the other had not
     * landed yet. This is the query every screen that OFFERS something
     * about a person reads: the member picker, the admin list, the
     * remove-member list, the birthday admin.
     */
    @Query(
        """
        SELECT * FROM members WHERE hasLeft = 0 AND deleted = 0
        ORDER BY (role = 'owner') DESC, displayName COLLATE NOCASE ASC
        """,
    )
    fun observeActiveMembers(): Flow<List<MemberEntity>>

    @Upsert
    suspend fun upsertAll(members: List<MemberEntity>)

    /** Everybody this device holds a tombstone for. See [upsertRoster]. */
    @Query("SELECT userId FROM members WHERE deleted = 1")
    suspend fun deletedMemberIds(): List<Long>

    /**
     * `GET /families/mine` applied whole: the live roster, plus the
     * `former_members` tombstones beside it.
     *
     * A live roster entry NEVER overwrites a stored tombstone. Deletion is
     * one-way and ids are never reused (docs/protocol.md, "Deleting an
     * account"), so a response naming a deleted account under `members`
     * can only be one the server computed before the deletion landed — and
     * a request in flight while a `member_deleted` frame arrives is
     * exactly that. Applying it would put the person's real name, real
     * picture and `deleted = 0` back on a row whose whole job is that they
     * are gone, and [observeActiveMembers] would offer them in the new-chat
     * picker, in the admin list and in every poll's "3 of 5 voted"
     * denominator until some later refresh happened to list them under
     * `former_members`. iOS carries the same guard in
     * ChatSyncCoordinator.upsertMember.
     *
     * The whole-row [upsertAll] is what makes the guard necessary: the
     * entity built from `members` leaves `deleted` and `hasLeft` at their
     * defaults, so the write is a resurrection rather than an update.
     */
    @Transaction
    suspend fun upsertRoster(roster: List<MemberEntity>, tombstones: List<MemberEntity>) {
        val tombstoned = deletedMemberIds().toSet()
        upsertAll(roster.filterNot { it.userId in tombstoned } + tombstones)
    }

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
     * They deleted their ACCOUNT: write the tombstone deliberately.
     *
     * This is the one write in the app whose job is to WIPE stored
     * fields, and that is why it is spelled out here rather than sent
     * through [upsertAll] or [upsertLeavingBirthday]. Everywhere else an
     * absent field must never clear a stored one — a `member_joined`
     * frame carrying no birthday must leave the birthday alone. Here the
     * server is not staying silent about the picture and the birthday: it
     * is telling us they are gone (docs/protocol.md, "Deleting an
     * account"), and going through the ordinary upsert would make the
     * distinction impossible to see at the call site.
     *
     * `hasLeft` is set alongside `deleted`: a `member_left` frame is sent
     * with the `member_deleted` one, but frames are best-effort and the
     * roster must not depend on both arriving. The role goes back to
     * "member" because a tombstone holds none — an ex-owner who deleted
     * their account must not still sort and read as the owner while
     * [setOwner] hands the family to somebody else.
     *
     * Insert-if-absent first: this device may never have seen them (they
     * joined and deleted between two of its resyncs) and their messages
     * still need a name.
     */
    @Transaction
    suspend fun writeTombstone(
        userId: Long,
        username: String,
        displayName: String,
    ) {
        insertIfAbsent(
            MemberEntity(
                userId = userId,
                username = username,
                displayName = displayName,
                role = "member",
                avatarVersion = 0,
                hasLeft = true,
                deleted = true,
            ),
        )
        markDeleted(userId = userId, username = username, displayName = displayName)
    }

    /**
     * Every column a tombstone means, and not one more. The two birthday
     * columns ARE in the SET list, unlike [updateLeavingBirthday]'s — see
     * [writeTombstone] for why that is the difference between the two.
     */
    @Query(
        """
        UPDATE members
        SET username = :username, displayName = :displayName, role = 'member',
            avatarVersion = 0, hasLeft = 1, deleted = 1,
            birthdayMonth = NULL, birthdayDay = NULL
        WHERE userId = :userId
        """,
    )
    suspend fun markDeleted(userId: Long, username: String, displayName: String)

    /**
     * The family has a new owner (docs/protocol.md, "Deleting an
     * account": ownership passes to the longest-standing member when an
     * owner deletes their account).
     *
     * Two statements rather than one so the table can never hold two
     * owners: demote whoever held it, then promote the named member. A
     * promote that matches no row is a no-op — this device may not have
     * that member yet, and the next roster refresh puts it right.
     */
    @Transaction
    suspend fun setOwner(userId: Long) {
        demoteOtherOwners(userId)
        promoteToOwner(userId)
    }

    @Query("UPDATE members SET role = 'member' WHERE role = 'owner' AND userId != :userId")
    suspend fun demoteOtherOwners(userId: Long)

    @Query("UPDATE members SET role = 'owner' WHERE userId = :userId")
    suspend fun promoteToOwner(userId: Long)

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
