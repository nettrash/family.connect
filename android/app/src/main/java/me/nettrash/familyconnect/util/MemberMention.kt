/*
 * MemberMention.kt
 * Family Connect (Android)
 *
 * Member mentions (docs/protocol.md, "Mentioning a member"): the grammar
 * every composer and every bubble share.
 *
 * The GRAMMAR is the assistant token's rule applied to a name: `@` followed
 * by exactly the name, at a boundary on both sides — an ASCII letter, digit
 * or `_` after it means it is a longer word, so `@Ann` is not found inside
 * `@Anna` and `mail@Anna` is an address. The server checks the same thing
 * (server/src/mentions.rs `names_member`) and refuses a mention the body
 * does not carry, so the two must agree.
 *
 * The RESOLUTION is from the text, at send: every active member whose
 * `@Name` the body carries, each member once, in order of first
 * appearance — so a name typed by hand mentions too, and a name deleted
 * after picking does not. Names are tried LONGEST FIRST and a token once
 * claimed is not offered again, so `@Anna Lee` names Anna Lee and not
 * also Anna: the boundary after `@Anna` is the space, and without the
 * claim both would be named and both notified.
 *
 * iOS counterpart: Models/MemberMentions.swift.
 */

package me.nettrash.familyconnect.util

import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.ui.chat.AssistantMention

object MemberMention {
    /** The private scheme a highlighted name carries, so a tap reaches the member. */
    const val SCHEME = "fcmember"

    fun url(userId: Long): String = "$SCHEME://$userId"

    fun userIdFrom(url: String): Long? =
        url.takeIf { it.startsWith("$SCHEME://") }?.removePrefix("$SCHEME://")?.toLongOrNull()

    /** Every `@name` in [body], the `@` included, as UTF-16 index ranges. */
    fun ranges(body: String, name: String): List<IntRange> {
        if (name.isEmpty() || body.length < name.length + 1) return emptyList()
        val found = mutableListOf<IntRange>()
        var index = 0
        val length = name.length + 1
        while (index + length <= body.length) {
            if (body[index] == '@' &&
                body.regionMatches(index + 1, name, 0, name.length) &&
                (index == 0 || AssistantMention.isBoundary(body[index - 1])) &&
                (index + length == body.length || AssistantMention.isBoundary(body[index + length]))
            ) {
                found += index until index + length
                index += length
            } else {
                index += 1
            }
        }
        return found
    }

    fun names(body: String, name: String): Boolean = ranges(body, name).isNotEmpty()

    /** The members [body] names, resolved against the roster — see the header. */
    fun resolve(body: String, roster: List<MentionDto>): List<MentionDto> {
        if ('@' !in body) return emptyList()
        val seen = mutableSetOf<Long>()
        val claimed = mutableListOf<IntRange>()
        return roster
            // Longest name first, then the LOWER id — a family may hold two
            // members called Anna, and one `@Anna` can only name one of
            // them. Roster order would name a different Anna on each
            // platform; the id is the one tie-break all three ports share
            // (docs/protocol.md, "Mentioning a member").
            .sortedWith(compareByDescending<MentionDto> { it.name.length }.thenBy { it.userId })
            .mapNotNull { member ->
                if (member.userId in seen) return@mapNotNull null
                val first = ranges(body, member.name).firstOrNull { range -> claimed.none { it.overlaps(range) } }
                    ?: return@mapNotNull null
                seen += member.userId
                claimed += first
                first.first to member
            }
            .sortedBy { it.first }
            .map { it.second }
    }

    /**
     * Every `@Name` token the message draws, one owner per token: the
     * members' tokens, longest name first, a token claimed once — the same
     * rule [resolve] named them by, so the bubble marks `@Anna Lee` as Anna
     * Lee even when the message names Anna as well.
     */
    fun tokens(text: String, mentions: List<MentionDto>): List<Pair<IntRange, MentionDto>> {
        if (mentions.isEmpty()) return emptyList()
        val claimed = mutableListOf<IntRange>()
        return mentions
            .sortedWith(compareByDescending<MentionDto> { it.name.length }.thenBy { it.userId })
            .flatMap { mention ->
                ranges(text, mention.name)
                    .filter { range -> claimed.none { it.overlaps(range) } }
                    .onEach { claimed += it }
                    .map { it to mention }
            }
            .sortedBy { it.first.first }
    }

    private fun IntRange.overlaps(other: IntRange): Boolean = first <= other.last && other.first <= last

    /**
     * The prefix being typed after a trailing `@`, or null when the composer
     * is not mid-mention: no `@` at a boundary, or a line break after it.
     * Empty when the `@` was just typed — every candidate is offered then.
     */
    fun query(draft: String): String? {
        val at = draft.lastIndexOf('@')
        if (at < 0) return null
        if (at > 0 && !AssistantMention.isBoundary(draft[at - 1])) return null
        val tail = draft.substring(at + 1)
        if ('\n' in tail) return null
        return tail
    }

    /** The roster narrowed to what [query] could be the start of, minus [excluding]. */
    fun candidates(roster: List<MentionDto>, query: String, excluding: Set<Long>): List<MentionDto> {
        val needle = query.lowercase()
        return roster.filter { member ->
            member.userId !in excluding &&
                (needle.isEmpty() || member.name.lowercase().startsWith(needle))
        }
    }

    /** The draft with the trailing `@prefix` replaced by `@Name `. */
    fun accept(draft: String, name: String): String {
        val at = draft.lastIndexOf('@')
        if (at < 0) return "$draft@$name "
        return draft.substring(0, at) + "@" + name + " "
    }
}
