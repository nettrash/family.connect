/*
 * PendingAttachmentDao.kt
 * Family Connect (Android)
 *
 * The unfinished attachments of queued media sends. Rows live only between
 * pressing Send and the server acking the message; a healthy database
 * holds none (docs/protocol.md, "Sending on an unreliable network").
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query

@Dao
interface PendingAttachmentDao {

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertAll(items: List<PendingAttachmentEntity>)

    /** One send's items, in the sender's order. */
    @Query("SELECT * FROM pending_attachments WHERE clientMsgId = :clientMsgId ORDER BY position ASC")
    suspend fun itemsFor(clientMsgId: String): List<PendingAttachmentEntity>

    /** Every send that still owes an upload, oldest first. */
    @Query(
        "SELECT DISTINCT clientMsgId FROM pending_attachments " +
            "WHERE attachmentId IS NULL ORDER BY localId ASC",
    )
    suspend fun sendsOwingUploads(): List<String>

    /**
     * Every send a background job should still try: it owes an upload AND
     * its message is still on its way — not acked, not failed.
     *
     * [sendsOwingUploads] answers the narrower question of what rows exist,
     * which is right for the in-app resume (it re-reads the row itself and
     * declines quietly). A scheduled job needs the wider one: a refusal
     * the app has already written onto the row as failed leaves its item
     * rows in place, and a job that watched only the items would come back
     * with backoff for ever over a send that can never go.
     */
    @Query(
        "SELECT DISTINCT p.clientMsgId FROM pending_attachments p " +
            "JOIN messages m ON m.clientMsgId = p.clientMsgId " +
            "WHERE p.attachmentId IS NULL AND m.serverId IS NULL AND m.status = 'SENDING' " +
            "ORDER BY p.localId ASC",
    )
    suspend fun sendsWorthUploading(): List<String>

    /** Every staged path still named by a row — what a sweep must keep. */
    @Query("SELECT localPath FROM pending_attachments WHERE localPath IS NOT NULL")
    suspend fun stagedPaths(): List<String>

    /**
     * Record a landed upload. `attachmentId IS NULL` in the WHERE is what
     * makes a racing second pass a no-op rather than a second claim.
     */
    @Query(
        "UPDATE pending_attachments SET attachmentId = :attachmentId, posterUploaded = :posterUploaded " +
            "WHERE localId = :localId AND attachmentId IS NULL",
    )
    suspend fun markUploaded(localId: Long, attachmentId: Long, posterUploaded: Boolean): Int

    /** Spend one of this item's own upload attempts. */
    @Query("UPDATE pending_attachments SET uploadAttempts = uploadAttempts + 1 WHERE localId = :localId")
    suspend fun noteUploadAttempt(localId: Long)

    /**
     * Forget every id for one send — the `attachment_expired` recovery.
     * The bytes are still staged, so the next pass uploads them again.
     */
    @Query(
        "UPDATE pending_attachments SET attachmentId = NULL, posterUploaded = 0 " +
            "WHERE clientMsgId = :clientMsgId",
    )
    suspend fun forgetUploads(clientMsgId: String)

    @Query("DELETE FROM pending_attachments WHERE clientMsgId = :clientMsgId")
    suspend fun deleteFor(clientMsgId: String)

    @Query("DELETE FROM pending_attachments")
    suspend fun deleteAll()
}
