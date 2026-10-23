/*
 * PushFamilyRouter.kt
 * Family Connect (Android)
 *
 * A push tap opens the app IN ITS FAMILY (#88; docs/protocol.md, "Several
 * families" — "Pushes"): one that carries `family_id` switches to that
 * family first; a direct chat's push carries none, and opens in the family
 * shown when the chat is listed there, else in the first that lists it.
 *
 * A seam so MainViewModel's tests stay free of Room and the repositories.
 */

package me.nettrash.familyconnect.data.push

import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.repo.FamilyRepository
import javax.inject.Inject
import javax.inject.Singleton

fun interface PushFamilyRouter {

    /** Switch to the family a push about [familyId] / [chatId] belongs to, before its route is followed. */
    suspend fun openIn(familyId: Long?, chatId: Long?)

    companion object {
        /** For tests that never route a push between families. */
        val NONE = PushFamilyRouter { _, _ -> }
    }
}

@Singleton
class FamilyPushRouter @Inject constructor(
    private val familyRepository: FamilyRepository,
    private val chatDao: ChatDao,
) : PushFamilyRouter {

    override suspend fun openIn(familyId: Long?, chatId: Long?) {
        val chatFamilies = chatId?.let { chatDao.getById(it)?.familyIds }
        familyRepository.switchForPush(familyId, chatFamilies)
    }
}
