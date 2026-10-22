/*
 * GroupRulesTest.kt
 * Family Connect (Android)
 *
 * The two tables every client agrees on for groups (#107; docs/protocol.md,
 * "Groups"; docs/groups-2026-10-21.md §7): who may do what to a group, and
 * what a group offers beside the family chat, a direct chat and the
 * assistant's own. A row wrong here is a button the server can only refuse,
 * or one missing where it would have said yes.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.util.GroupRules.Action
import org.junit.Test

class GroupRulesTest {

    private companion object {
        const val ADMIN = 7L
        const val MEMBER = 9L
        const val OTHER = 11L
        const val OUTSIDER = 12L
        val MEMBERS = listOf(ADMIN, MEMBER, OTHER)
    }

    private fun may(action: Action, me: Long?, target: Long? = null, kind: String? = "group", admin: Long? = ADMIN) =
        GroupRules.may(action, kind, MEMBERS, admin, me, target)

    // -- Who may do what (D1: the creator is admin) ---------------------------------

    @Test
    fun `the admin may rename, add, remove somebody else, leave and delete`() {
        assertThat(may(Action.RENAME, ADMIN)).isTrue()
        assertThat(may(Action.ADD_MEMBERS, ADMIN)).isTrue()
        assertThat(may(Action.REMOVE_MEMBER, ADMIN, target = MEMBER)).isTrue()
        assertThat(may(Action.LEAVE, ADMIN)).isTrue()
        assertThat(may(Action.DELETE, ADMIN)).isTrue()
    }

    @Test
    fun `any other member may only leave`() {
        assertThat(may(Action.LEAVE, MEMBER)).isTrue()
        assertThat(may(Action.RENAME, MEMBER)).isFalse()
        assertThat(may(Action.ADD_MEMBERS, MEMBER)).isFalse()
        assertThat(may(Action.REMOVE_MEMBER, MEMBER, target = OTHER)).isFalse()
        assertThat(may(Action.DELETE, MEMBER)).isFalse()
    }

    /** Removing yourself is leaving; removing somebody not in it is `validation`. */
    @Test
    fun `the admin removes only somebody else who is in it`() {
        assertThat(may(Action.REMOVE_MEMBER, ADMIN, target = ADMIN)).isFalse()
        assertThat(may(Action.REMOVE_MEMBER, ADMIN, target = OUTSIDER)).isFalse()
        assertThat(may(Action.REMOVE_MEMBER, ADMIN, target = null)).isFalse()
    }

    /** The family owner has no power over a group they are not in (D2). */
    @Test
    fun `nobody outside the group may do anything to it`() {
        for (action in Action.entries) {
            assertWithMessage("$action").that(may(action, OUTSIDER, target = MEMBER)).isFalse()
            assertWithMessage("$action, nobody signed in").that(may(action, null, target = MEMBER)).isFalse()
        }
    }

    /** None of them can rename, empty or delete the family chat, a direct chat or an `ai` chat. */
    @Test
    fun `nothing is done to a chat that is not a group`() {
        for (kind in listOf("family", "direct", "ai", null)) {
            for (action in Action.entries) {
                assertWithMessage("$kind/$action").that(may(action, ADMIN, target = MEMBER, kind = kind)).isFalse()
            }
        }
    }

    /** An admin hand-off moves every admin row with it. */
    @Test
    fun `whoever is admin now holds the admin's rows`() {
        assertThat(may(Action.DELETE, MEMBER, admin = MEMBER)).isTrue()
        assertThat(may(Action.DELETE, ADMIN, admin = MEMBER)).isFalse()
        assertThat(may(Action.LEAVE, ADMIN, admin = MEMBER)).isTrue()
    }

    // -- What a chat offers ----------------------------------------------------------

    private data class Offer(
        val senderNames: Boolean,
        val mentions: Boolean,
        val polls: Boolean,
        val assistantMention: Boolean,
        val calls: Boolean,
        val ticks: Boolean,
        val videoMessages: Boolean,
        val transcriptsDirectRule: Boolean,
    )

    private fun offer(kind: String?) = Offer(
        senderNames = GroupRules.showsSenderNames(kind),
        mentions = GroupRules.offersMemberMentions(kind),
        polls = GroupRules.offersPolls(kind),
        assistantMention = GroupRules.answersAssistantMention(kind),
        calls = GroupRules.offersCalls(kind),
        ticks = GroupRules.drawsSeenTicks(kind),
        videoMessages = GroupRules.offersVideoMessages(kind),
        transcriptsDirectRule = GroupRules.transcriptsFollowDirectRule(kind),
    )

    /** "Inside, it is the family chat with fewer people" — except calls, and transcripts (D4). */
    @Test
    fun `a group offers what the family chat does, without calls or ticks, with the direct transcript rule`() {
        assertThat(offer("group")).isEqualTo(
            Offer(
                senderNames = true, mentions = true, polls = true, assistantMention = true,
                calls = false, ticks = false, videoMessages = true, transcriptsDirectRule = true,
            ),
        )
        assertThat(offer("family")).isEqualTo(
            Offer(
                senderNames = true, mentions = true, polls = true, assistantMention = true,
                calls = false, ticks = false, videoMessages = true, transcriptsDirectRule = false,
            ),
        )
        assertThat(offer("direct")).isEqualTo(
            Offer(
                senderNames = false, mentions = false, polls = false, assistantMention = false,
                calls = true, ticks = true, videoMessages = true, transcriptsDirectRule = true,
            ),
        )
        assertThat(offer("ai")).isEqualTo(
            Offer(
                senderNames = false, mentions = false, polls = false, assistantMention = false,
                calls = false, ticks = false, videoMessages = false, transcriptsDirectRule = false,
            ),
        )
        assertThat(offer(null)).isEqualTo(offer("ai"))
    }

    // -- Names and members -----------------------------------------------------------

    @Test
    fun `a name is trimmed and must then be 1 to 64 characters`() {
        assertThat(GroupRules.validName("  Cousins  ")).isEqualTo("Cousins")
        assertThat(GroupRules.validName("")).isNull()
        assertThat(GroupRules.validName("   \n\t ")).isNull()
        assertThat(GroupRules.validName("a".repeat(64))).isEqualTo("a".repeat(64))
        assertThat(GroupRules.validName("a".repeat(65))).isNull()
    }

    /** The server counts `chars()`: an emoji outside the BMP is one character, not two. */
    @Test
    fun `characters are counted as the server counts them`() {
        val sixtyFourEmoji = "🎉".repeat(64)
        assertThat(sixtyFourEmoji.length).isEqualTo(128)
        assertThat(GroupRules.validName(sixtyFourEmoji)).isEqualTo(sixtyFourEmoji)
        assertThat(GroupRules.validName("🎉".repeat(65))).isNull()
    }

    @Test
    fun `a group is started with anybody active but the creator`() {
        assertThat(GroupRules.startableWith(listOf(ADMIN, MEMBER, OTHER), ADMIN)).containsExactly(MEMBER, OTHER).inOrder()
        assertThat(GroupRules.startableWith(listOf(ADMIN), ADMIN)).isEmpty()
    }

    @Test
    fun `add members offers the active family not already in it`() {
        assertThat(GroupRules.addable(listOf(ADMIN, MEMBER, OTHER, OUTSIDER), MEMBERS)).containsExactly(OUTSIDER)
        assertThat(GroupRules.addable(MEMBERS, MEMBERS)).isEmpty()
    }

    /** "Member mentions name members of THAT GROUP only." */
    @Test
    fun `a mention in a group names its members only`() {
        val roster = listOf(MentionDto(ADMIN, "Anna"), MentionDto(MEMBER, "Ben"), MentionDto(OUTSIDER, "Zoe"))
        assertThat(GroupRules.mentionRoster(roster, "group", listOf(ADMIN, MEMBER)).map { it.userId })
            .containsExactly(ADMIN, MEMBER).inOrder()
        assertThat(GroupRules.mentionRoster(roster, "family", listOf(ADMIN))).isEqualTo(roster)
    }

    /** "4 of 5 voted" counts the group's members. */
    @Test
    fun `a poll in a group counts the group`() {
        assertThat(GroupRules.pollAudience("group", MEMBERS, familyActiveCount = 9)).isEqualTo(3)
        assertThat(GroupRules.pollAudience("family", MEMBERS, familyActiveCount = 9)).isEqualTo(9)
    }

    // -- What a refusal says ---------------------------------------------------------

    @Test
    fun `creating says what the server refused, and never calls a dead network a refusal`() {
        assertThat(GroupRules.createFailure(ApiResult.HttpError(409, "too_many_groups", null))).isEqualTo(R.string.e_too_many_groups)
        assertThat(GroupRules.createFailure(ApiResult.HttpError(400, "validation", null))).isEqualTo(R.string.e_group_name_length)
        assertThat(GroupRules.createFailure(ApiResult.HttpError(409, "not_same_family", null))).isEqualTo(R.string.e_create_group_failed)
        assertThat(GroupRules.createFailure(ApiResult.HttpError(500, "internal", null))).isEqualTo(R.string.e_unreachable)
        assertThat(GroupRules.createFailure(ApiResult.HttpError(429, null, null))).isEqualTo(R.string.e_unreachable)
        assertThat(GroupRules.createFailure(ApiResult.NetworkError(IllegalStateException()))).isEqualTo(R.string.e_unreachable)
    }

    @Test
    fun `changing says when only the admin may, and nothing better otherwise`() {
        assertThat(GroupRules.changeFailure(ApiResult.HttpError(403, "not_group_admin", null))).isEqualTo(R.string.e_not_group_admin)
        assertThat(GroupRules.changeFailure(ApiResult.HttpError(400, "validation", null))).isEqualTo(R.string.e_change_group_failed)
        assertThat(GroupRules.changeFailure(ApiResult.HttpError(400, "validation", null), renaming = true))
            .isEqualTo(R.string.e_group_name_length)
        assertThat(GroupRules.changeFailure(ApiResult.HttpError(404, "chat_not_found", null))).isEqualTo(R.string.e_change_group_failed)
        assertThat(GroupRules.changeFailure(ApiResult.NetworkError(IllegalStateException()))).isEqualTo(R.string.e_unreachable)
    }

    @Test
    fun `only chat not found says the group is gone`() {
        assertThat(GroupRules.isGone(ApiResult.HttpError(404, "chat_not_found", null))).isTrue()
        assertThat(GroupRules.isGone(ApiResult.HttpError(403, "not_group_admin", null))).isFalse()
        assertThat(GroupRules.isGone(ApiResult.HttpError(403, "not_chat_member", null))).isFalse()
        assertThat(GroupRules.isGone(ApiResult.NetworkError(IllegalStateException()))).isFalse()
    }
}
