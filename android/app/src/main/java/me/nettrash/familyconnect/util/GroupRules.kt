/*
 * GroupRules.kt
 * Family Connect (Android)
 *
 * Groups inside a family (#107; docs/protocol.md, "Groups"), as the two
 * tables every client has to agree on: who may do what to a group, and what
 * a group offers next to the family chat, a direct chat and the assistant's
 * own. Arithmetic, free of Compose and Android, so a plain JUnit test pins
 * it (GroupRulesTest) — and one object, so the chat, the thread, the list
 * and the group's info screen cannot disagree about it.
 *
 * Shared with the other clients by CONTRACT rather than by code
 * (docs/groups-2026-10-21.md, §7: "one shared table per client, pinned by
 * tests"). It MIRRORS server/src/handlers_group.rs: a disagreement is
 * either a button that can only ever be refused, or one missing where the
 * server would have said yes.
 */

package me.nettrash.familyconnect.util

import androidx.annotation.StringRes
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.MentionDto

object GroupRules {

    // -- Chat kinds (docs/protocol.md, `Chat`) --------------------------------

    const val FAMILY = "family"
    const val DIRECT = "direct"
    const val AI = "ai"
    const val GROUP = "group"

    /** A group's name, trimmed, is 1 to this many characters. */
    const val MAX_TITLE_CHARS = 64

    // -- What a chat offers -----------------------------------------------------

    fun isGroup(kind: String?): Boolean = kind == GROUP

    /**
     * The family chat or a group: a conversation of several of the family.
     * "Inside, it is the family chat with fewer people" — sender names,
     * member mentions, polls, `@ai`, the assistant's picture rules — "and
     * every rule this document states for the family chat applies to a
     * group unless this section or the rule itself says otherwise".
     */
    fun isShared(kind: String?): Boolean = kind == FAMILY || kind == GROUP

    /** The sender's name over a run of bubbles: many people write here. */
    fun showsSenderNames(kind: String?): Boolean = isShared(kind)

    /** `@Name` mentions — of that group's members, in a group. */
    fun offersMemberMentions(kind: String?): Boolean = isShared(kind)

    /** Polls ("Polls", amended 2026-10-22). Anywhere else is `invalid_poll`. */
    fun offersPolls(kind: String?): Boolean = isShared(kind)

    /**
     * Where an `@ai` mention is answered, for everyone in the chat to read
     * ("Mentioning the assistant in the family chat", amended 2026-10-22).
     * The member's own `ai` chat answers everything and is not this.
     */
    fun answersAssistantMention(kind: String?): Boolean = isShared(kind)

    /** A call lives in a direct chat: a group is `invalid_call`, like the family chat. */
    fun offersCalls(kind: String?): Boolean = kind == DIRECT

    /** ✓✓ "Seen": one peer marker, so a direct chat only — never many readers. */
    fun drawsSeenTicks(kind: String?): Boolean = kind == DIRECT

    /** The video door (#79): any chat of people, never the assistant's. */
    fun offersVideoMessages(kind: String?): Boolean = kind == FAMILY || kind == DIRECT || kind == GROUP

    /**
     * Somebody else's recording follows the DIRECT chat's rule here — that
     * sender's consent alone, no owner's switch (D4; "Transcripts on
     * request", amended 2026-10-22). The family chat alone has the
     * owner's `ai_transcripts`.
     */
    fun transcriptsFollowDirectRule(kind: String?): Boolean = kind == DIRECT || kind == GROUP

    // -- Who may do what (D1: the creator is admin) -----------------------------

    /** What a member can do to a group from its info screen. */
    enum class Action { RENAME, ADD_MEMBERS, REMOVE_MEMBER, LEAVE, DELETE }

    /**
     * May [me] do [action] to this chat?
     *
     * Only to a group, and only while [me] is in it. RENAME, ADD_MEMBERS,
     * REMOVE_MEMBER and DELETE are the admin's; LEAVE is anybody's. For
     * REMOVE_MEMBER, [target] must be somebody else who is in it — removing
     * yourself is leaving, and the server answers it as such.
     */
    fun may(
        action: Action,
        kind: String?,
        memberIds: List<Long>,
        adminUserId: Long?,
        me: Long?,
        target: Long? = null,
    ): Boolean {
        if (!isGroup(kind) || me == null || me !in memberIds) return false
        val isAdmin = adminUserId == me
        return when (action) {
            Action.RENAME, Action.ADD_MEMBERS, Action.DELETE -> isAdmin
            Action.REMOVE_MEMBER -> isAdmin && target != null && target != me && target in memberIds
            Action.LEAVE -> true
        }
    }

    // -- Names and members ------------------------------------------------------

    /**
     * The name as it will be sent: trimmed, and 1 to [MAX_TITLE_CHARS]
     * characters — or null when the server would answer `validation`.
     * Characters are code points, as the server counts them (`chars()`), so
     * an emoji is one and not two.
     */
    fun validName(raw: String): String? {
        val trimmed = raw.trim()
        val count = trimmed.codePointCount(0, trimmed.length)
        return trimmed.takeIf { count in 1..MAX_TITLE_CHARS }
    }

    /**
     * Who a new group may be started with: the family's active members
     * ([activeIds] — never somebody who left, never a deleted account, never
     * the assistant), not the creator, who is added anyway.
     *
     * Blocked members are NOT left out: "A block is never a reason to refuse
     * an add, in either direction" — the server never refuses one, and a
     * group behaves like the family chat about a block.
     */
    fun startableWith(activeIds: List<Long>, me: Long?): List<Long> = activeIds.filter { it != me }

    /** Who Add Members offers: active family members not already in the group. */
    fun addable(activeIds: List<Long>, memberIds: List<Long>): List<Long> =
        activeIds.filter { it !in memberIds }

    /**
     * Who a mention may name here: the family's active roster in the family
     * chat, and in a group only that group's members — "somebody outside it
     * would not even receive the message" ("Mentioning a member").
     */
    fun mentionRoster(roster: List<MentionDto>, kind: String?, memberIds: List<Long>): List<MentionDto> =
        if (isGroup(kind)) roster.filter { it.userId in memberIds } else roster

    /**
     * How many people a poll's "3 of 5 voted" counts against: the group's
     * members in a group, the family's active roster everywhere else.
     */
    fun pollAudience(kind: String?, memberIds: List<Long>, familyActiveCount: Int): Int =
        if (isGroup(kind)) memberIds.size else familyActiveCount

    // -- What a refusal says ----------------------------------------------------

    /**
     * The words for a failed `POST /chats/groups`.
     *
     * A failure that says nothing about the request — the network, a 5xx, a
     * rate limit — is not a refusal ([ApiResult.isTransient]): it is said
     * as the server being unreachable, never as "couldn't create".
     */
    @StringRes
    fun createFailure(result: ApiResult<*>): Int = when {
        result.isTransient -> R.string.e_unreachable
        result is ApiResult.HttpError && result.code == "too_many_groups" -> R.string.e_too_many_groups
        // The member list is checked before sending, so what is left for
        // `validation` to be about is the name.
        result is ApiResult.HttpError && result.code == "validation" -> R.string.e_group_name_length
        else -> R.string.e_create_group_failed
    }

    /** The words for a failed rename, add, remove, leave or delete. */
    @StringRes
    fun changeFailure(result: ApiResult<*>, renaming: Boolean = false): Int = when {
        result.isTransient -> R.string.e_unreachable
        result is ApiResult.HttpError && result.code == "not_group_admin" -> R.string.e_not_group_admin
        renaming && result is ApiResult.HttpError && result.code == "validation" -> R.string.e_group_name_length
        else -> R.string.e_change_group_failed
    }

    /**
     * The group is not there for this person any more — left, removed from
     * or deleted while they looked at it. Answered `chat_not_found` to
     * anybody not in it, never `not_chat_member`, so it cannot be probed.
     */
    fun isGone(result: ApiResult<*>): Boolean =
        result is ApiResult.HttpError && result.code == "chat_not_found"
}
