/*
 * NoteDao.kt
 * Family Connect (Android)
 *
 * The family board's local cache. A delete removes the row AND remembers the
 * id in `goneNotes`: a client that has been told a note is gone must not be
 * talked out of it by an older copy still travelling (docs/protocol.md,
 * "Board", and [GoneNoteEntity]).
 *
 * iOS counterpart: the NoteEntity fetches in ChatSyncCoordinator.
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
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

    /**
     * What a full board read leaves out, gone: every note at or below the
     * read's mark that it did not list (docs/protocol.md, "Board").
     */
    @Query("DELETE FROM notes WHERE boardSeq <= :max AND id NOT IN (:listed)")
    suspend fun deleteNotListed(max: Long, listed: List<Long>)

    /**
     * The ids a full read is about to drop, read BEFORE it drops them — a note
     * the read left out is a note that is gone, and it has to be remembered as
     * gone or the next page that carries it puts it back.
     */
    @Query("SELECT id FROM notes WHERE boardSeq <= :max AND id NOT IN (:listed)")
    suspend fun idsNotListed(max: Long, listed: List<Long>): List<Long>

    // ---- the notes a tombstone has taken ---------------------------------

    /** Remembered, once and for all. Idempotent: a second tombstone is the same news. */
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun remember(gone: GoneNoteEntity)

    @Query("SELECT EXISTS(SELECT 1 FROM goneNotes WHERE noteId = :id)")
    suspend fun isGone(id: Long): Boolean

    @Query("SELECT noteId FROM goneNotes")
    suspend fun goneIds(): List<Long>
}
