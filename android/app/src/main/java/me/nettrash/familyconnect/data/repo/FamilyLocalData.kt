/*
 * FamilyLocalData.kt
 * Family Connect (Android)
 *
 * What this device keeps per FAMILY, as one seam (#88; docs/protocol.md,
 * "Several families"): claiming the rows cached before several families for
 * the home family, and dropping ONE family's chats, board, pack and roster
 * when this account is out of it — never the whole database, which is what
 * leaving "the family" used to mean.
 *
 * A seam rather than DAO calls inside SessionRepository for the reason
 * LocalDataWiper is one: the session's plain-JVM tests record the call
 * without Room.
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.NoteDao
import me.nettrash.familyconnect.data.db.PackDao
import javax.inject.Inject
import javax.inject.Singleton

interface FamilyLocalData {

    /**
     * Rows cached before several families — family 0 on notes, pack items
     * and the roster — are the HOME family's: the only family an app from
     * before ever saw. Idempotent: nothing is family 0 the second time.
     */
    suspend fun claimLegacy(homeFamilyId: Long)

    /**
     * This account is out of [familyId]: drop that family's chats, board,
     * pack and roster, and only that family's. A direct chat stays while its
     * `family_ids` still name one of [remainingFamilyIds].
     */
    suspend fun dropFamily(familyId: Long, remainingFamilyIds: Set<Long>)

    companion object {
        /** For tests that never hold more than one family. */
        val NONE: FamilyLocalData = object : FamilyLocalData {
            override suspend fun claimLegacy(homeFamilyId: Long) = Unit
            override suspend fun dropFamily(familyId: Long, remainingFamilyIds: Set<Long>) = Unit
        }
    }
}

@Singleton
class RoomFamilyLocalData @Inject constructor(
    private val chatRepository: ChatRepository,
    private val packRepository: PackRepository,
    private val memberDao: MemberDao,
    private val noteDao: NoteDao,
    private val packDao: PackDao,
) : FamilyLocalData {

    override suspend fun claimLegacy(homeFamilyId: Long) {
        memberDao.claimLegacy(homeFamilyId)
        noteDao.claimLegacy(homeFamilyId)
        packDao.claimLegacy(homeFamilyId)
    }

    override suspend fun dropFamily(familyId: Long, remainingFamilyIds: Set<Long>) {
        // Chats first: their messages name members, and a bubble left for an
        // instant without a roster row would draw "Member 11".
        chatRepository.dropFamily(familyId, remainingFamilyIds)
        noteDao.deleteFamily(familyId)
        // Through the repository: the pictures live on disk beside the rows.
        packRepository.dropFamily(familyId)
        memberDao.deleteFamily(familyId)
    }
}
