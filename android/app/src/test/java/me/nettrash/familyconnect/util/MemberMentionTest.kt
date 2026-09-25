/*
 * MemberMentionTest.kt
 * Family Connect (Android) — tests
 *
 * The member-mention grammar (docs/protocol.md, "Mentioning a member"),
 * pinned by value against the server's `names_member` and iOS's
 * MemberMentions: the same bodies, the same names, the same answers.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.MentionDto
import org.junit.Test

class MemberMentionTest {

    @Test
    fun `a member is named only by the whole name at a boundary`() {
        assertThat(MemberMention.names("@Anna are you in?", "Anna")).isTrue()
        assertThat(MemberMention.names("hey @Anna, dinner?", "Anna")).isTrue()
        assertThat(MemberMention.names("(@Anna)", "Anna")).isTrue()
        assertThat(MemberMention.names("@Uncle Bob is here", "Uncle Bob")).isTrue()
        assertThat(MemberMention.names("@Анна привет", "Анна")).isTrue()
        assertThat(MemberMention.names("@Анна", "Анна")).isTrue()
        assertThat(MemberMention.names("@Annabel", "Anna")).isFalse()
        assertThat(MemberMention.names("@Ann", "Anna")).isFalse()
        assertThat(MemberMention.names("@anna", "Anna")).isFalse()
        assertThat(MemberMention.names("mail@Anna", "Anna")).isFalse()
        assertThat(MemberMention.names("Anna", "Anna")).isFalse()
        assertThat(MemberMention.names("@Anna", "")).isFalse()
        assertThat(MemberMention.names("@Annabel and @Anna", "Anna")).isTrue()
        val body = "@Anna and @Anna"
        assertThat(MemberMention.ranges(body, "Anna").map { body.substring(it) })
            .containsExactly("@Anna", "@Anna").inOrder()
    }

    @Test
    fun `resolution is longest name first, each member once, in order of appearance`() {
        val bob = MentionDto(1, "Bob")
        val uncle = MentionDto(2, "Uncle Bob")
        val anna = MentionDto(3, "Anna")
        val roster = listOf(bob, uncle, anna)
        assertThat(MemberMention.resolve("@Anna and @Uncle Bob: 7?", roster)).containsExactly(anna, uncle).inOrder()
        assertThat(MemberMention.resolve("@Uncle Bob and @Anna", roster)).containsExactly(uncle, anna).inOrder()
        assertThat(MemberMention.resolve("@Bob @Bob", roster)).containsExactly(bob)
        assertThat(MemberMention.resolve("no one here", roster)).isEmpty()
        assertThat(MemberMention.resolve("@Bobby", roster)).isEmpty()
    }

    /**
     * The grammar is the SERVER's — including where a combining mark, a ZWJ
     * sequence or a variation selector follows the token. Kotlin reads
     * UTF-16 units and the server reads bytes; both call those boundaries,
     * and the two must not part company over them.
     */
    @Test
    fun `a mention the server accepts is a mention here, mid-cluster or not`() {
        assertThat(MemberMention.names("@Anna\u0301 are you in?", "Anna")).isTrue()
        assertThat(MemberMention.names("@Anna\u200D\uD83D\uDC69 hi", "Anna")).isTrue()
        assertThat(MemberMention.names("@Anna\uFE0F", "Anna")).isTrue()
        assertThat(MemberMention.names("@Anna, dinner?", "Anna")).isTrue()
        assertThat(MemberMention.names("@Annabel", "Anna")).isFalse()
        // The boundary class is ASCII alphanumerics and `_`, and NOTHING
        // else — a Unicode-aware "is this a letter" would call Ж a letter
        // and end the token differently from the server, which reads bytes.
        assertThat(MemberMention.names("@AnnaЖ", "Anna")).isTrue()
        assertThat(MemberMention.names("@Anna文", "Anna")).isTrue()
        assertThat(MemberMention.names("@Annab", "Anna")).isFalse()
        assertThat(MemberMention.names("@Anna9", "Anna")).isFalse()
        assertThat(MemberMention.names("@Anna_", "Anna")).isFalse()
    }

    /**
     * Two members called Anna and one `@Anna`: the token can only name one,
     * and it must be the SAME one on every platform. Roster order is not
     * shared between the ports; the id is.
     */
    @Test
    fun `equal names are broken by the lower id, not by roster order`() {
        val annaHigh = MentionDto(12, "Anna")
        val annaLow = MentionDto(3, "Anna")
        assertThat(MemberMention.resolve("@Anna?", listOf(annaHigh, annaLow))).containsExactly(annaLow)
        assertThat(MemberMention.resolve("@Anna?", listOf(annaLow, annaHigh))).containsExactly(annaLow)
        assertThat(MemberMention.tokens("@Anna?", listOf(annaHigh, annaLow)).map { it.second.userId })
            .containsExactly(3L)
    }

    /**
     * A name that is the start of another's: `@Anna Lee` ends `@Anna` at a
     * space, which is a boundary — only the claim keeps Anna out of it.
     */
    @Test
    fun `a token claimed by the longer name is not the shorter one's too`() {
        val anna = MentionDto(3, "Anna")
        val annaLee = MentionDto(4, "Anna Lee")
        val roster = listOf(anna, annaLee)
        assertThat(MemberMention.resolve("@Anna Lee is here", roster)).containsExactly(annaLee)
        assertThat(MemberMention.resolve("@Anna, is @Anna Lee coming?", roster)).containsExactly(anna, annaLee).inOrder()
        assertThat(MemberMention.resolve("@Anna Lee and @Anna", roster)).containsExactly(annaLee, anna).inOrder()
        // And the bubble marks the tokens the same way, whatever order the
        // sender named them in.
        val text = "@Anna, is @Anna Lee coming?"
        val marked = MemberMention.tokens(text, listOf(anna, annaLee)).map { (range, m) -> text.substring(range) to m.userId }
        assertThat(marked).containsExactly("@Anna" to 3L, "@Anna Lee" to 4L).inOrder()
    }

    @Test
    fun `the query is the trailing at token, and accepting rewrites it`() {
        assertThat(MemberMention.query("hey @An")).isEqualTo("An")
        assertThat(MemberMention.query("@")).isEqualTo("")
        assertThat(MemberMention.query("hey @Uncle ")).isEqualTo("Uncle ")
        assertThat(MemberMention.query("mail@x")).isNull()
        assertThat(MemberMention.query("@Anna\nnext")).isNull()
        assertThat(MemberMention.query("no at")).isNull()
        assertThat(MemberMention.accept("hey @An", "Anna")).isEqualTo("hey @Anna ")
        assertThat(MemberMention.accept("@", "Uncle Bob")).isEqualTo("@Uncle Bob ")
    }

    @Test
    fun `candidates leave out the reader, the blocked and non-matches`() {
        val roster = listOf(MentionDto(7, "Me"), MentionDto(8, "Anna"), MentionDto(9, "Andy"), MentionDto(10, "Bob"))
        assertThat(MemberMention.candidates(roster, "an", setOf(7, 9))).containsExactly(roster[1])
        assertThat(MemberMention.candidates(roster, "", setOf(7))).hasSize(3)
        assertThat(MemberMention.candidates(roster, "zz", emptySet())).isEmpty()
    }

    @Test
    fun `the private scheme round-trips a member id and nothing else`() {
        assertThat(MemberMention.userIdFrom(MemberMention.url(42))).isEqualTo(42L)
        assertThat(MemberMention.userIdFrom("https://example.com/42")).isNull()
    }
}
