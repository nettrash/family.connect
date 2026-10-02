/*
 * TranscriptDao.kt
 * Family Connect (Android)
 *
 * The texts of recordings this member asked for, one per attachment
 * (docs/protocol.md, "Transcripts on request"). See [TranscriptEntity].
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Dao
import androidx.room.Query
import androidx.room.Upsert
import kotlinx.coroutines.flow.Flow

@Dao
interface TranscriptDao {

    @Query("SELECT * FROM transcripts WHERE attachmentId = :attachmentId")
    fun observe(attachmentId: Long): Flow<TranscriptEntity?>

    @Query("SELECT * FROM transcripts WHERE attachmentId = :attachmentId")
    suspend fun find(attachmentId: Long): TranscriptEntity?

    @Upsert
    suspend fun upsert(transcript: TranscriptEntity)

    @Query("UPDATE transcripts SET hidden = :hidden WHERE attachmentId = :attachmentId")
    suspend fun setHidden(attachmentId: Long, hidden: Boolean)
}
