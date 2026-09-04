package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.db.MessageEntity

/**
 * When a row IS the assistant's "still working" state.
 *
 * The server stores and fans out an EMPTY assistant message before it calls
 * the provider, so a bubble appears at once and every one of the member's
 * devices has a row to fill (docs/protocol.md, "How a picture comes back",
 * step 2). Step 3 says what that row means, and says it about the message
 * rather than about one app process:
 *
 * > **no `ai_delta` frames.** An image model produces no token stream and
 * > there is nothing honest to stream. The empty row is the "still working"
 * > state — which is exactly what it already is before the first delta of a
 * > text answer.
 *
 * Which is why this is asked of the ROW. The set of ids
 * `MessageRepository.streamingMessageIds` holds is a fact about what this
 * launch watched arrive: a history page populates none of it, a cold start
 * populates none of it, and a picture answer populates none of it even
 * live, because there is no delta to populate it with. Deciding "working"
 * from that set alone drew a `/draw` in progress as a completely blank
 * balloon — no cursor, no error, nothing — for anybody who reopened the app
 * while it was still generating.
 *
 * Same rule on iOS and macOS
 * (`MessagePresentation.isAwaitedAssistantAnswer`), pinned by the same
 * cases.
 */
object AssistantAnswer {

    /**
     * Does this row read as an answer that has not arrived yet?
     *
     * Deliberately narrow, and every clause earns its place:
     *
     * - it CARRIES NOTHING — no body, no attachment, no poll, no call. Any
     *   of those is an answer that landed, and a picture answer gains its
     *   attachment through `message_edited`, which is precisely what ends
     *   this state;
     * - it is not MINE. My own empty row is a send in flight;
     * - and it is from somebody who could be the assistant: its own `ai`
     *   chat, where two participants mean anything not mine is its, or the
     *   family chat, where only its reserved account names it.
     *
     * A member cannot produce this shape anyway — the server refuses a send
     * with neither body nor attachment (`message_empty`) — so an empty,
     * attachment-less row in an `ai` chat is the placeholder and nothing
     * else.
     *
     * @param isAssistantChat the chat's kind is `ai`.
     * @param assistantUserId the reserved account, when this server has one.
     * @param myUserId this device's own user, or null before sign-in
     *   finished — in which case nothing is "mine" and the other clauses
     *   still decide.
     */
    fun isAwaited(
        entity: MessageEntity,
        isAssistantChat: Boolean,
        assistantUserId: Long?,
        myUserId: Long?,
    ): Boolean {
        val carriesNothing = entity.body.isEmpty() &&
            entity.attachmentList.isEmpty() &&
            entity.pollJson == null &&
            entity.call == null
        if (!carriesNothing) return false
        if (myUserId != null && entity.senderId == myUserId) return false
        return isAssistantChat || entity.senderId == assistantUserId
    }

    /**
     * What the bubble is actually given: the row rule, the live set, and
     * the failure mark, resolved in the order they outrank each other.
     *
     * A FAILURE wins over both. The cursor and the "ask again" line are
     * drawn in the same place, and an `ai_error` this launch saw is newer
     * information than the row's own shape. After a relaunch that knowledge
     * is gone and the row reads as working again — which is exactly what
     * protocol.md says an empty row means, and the member's remedy is the
     * one it names: ask again.
     *
     * The SET is still consulted, because it answers the case the row
     * cannot: a text answer part-way through has a body, so the row no
     * longer "carries nothing" while the cursor still belongs after its
     * last fragment.
     */
    fun isWorking(
        entity: MessageEntity,
        isAssistantChat: Boolean,
        assistantUserId: Long?,
        myUserId: Long?,
        streamingIds: Set<Long>,
        failedIds: Set<Long>,
    ): Boolean {
        val serverId = entity.serverId
        if (serverId != null) {
            if (serverId in failedIds) return false
            if (serverId in streamingIds) return true
        }
        return isAwaited(entity, isAssistantChat, assistantUserId, myUserId)
    }
}
