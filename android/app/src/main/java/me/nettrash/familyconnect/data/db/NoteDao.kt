/*
 * NoteDao.kt
 * Family Connect (Android)
 *
 * The family board's local cache. Tombstones are never stored — a delete
 * removes the row, because a client that has been told a note is gone has
 * nothing left to remember.
 *
 * iOS counterpart: the NoteEntity fetches in ChatSyncCoordinator.
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Dao
import androidx.room.Query
import androidx.room.Upsert
import kotlinx.coroutines.flow.Flow

@Dao
interface NoteDao {

    /** Board order is stable across devices: oldest change first. */
    @Query("SELECT * FROM notes ORDER BY boardSeq ASC")
    fun observeNotes(): Flow<List<NoteEntity>>

    @Query("SELECT * FROM notes WHERE id = :id")
    suspend fun findById(id: Long): NoteEntity?

    @Upsert
    suspend fun upsert(note: NoteEntity)

    @Query("DELETE FROM notes WHERE id = :id")
    suspend fun delete(id: Long)

    @Query("DELETE FROM notes")
    suspend fun deleteAll()
}
