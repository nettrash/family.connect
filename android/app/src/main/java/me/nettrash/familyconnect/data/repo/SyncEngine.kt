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
 *      Then, when the server's max_reaction_seq (step 2) beats the
 *      locally stored reaction cursor: after_seq pages until short —
 *      this is what repairs reactions missed while offline.
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

        // 2. Chat list + authoritative unread counts (and the server's
        // per-chat max_reaction_seq, driving step 3b below).
        val serverCursors = chatRepository.refreshChats().okOrNull() ?: emptyMap()

        // 3. Catch-up per chat, looped while pages come back full.
        for (chatId in chatDao.allChatIds()) {
            while (true) {
                val after = messageDao.maxServerId(chatId) ?: 0L
                val pageSize = messageRepository.catchUp(chatId, after, CATCH_UP_PAGE) ?: break
                if (pageSize < CATCH_UP_PAGE) break
            }
            // 3b. Reactions missed while offline: the server's cursor
            // beats ours → page /reactions from the stored cursor.
            val localSeq = chatDao.maxReactionSeq(chatId) ?: 0L
            if ((serverCursors[chatId]?.reactions ?: 0L) > localSeq) {
                messageRepository.catchUpReactions(chatId, localSeq)
            }
            // 3c. Edits missed while offline. `after_id` is WHERE id >
            // cursor and can never see a change to an OLDER row, so this
            // is the only way a message we already hold is learned to
            // have been rewritten.
            val localEditSeq = chatDao.maxEditSeq(chatId) ?: 0L
            if ((serverCursors[chatId]?.edits ?: 0L) > localEditSeq) {
                messageRepository.catchUpEdits(chatId, localEditSeq)
            }
        }

        // 3d. Repair any location stored without its coordinates. Same
        // reason as edits above: `after_id` can never see an older row, so
        // nothing else would ever fix one.
        messageRepository.repairLocationsMissingCoordinates()

        // 4. Re-send anything still pending — safe, client_msg_id dedups.
        messageRepository.flushPending()
    }

    companion object {
        const val CATCH_UP_PAGE = 200
    }
}
