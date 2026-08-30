//
//  BlockedMessageRule.kt
//  FamilyConnect
//
//  What a blocked member's content looks like, as arithmetic.
//
//  Kept free of Compose and Android, like the rest of this package, so the
//  rules can be pinned directly by a plain JUnit test. One rule in one
//  place, because the alternative is the same `contains` at forty draw
//  sites and a leak wherever somebody forgets it.
//
//  The block is SILENT: nothing about it ever reaches the person blocked.
//  Everything here is therefore about what the BLOCKER's own screen draws,
//  and never about what either side sends (docs/protocol.md, "Blocking a
//  member").
//

package me.nettrash.familyconnect.ui.chat

object BlockedMessageRule {

    /**
     * Whether this row draws as the "Hidden — blocked member" placeholder.
     *
     * A hidden row draws the placeholder and the timestamp and NOTHING
     * else: no display name, no avatar, no attachment thumbnail, no
     * reaction chips — all of which come back with the reveal.
     *
     * Own messages are never hidden: blocking yourself is refused by the
     * server, so the guard is belt-and-braces against a corrupt store
     * rather than a real case.
     */
    fun isHidden(senderId: Long, myUserId: Long, blockedUserIds: Set<Long>): Boolean =
        senderId != myUserId && senderId in blockedUserIds

    /**
     * Whether a QUOTE of somebody draws as the same placeholder.
     *
     * Identical arithmetic to [isHidden] and deliberately its own name: a
     * quote is masked INDEPENDENTLY of the message quoting it, and the two
     * levels of quote are independent of each other again. Without the
     * rule the block is defeated by the most ordinary thing in the chat —
     * anybody replying to a blocked member carries 120 characters of them
     * into a bubble that is not hidden.
     *
     * The server sends the excerpt unchanged, because it recomputes quotes
     * on every read for everybody; the hiding is the client's alone.
     */
    fun isQuoteHidden(authorId: Long?, myUserId: Long, blockedUserIds: Set<Long>): Boolean =
        authorId != null && isHidden(authorId, myUserId, blockedUserIds)

    /**
     * Whether a board note hides its CONTENT as well as its author.
     *
     * The note is the one object where the text goes too: it is a piece of
     * writing pinned to a shared wall with no bubble to collapse into a
     * hidden row, so dropping only the name would hide nothing that
     * mattered (docs/protocol.md, "Board").
     */
    fun isNoteHidden(authorId: Long, myUserId: Long, blockedUserIds: Set<Long>): Boolean =
        isHidden(authorId, myUserId, blockedUserIds)

    /**
     * The voters whose face and name a poll option may draw.
     *
     * The tallies, the bars and the "N of M voted" line are NOT filtered:
     * integers are not presence, and a count that changed when you blocked
     * somebody would tell you they had reacted. Only the identities go.
     */
    fun drawableVoters(voterIds: List<Long>, myUserId: Long, blockedUserIds: Set<Long>): List<Long> =
        voterIds.filterNot { isHidden(it, myUserId, blockedUserIds) }
}
