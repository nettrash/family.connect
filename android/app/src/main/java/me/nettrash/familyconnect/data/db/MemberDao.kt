/*
 * MemberDao.kt
 * Family Connect (Android)
 *
 * Family roster — drives sender names in the family chat, the new-chat
 * member picker, and the admin member list. Owner sorts first, then
 * alphabetical.
 *
 * One row per (family, member) since several families (docs/protocol.md,
 * "Several families"). What is a FAMILY's — the role, whether they left —
 * is written to that family's row; what is the ACCOUNT's — the name, the
 * picture, the birthday, a deletion — to every row of theirs at once, so
 * the copies never disagree about who somebody is.
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
        WHERE rowid = (
            SELECT m2.rowid FROM members m2 WHERE m2.userId = members.userId
            ORDER BY m2.hasLeft ASC, m2.familyId ASC LIMIT 1
        )
        ORDER BY (role = 'owner') DESC, displayName COLLATE NOCASE ASC
        """,
    )
    // ONE row per person, whichever families they are in: the account's
    // facts are the same on every copy, and a person still in some family
    // is read from a family they are still in.
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
        SELECT * FROM members WHERE familyId = :familyId AND hasLeft = 0 AND deleted = 0
        ORDER BY (role = 'owner') DESC, displayName COLLATE NOCASE ASC
        """,
    )
    fun observeActiveMembers(familyId: Long): Flow<List<MemberEntity>>

    /**
     * Every live membership in every family this device holds — what the
     * owner's "Invite…" picker is drawn from: the members of their OTHER
     * families who are not in this one (docs/protocol.md, "Several
     * families").
     */
    @Query("SELECT * FROM members WHERE hasLeft = 0 AND deleted = 0")
    fun observeAllActiveMemberships(): Flow<List<MemberEntity>>

    /**
     * The same roster, read once — for resolving the members a board note's
     * text names (docs/protocol.md, "Board"), which is a question asked at
     * the moment of a write rather than watched.
     */
    @Query(
        """
        SELECT * FROM members WHERE familyId = :familyId AND hasLeft = 0 AND deleted = 0
        ORDER BY displayName COLLATE NOCASE ASC
        """,
    )
    suspend fun activeMembers(familyId: Long): List<MemberEntity>

    /**
     * One member's display name, or null when this device holds no row for
     * them. Tombstones included on purpose — a departing owner's successor
     * is by definition still here, but a name is better than nothing if
     * the roster is mid-resync.
     */
    @Query("SELECT displayName FROM members WHERE userId = :userId LIMIT 1")
    suspend fun displayName(userId: Long): String?

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
     * things here. The role is the other column the frame does not really
     * carry: [MemberEntity.role] lands on a NEW row, and on one coming back
     * from having left, but never over a live row's — see
     * [updateLeavingBirthday].
     *
     * `passwordReset` follows the same rule by a different road: the frame
     * carries it on the family OWNER's copy alone (docs/protocol.md, "Several
     * families"), so it is written when [MemberEntity.passwordReset] is
     * non-null and a null leaves the stored answer where it is — a
     * re-approved member's known "false" must not read as "offer the reset"
     * because some copy of a frame did not say.
     */
    @Transaction
    suspend fun upsertLeavingBirthday(member: MemberEntity) {
        if (insertIfAbsent(member) == -1L) {
            updateLeavingBirthday(
                familyId = member.familyId,
                userId = member.userId,
                role = member.role,
                passwordReset = member.passwordReset,
            )
        }
        // The account's half on every row of theirs, the new one included.
        updateAccount(
            userId = member.userId,
            username = member.username,
            displayName = member.displayName,
            avatarVersion = member.avatarVersion,
        )
    }

    /** -1 when a row for this member is already there. See [upsertLeavingBirthday]. */
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun insertIfAbsent(member: MemberEntity): Long

    /**
     * The FAMILY's columns a `member_joined` frame carries: the role, that
     * they are back, and — only when the frame said it — whether the owner
     * may reset their password (COALESCE: a null keeps what is stored).
     *
     * The role is written only over a row that had LEFT — a rejoin, where
     * the frame's role is the news. A row still live is somebody this device
     * already holds in the family, and the frame cannot be telling it a role
     * change: `member_joined` carries no role at all, and the one the caller
     * passes is a default. The case that made this matter is a family
     * created on THIS device (docs/protocol.md, "Several families"):
     * `create` reads the new roster with the creator as owner, and the
     * server's `member_joined` naming the creator arrives after it — written
     * through, it demoted the owner of the family they had just made to
     * "member" (no crown, sorted among the rest, every owner check on the
     * roster wrong) until the next roster read. Ownership moves only by
     * `family_owner` ([setOwner]) and by a roster read.
     */
    @Query(
        """
        UPDATE members
        SET role = CASE WHEN hasLeft = 1 THEN :role ELSE role END,
            hasLeft = 0,
            passwordReset = COALESCE(:passwordReset, passwordReset)
        WHERE familyId = :familyId AND userId = :userId
        """,
    )
    suspend fun updateLeavingBirthday(familyId: Long, userId: Long, role: String, passwordReset: Boolean?)

    /** The ACCOUNT's columns a `member_joined` frame carries, on every row of theirs. */
    @Query(
        """
        UPDATE members SET username = :username, displayName = :displayName, avatarVersion = :avatarVersion
        WHERE userId = :userId
        """,
    )
    suspend fun updateAccount(userId: Long, username: String, displayName: String, avatarVersion: Long)

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
        familyId: Long,
        userId: Long,
        username: String,
        displayName: String,
    ) {
        insertIfAbsent(
            MemberEntity(
                familyId = familyId,
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
    suspend fun setOwner(familyId: Long, userId: Long) {
        demoteOtherOwners(familyId, userId)
        promoteToOwner(familyId, userId)
    }

    @Query("UPDATE members SET role = 'member' WHERE familyId = :familyId AND role = 'owner' AND userId != :userId")
    suspend fun demoteOtherOwners(familyId: Long, userId: Long)

    @Query("UPDATE members SET role = 'owner' WHERE familyId = :familyId AND userId = :userId")
    suspend fun promoteToOwner(familyId: Long, userId: Long)

    /**
     * They left THAT family; the row stays so their old messages keep a
     * name and a face. Deleting it is what orphaned them.
     */
    @Query("UPDATE members SET hasLeft = 1 WHERE familyId = :familyId AND userId = :userId")
    suspend fun markLeft(familyId: Long, userId: Long)

    /**
     * Everyone the server no longer lists in this family has left it.
     * Called after a roster upsert, which is what replaces the old
     * delete-then-insert: that threw away departed members entirely.
     */
    @Query("UPDATE members SET hasLeft = 1 WHERE familyId = :familyId AND userId NOT IN (:presentUserIds)")
    suspend fun markLeftExcept(familyId: Long, presentUserIds: List<Long>)

    /**
     * This account is out of [familyId]: its roster goes (docs/protocol.md,
     * "Several families" — "drops the family's chats, board, pack, roster").
     * A person also in another family keeps that family's row, and with it
     * their name.
     */
    @Query("DELETE FROM members WHERE familyId = :familyId")
    suspend fun deleteFamily(familyId: Long)

    /**
     * Rows cached before several families (family 0) are the HOME family's —
     * the one family an app from before ever saw. A row the home family
     * already holds wins; the leftover copy goes.
     */
    @Transaction
    suspend fun claimLegacy(homeFamilyId: Long) {
        adoptLegacy(homeFamilyId)
        dropLegacy()
    }

    @Query("UPDATE OR IGNORE members SET familyId = :homeFamilyId WHERE familyId = 0")
    suspend fun adoptLegacy(homeFamilyId: Long)

    @Query("DELETE FROM members WHERE familyId = 0")
    suspend fun dropLegacy()

    /** Session teardown only — logout wipes local data outright. */
    @Query("DELETE FROM members")
    suspend fun deleteAll()
}
