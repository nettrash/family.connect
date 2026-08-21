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
}
