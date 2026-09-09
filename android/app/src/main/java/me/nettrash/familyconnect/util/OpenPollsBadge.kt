/*
 * OpenPollsBadge.kt
 * Family Connect (Android)
 *
 * What the open-polls badge counts, in ONE place (docs/protocol.md,
 * "Finding the open ones").
 *
 * The rule is: **open polls this reader has not voted in**. Not "all open
 * polls", and not "polls created since you last looked".
 *
 * Why not all of them. A badge is a claim that there is something for YOU to
 * do. A count of every open poll stays lit after you have answered all of
 * them, until somebody else gets round to closing them — so it stops meaning
 * anything within a day, and a family learns to ignore it. This one clears
 * itself the moment you vote, because voting is exactly the thing it was
 * asking for.
 *
 * Why not a "seen" mark. The board needs one (`content_seq` and the mark
 * beside it) because a note is something to READ, and there is no act that
 * proves you have. A poll has one: the vote. So this needs no server field,
 * no DataStore key, no Room column, no seed rule for a device that has never
 * looked, and nothing to keep in step — a whole apparatus the board had to
 * build and this does not.
 *
 * Why it can be computed here at all. Every poll carries the full list of
 * user ids that chose each option, because a frame is serialised once and
 * sent to everybody: a field whose value depends on who is reading — "did I
 * vote" — cannot exist on the wire, and the protocol says in terms that
 * clients derive it from the list. This is that derivation.
 *
 * iOS counterpart: Core/OpenPollsBadge.swift — same rule, same vectors
 * (OpenPollsBadgeTests.swift / OpenPollsBadgeTest.kt).
 */

package me.nettrash.familyconnect.util

/**
 * The two things the badge rule needs of a poll, whichever type is holding
 * it — the wire DTO, or whatever a screen has mapped it into.
 */
interface PollVoteState {
    val closed: Boolean

    /** Everyone who has chosen any option. */
    val voterIds: List<Long>
}

object OpenPollsBadge {

    /**
     * How many of these polls this reader still has to answer.
     *
     * A CLOSED poll never counts, whatever the reader did: a closed poll is a
     * result, and there is nothing left to ask of anybody. The endpoint that
     * feeds the surface returns only open ones, so that guard is belt and
     * braces there — but the badge is computed over the polls the CHAT holds,
     * where closed ones certainly do appear, and there it is the whole filter.
     */
    fun count(polls: List<PollVoteState>, currentUserId: Long): Int =
        polls.count { !it.closed && !it.voterIds.contains(currentUserId) }

    /**
     * Whether this reader has voted in one poll.
     *
     * One choice per member, so any appearance is the appearance — but this
     * deliberately does not assume that: it asks whether the id appears at
     * all, which stays correct if the protocol ever grows multiple choice,
     * and costs nothing today.
     */
    fun hasVoted(poll: PollVoteState, userId: Long): Boolean =
        poll.voterIds.contains(userId)
}
