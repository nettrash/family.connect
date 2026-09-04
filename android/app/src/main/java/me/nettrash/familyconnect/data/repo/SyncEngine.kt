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
 *      Read ONCE per chat and advanced by each page's largest id, never
 *      re-read from the store between pages (see the loop below).
 *      Then, when the server's max_reaction_seq (step 2) beats the
 *      locally stored reaction cursor: after_seq pages until short —
 *      this is what repairs reactions missed while offline.
 *   3f. The board's own catch-up, on the third cursor — a board is
 *       nothing but changes to older rows, which after_id cannot see.
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
    private val boardRepository: BoardRepository,
    private val chatDao: ChatDao,
    private val messageDao: MessageDao,
) {

    suspend fun resync() {
        // 0. The outbox, BEFORE anything can fail.
        //
        // It used to be step 4, at the end of this function, behind the
        // `return` below — so on the network most likely to have stranded
        // a message, the one thing that recovers it never ran. It is not a
        // step of the read pipeline and is ordered against nothing after it
        // (docs/protocol.md, "Best-effort delivery"): re-sending is
        // idempotent on client_msg_id, so doing it first is never wrong.
        messageRepository.flushPending()

        // 1. Membership first — if we were removed or the session died,
        // the event handlers reroute and the rest is moot.
        val me = sessionRepository.refreshMe().okOrNull() ?: return
        if (!me.canChat) return

        val mine = familyRepository.refreshMine().okOrNull()

        // 2. Chat list + authoritative unread counts (and the server's
        // per-chat max_reaction_seq, driving step 3b below).
        val serverCursors = chatRepository.refreshChats().okOrNull() ?: emptyMap()

        // 3. Catch-up per chat, looped while pages come back full.
        //
        // The cursor belongs to the LOOP: read once here, then advanced by
        // the largest id each page actually returned. Re-reading
        // `maxServerId` per page instead let a live `message` frame landing
        // mid-loop (the socket is open — that is what started this resync)
        // move the cursor to its own much higher id, and everything between
        // the last page and it was skipped for good: `after_id` never looks
        // back, and `loadOlder` only pages older than the OLDEST row held.
        // protocol.md, "Best-effort delivery", step 3.
        for (chatId in chatDao.allChatIds()) {
            var after = messageDao.maxServerId(chatId) ?: 0L
            while (true) {
                val page = messageRepository.catchUp(chatId, after, CATCH_UP_PAGE) ?: break
                page.maxServerId?.let { after = maxOf(after, it) }
                if (page.size < CATCH_UP_PAGE) break
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
            // 3d. Votes missed while offline, for the same reason again:
            // a vote is a change to an OLDER row, which `after_id` can
            // never see. Gated on the chat's max_poll_seq beating the
            // stored cursor, so a family that has never held a poll — or
            // one whose polls this device is level with — makes no
            // request at all.
            val localPollSeq = chatDao.maxPollSeq(chatId) ?: 0L
            if ((serverCursors[chatId]?.polls ?: 0L) > localPollSeq) {
                messageRepository.catchUpPolls(chatId, localPollSeq)
            }
        }

        // 3d½. The board, on its own cursor — the third of the same shape,
        // and for the third time the same reason: `after_id` can never see
        // a change to an older row, and a board is nothing BUT changes to
        // older rows. The family read above already told us the server's
        // max, so a board nothing has happened on costs no request at all.
        //
        // It belongs HERE, and not only on the board screen where it used
        // to live, because a cache filled by socket frames alone is a cache
        // full of holes: a note this device never held would first appear
        // the moment somebody DRAGGED it, and a badge counting whatever had
        // materialised counted that drag as news (issue #53). The Apple
        // clients have always caught the board up in their resync.
        boardRepository.catchUpBoard(mine?.maxBoardSeq ?: 0L)

        // 3e. Repair any location stored without its coordinates. Same
        // reason as edits above: `after_id` can never see an older row, so
        // nothing else would ever fix one.
        messageRepository.repairLocationsMissingCoordinates()
    }

    companion object {
        const val CATCH_UP_PAGE = 200
    }
}
