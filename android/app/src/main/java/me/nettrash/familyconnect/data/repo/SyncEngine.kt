/*
 * SyncEngine.kt
 * Family Connect (Android)
 *
 * The reconnect resync, exactly as docs/protocol.md prescribes
 * ("Best-effort delivery" — the socket is a live wire, REST is the
 * source of truth):
 *
 *   1. GET /me                — reconcile membership.
 *   2. GET /chats             — chat list, previews, authoritative unread.
 *   3. Per chat: after_id=<max known id> pages (limit 200) until short —
 *      message ids are globally monotonic, so max(serverId) is the cursor.
 *   4. Re-send locally pending outbound messages (client_msg_id dedups).
 *
 * Also refreshes the family roster so sender names resolve.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Repo/SyncEngine.swift
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.MessageDao
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class SyncEngine @Inject constructor(
    private val sessionRepository: SessionRepository,
    private val chatRepository: ChatRepository,
    private val familyRepository: FamilyRepository,
    private val messageRepository: MessageRepository,
    private val chatDao: ChatDao,
    private val messageDao: MessageDao,
) {

    suspend fun resync() {
        // 1. Membership first — if we were removed or the session died,
        // the event handlers reroute and the rest is moot.
        val me = sessionRepository.refreshMe().okOrNull() ?: return
        if (!me.canChat) return

        familyRepository.refreshMine()

        // 2. Chat list + authoritative unread counts.
        chatRepository.refreshChats()

        // 3. Catch-up per chat, looped while pages come back full.
        for (chatId in chatDao.allChatIds()) {
            while (true) {
                val after = messageDao.maxServerId(chatId) ?: 0L
                val pageSize = messageRepository.catchUp(chatId, after, CATCH_UP_PAGE) ?: break
                if (pageSize < CATCH_UP_PAGE) break
            }
        }

        // 4. Re-send anything still pending — safe, client_msg_id dedups.
        messageRepository.flushPending()
    }

    companion object {
        const val CATCH_UP_PAGE = 200
    }
}
