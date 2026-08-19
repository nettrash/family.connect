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

    @Query(
        """
        SELECT * FROM members
        ORDER BY (role = 'owner') DESC, displayName COLLATE NOCASE ASC
        """,
    )
    fun observeMembers(): Flow<List<MemberEntity>>

    @Upsert
    suspend fun upsertAll(members: List<MemberEntity>)

    @Query("DELETE FROM members WHERE userId = :userId")
    suspend fun delete(userId: Long)

    @Query("DELETE FROM members")
    suspend fun deleteAll()
}
