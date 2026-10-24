/*
 * PackDao.kt
 * Family Connect (Android)
 *
 * The family's sticker pack, cached locally the way the board is
 * (docs/protocol.md, "Sticker pack"). A removal deletes the row AND
 * remembers the id in `gonePackItems`: a client that has been told an item
 * is gone must not be talked out of it by an older copy still travelling —
 * see [GonePackItemEntity].
 *
 * iOS counterpart: the PackItemEntity fetches in ChatSyncCoordinator.
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.Upsert
import kotlinx.coroutines.flow.Flow

@Dao
interface PackDao {

    /**
     * Panel order: the order the items were added, which is ascending id —
     * NOT ascending seq as the board reads, because a pack item never
     * changes after it is added and the panel must not shuffle.
     */
    @Query("SELECT * FROM packItems ORDER BY id ASC")
    fun observeItems(): Flow<List<PackItemEntity>>

    /** One family's pack — the CURRENT family's is what the panel shows. */
    @Query("SELECT * FROM packItems WHERE familyId = :familyId ORDER BY id ASC")
    fun observeItems(familyId: Long): Flow<List<PackItemEntity>>

    /** Every family's items — what the prefetch fills the disk with. */
    @Query("SELECT * FROM packItems ORDER BY id ASC")
    suspend fun items(): List<PackItemEntity>

    @Query("SELECT * FROM packItems WHERE familyId = :familyId ORDER BY id ASC")
    suspend fun items(familyId: Long): List<PackItemEntity>

    @Query("SELECT * FROM packItems WHERE id = :id")
    suspend fun findById(id: Long): PackItemEntity?

    @Query("SELECT COUNT(*) FROM packItems")
    suspend fun count(): Int

    @Query("SELECT COUNT(*) FROM packItems WHERE familyId = :familyId")
    suspend fun count(familyId: Long): Int

    @Upsert
    suspend fun upsert(item: PackItemEntity)

    @Query("DELETE FROM packItems WHERE id = :id")
    suspend fun delete(id: Long)

    /**
     * What a full pack read leaves out, gone: every item at or below the
     * read's mark that it did not list. An item held ABOVE the mark arrived
     * after the read was taken, and stays (docs/protocol.md, "Sticker
     * pack").
     */
    @Query("DELETE FROM packItems WHERE familyId = :familyId AND packSeq <= :max AND id NOT IN (:listed)")
    suspend fun deleteNotListed(familyId: Long, max: Long, listed: List<Long>)

    /** The ids a full read is about to drop, read BEFORE it drops them. */
    @Query("SELECT id FROM packItems WHERE familyId = :familyId AND packSeq <= :max AND id NOT IN (:listed)")
    suspend fun idsNotListed(familyId: Long, max: Long, listed: List<Long>): List<Long>

    /** This account is out of [familyId]: its pack goes with it. */
    @Query("DELETE FROM packItems WHERE familyId = :familyId")
    suspend fun deleteFamily(familyId: Long)

    /** Items cached before several families (family 0) are the home family's. */
    @Query("UPDATE packItems SET familyId = :homeFamilyId WHERE familyId = 0")
    suspend fun claimLegacy(homeFamilyId: Long)

    // ---- the items a tombstone has taken ---------------------------------

    /** Remembered, once and for all. Idempotent: a second tombstone is the same news. */
    @Insert(onConflict = OnConflictStrategy.IGNORE)
    suspend fun remember(gone: GonePackItemEntity)

    @Query("SELECT EXISTS(SELECT 1 FROM gonePackItems WHERE itemId = :id)")
    suspend fun isGone(id: Long): Boolean
}
