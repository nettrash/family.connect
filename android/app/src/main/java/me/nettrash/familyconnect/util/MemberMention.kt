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

    // -- The assistant in the strip (#106) ---------------------------------

    /**
     * One chip of the strip above the composer: the assistant or a member.
     *
     * The assistant is not a member for anything but this strip
     * (docs/protocol.md, "Mentioning a member", amended 2026-10-20 for
     * #106): it is never a mention target, never resolved at send, never
     * marked in a bubble as a member is. So it is its own case here rather
     * than a [MentionDto] with a made-up id that [resolve] could one day
     * mistake for somebody.
     */
    sealed interface Suggestion {
        /** Stable across keystrokes, for the strip's list keys. */
        val key: String

        /** `@ai`, shown under the assistant's own name. */
        data class Assistant(val displayName: String) : Suggestion {
            override val key: String get() = "assistant"
        }

        data class Member(val mention: MentionDto) : Suggestion {
            override val key: String get() = "member-${mention.userId}"
        }
    }

    /** `ai`, what the assistant is narrowed by besides its name: the token without its `@`. */
    private val ASSISTANT_WORD = AssistantMention.TOKEN.removePrefix("@")

    /**
     * Whether the assistant is offered for [query]: as a member is by
     * their name — the prefix being typed starts its display name — and
     * also by `ai`, the word that will land in the text, so somebody who
     * knows the token finds it by typing it. Never twice: a draft that
     * already asks it (anywhere before the `@` being typed) has nothing
     * to gain from a second `@ai`.
     */
    fun offersAssistant(draft: String, query: String, displayName: String): Boolean {
        val at = draft.lastIndexOf('@')
        if (at > 0 && AssistantMention.mentions(draft.substring(0, at))) return false
        val needle = query.lowercase()
        return needle.isEmpty() ||
            displayName.lowercase().startsWith(needle) ||
            ASSISTANT_WORD.startsWith(needle)
    }

    /**
     * What the strip offers for [draft] (#106): the assistant FIRST, above
     * the members, when [assistantName] says it may be offered at all —
     * the caller decides that (the family chat, a server that advertises
     * an assistant WITH its processor); then the members [candidates]
     * offers, never the assistant's own account among them even if a
     * roster carried it. Empty outside a token.
     */
    fun suggestions(
        draft: String,
        roster: List<MentionDto>,
        excluding: Set<Long>,
        assistantName: String?,
        assistantUserId: Long?,
    ): List<Suggestion> {
        val query = query(draft) ?: return emptyList()
        val members = candidates(roster, query, excluding + setOfNotNull(assistantUserId))
            .map { Suggestion.Member(it) }
        val assistant = assistantName
            ?.takeIf { offersAssistant(draft, query, it) }
            ?.let { Suggestion.Assistant(it) }
        return listOfNotNull(assistant) + members
    }

    /**
     * The draft with the trailing `@prefix` replaced by [Suggestion]'s
     * token: `@Name ` for a member, and for the assistant `@ai ` — the
     * token "Mentioning the assistant in the family chat" defines, NEVER
     * `@<display name>`, which the server would read as plain words and
     * nobody would answer.
     */
    fun accept(draft: String, suggestion: Suggestion): String = when (suggestion) {
        is Suggestion.Assistant -> accept(draft, ASSISTANT_WORD)
        is Suggestion.Member -> accept(draft, suggestion.mention.name)
    }
}
