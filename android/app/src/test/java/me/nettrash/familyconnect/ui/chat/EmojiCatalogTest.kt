/*
 * EmojiCatalogTest.kt
 * Family Connect (Android)
 *
 * Sanity pins on the embedded full-picker catalog. The catalog is a
 * cross-platform contract (iOS embeds the identical list), so the shape
 * test pins the category names AND their order; the byte test pins the
 * one hard server rule — reactions over 32 UTF-8 bytes are rejected, so
 * nothing the picker can offer may exceed that.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class EmojiCatalogTest {

    @Test
    fun catalogHasTheCanonicalCategoriesInOrder() {
        assertThat(EMOJI_CATALOG.map { it.name }).containsExactly(
            "Smileys",
            "Gestures",
            "Hearts",
            "Animals & Nature",
            "Food & Drink",
            "Activities",
            "Travel & Places",
            "Objects & Symbols",
        ).inOrder()
    }

    @Test
    fun noCategoryIsEmpty() {
        for (category in EMOJI_CATALOG) {
            assertThat(category.name).isNotEmpty()
            assertThat(category.emoji).isNotEmpty()
        }
    }

    @Test
    fun everyEntryIsNonEmpty() {
        for (category in EMOJI_CATALOG) {
            for (emoji in category.emoji) {
                assertThat(emoji).isNotEmpty()
            }
        }
    }

    @Test
    fun entriesAreUniqueWithinEachCategory() {
        // Duplicates within a section would render the same emoji twice
        // and collide on the grid's per-category item keys. (The same
        // emoji MAY appear in two different categories — 💫 and 🎂 do.)
        for (category in EMOJI_CATALOG) {
            val duplicates = category.emoji.groupingBy { it }.eachCount().filterValues { it > 1 }
            assertThat(duplicates).isEmpty()
        }
    }

    @Test
    fun everyEntryFitsTheServersThirtyTwoByteReactionLimit() {
        // The server hard-rejects reactions over 32 UTF-8 bytes — the
        // picker must never offer one it would bounce.
        for (category in EMOJI_CATALOG) {
            for (emoji in category.emoji) {
                val bytes = emoji.toByteArray(Charsets.UTF_8).size
                assertThat(bytes).isAtMost(32)
            }
        }
    }

    @Test
    fun doubleTapReactionIsAQuickReactionWithinTheLimit() {
        assertThat(DOUBLE_TAP_REACTION.toByteArray(Charsets.UTF_8).size).isAtMost(32)
        // Inside the quick set so the capsule shows it selected after
        // a double-tap. iOS pins the same value.
        assertThat(QUICK_REACTIONS).contains(DOUBLE_TAP_REACTION)
    }

    @Test
    fun localizedIndexCoversEveryEmojiInAllNineLanguages() {
        val fireNames = mapOf(
            "en" to "fire", "de" to "Feuer", "es" to "fuego", "fr" to "feu", "ja" to "火",
            "ru" to "огонь", "sr" to "ватра", "sr-Latn" to "vatra", "zh-Hans" to "火焰",
        )
        val catalogue = EMOJI_CATALOG.flatMap { it.emoji }.distinct()
        assertThat(catalogue).hasSize(769)
        for ((tag, fireName) in fireNames) {
            val index = ReactionSearch.index(tag)
            assertThat(index.name("🔥")).isEqualTo(fireName)
            assertThat(index.search(fireName)).contains("🔥")
            for (emoji in catalogue) assertThat(index.name(emoji)).isNotEqualTo(emoji)
        }
        assertThat(ReactionSearch.index("de").search("  FUSSBALL  ")).contains("⚽")
        assertThat(ReactionSearch.index("ja").search("火")).contains("🔥")
    }

    @Test
    fun preparedIndexIsReusedWhenReopeningAndKeepingLocalesSeparate() {
        val russian = ReactionSearch.index("ru")
        val english = ReactionSearch.index("en")
        assertThat(ReactionSearch.cachedIndex("ru")).isSameInstanceAs(russian)
        assertThat(ReactionSearch.index("en")).isSameInstanceAs(english)
        assertThat(russian.name("🔥")).isEqualTo("огонь")
        assertThat(english.name("🔥")).isEqualTo("fire")
        assertThat(russian.search(":ICE:").first()).isEqualTo("🧊")
    }

    @Test
    fun aliasesAndEnglishFallbackRankAheadOfKeywordMatches() {
        val russian = ReactionSearch.index("ru")
        assertThat(russian.search("  :ICE:  ").first()).isEqualTo("🧊")
        val prefixes = russian.search("ic")
        assertThat(prefixes).containsAtLeast("🧊", "😀").inOrder()
        assertThat(russian.search(":LIKE:").first()).isEqualTo("👍")
        val english = ReactionSearch.index("en")
        assertThat(english.search("red_heart").first()).isEqualTo("❤️")
        assertThat(english.search("fire nonexistentkeyword")).isEmpty()
    }

    @Test
    fun exactVariantsReturnCatalogueBytesAndResultsDeduplicateInCatalogueOrder() {
        val index = ReactionSearch.index("en")
        assertThat(index.search(" ❤ ")).containsExactly("❤️")
        assertThat(index.search("❤︎")).isEmpty()
        assertThat(index.search("❤️")).containsExactly("❤️")
        assertThat(index.search("👍🏽")).isEmpty()
        val stars = index.search("star")
        assertThat(stars).containsNoDuplicates()
        // Both names contain the keyword. Neither starts with it.
        assertThat(stars).containsAtLeast("💫", "🌠").inOrder()
    }

    @Test
    fun emptyQueriesRestoreBrowseAndUnmatchedQueriesAreStillSearches() {
        val index = ReactionSearch.index("en")
        for (query in listOf("", "  ", "__::")) {
            assertThat(ReactionSearch.normalize(query)).isEmpty()
            assertThat(index.search(query)).isEmpty()
        }
        val query = "no-such-emoji-8372"
        assertThat(ReactionSearch.normalize(query)).isNotEmpty()
        assertThat(index.search(query)).isEmpty()
    }

    @Test
    fun displayedRecentsCleanUnknownGlyphsAndResolveVariants() {
        assertThat(ReactionSearch.recents(listOf("❤", "🔥", "❤️", "bogus", "👍🏽")))
            .containsExactly("❤️", "🔥").inOrder()
    }
}
