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
import androidx.room.Query
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
