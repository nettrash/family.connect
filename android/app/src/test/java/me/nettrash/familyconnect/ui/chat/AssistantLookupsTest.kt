/*
 * AssistantLookupsTest.kt
 * Family Connect (Android)
 *
 * The decisions behind "Looking things up" on this client
 * (docs/protocol.md): when the lookup question may be asked, what the
 * consent screen offers, what the member's Settings row says, and how the
 * providers are named in a sentence.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class AssistantLookupsTest {

    private val processor = "Microsoft — Azure OpenAI (Sweden Central)"
    private val allThree = listOf("Brave Search", "Open-Meteo", "Wikipedia")

    // -- Providers ---------------------------------------------------------------

    @Test
    fun `absent and empty both name nobody, and blanks are dropped`() {
        assertThat(AssistantLookups.providers(null)).isEmpty()
        assertThat(AssistantLookups.providers(emptyList())).isEmpty()
        assertThat(AssistantLookups.providers(listOf(" ", "", " SearXNG "))).containsExactly("SearXNG")
        assertThat(AssistantLookups.providers(allThree)).containsExactlyElementsIn(allThree).inOrder()
    }

    @Test
    fun `the question is asked only where the assistant and every provider can be named`() {
        assertThat(AssistantLookups.isOffered(processor, allThree)).isTrue()
        assertThat(AssistantLookups.isOffered(processor, listOf("SearXNG"))).isTrue()
        // No source on this server.
        assertThat(AssistantLookups.isOffered(processor, null)).isFalse()
        assertThat(AssistantLookups.isOffered(processor, emptyList())).isFalse()
        // An assistant this client does not offer at all.
        assertThat(AssistantLookups.isOffered(null, allThree)).isFalse()
        assertThat(AssistantLookups.isOffered("  ", allThree)).isFalse()
    }

    @Test
    fun `the owner's switch is drawn only on a server with a source`() {
        assertThat(AssistantLookups.showsOwnerSwitch(allThree)).isTrue()
        assertThat(AssistantLookups.showsOwnerSwitch(null)).isFalse()
        assertThat(AssistantLookups.showsOwnerSwitch(emptyList())).isFalse()
    }

    // -- The consent screen's buttons --------------------------------------------

    @Test
    fun `without lookups the screen is what it always was`() {
        assertThat(AssistantLookups.consentButtons(lookupsOffered = false, assistantAgreedAt = null))
            .isEqualTo(AssistantLookups.ConsentButtons.AGREE)
        assertThat(AssistantLookups.consentButtons(lookupsOffered = false, assistantAgreedAt = "2026-09-19T19:34:43Z"))
            .isEqualTo(AssistantLookups.ConsentButtons.AGREE)
    }

    @Test
    fun `a member who agreed to nothing chooses with or without lookups`() {
        assertThat(AssistantLookups.consentButtons(lookupsOffered = true, assistantAgreedAt = null))
            .isEqualTo(AssistantLookups.ConsentButtons.AGREE_WITH_OR_WITHOUT_LOOKUPS)
        assertThat(AssistantLookups.consentButtons(lookupsOffered = true, assistantAgreedAt = ""))
            .isEqualTo(AssistantLookups.ConsentButtons.AGREE_WITH_OR_WITHOUT_LOOKUPS)
    }

    @Test
    fun `a member who already agreed to the assistant is asked only about lookups`() {
        assertThat(AssistantLookups.consentButtons(lookupsOffered = true, assistantAgreedAt = "2026-09-19T19:34:43Z"))
            .isEqualTo(AssistantLookups.ConsentButtons.AGREE_TO_LOOKUPS)
    }

    // -- Settings ----------------------------------------------------------------

    @Test
    fun `the settings row follows the server, the assistant and the member's own answer`() {
        assertThat(AssistantLookups.settingsRow(processor, null, null))
            .isEqualTo(AssistantLookups.SettingsRow.HIDDEN)
        assertThat(AssistantLookups.settingsRow(null, allThree, "2026-10-03T09:00:00Z"))
            .isEqualTo(AssistantLookups.SettingsRow.HIDDEN)
        assertThat(AssistantLookups.settingsRow(processor, allThree, null))
            .isEqualTo(AssistantLookups.SettingsRow.ALLOW)
        assertThat(AssistantLookups.settingsRow(processor, allThree, " "))
            .isEqualTo(AssistantLookups.SettingsRow.ALLOW)
        assertThat(AssistantLookups.settingsRow(processor, allThree, "2026-10-03T09:00:00Z"))
            .isEqualTo(AssistantLookups.SettingsRow.AGREED)
    }

    // -- Naming the providers ----------------------------------------------------

    private fun join(names: List<String>) = AssistantLookups.joinProviders(
        names,
        two = { a, b -> "$a and $b" },
        three = { a, b, c -> "$a, $b and $c" },
    )

    @Test
    fun `one, two and three providers read as a sentence`() {
        assertThat(join(listOf("SearXNG"))).isEqualTo("SearXNG")
        assertThat(join(listOf("Open-Meteo", "Wikipedia"))).isEqualTo("Open-Meteo and Wikipedia")
        assertThat(join(allThree)).isEqualTo("Brave Search, Open-Meteo and Wikipedia")
    }

    @Test
    fun `the language's own joiners are used, not an English comma`() {
        val japanese = AssistantLookups.joinProviders(
            allThree,
            two = { a, b -> "${a}と$b" },
            three = { a, b, c -> "$a、$b、$c" },
        )
        assertThat(japanese).isEqualTo("Brave Search、Open-Meteo、Wikipedia")
    }

    @Test
    fun `more than three still names every one of them`() {
        assertThat(join(listOf("A", "B", "C", "D"))).isEqualTo("A, B, C and D")
    }

    @Test
    fun `nobody is an empty phrase, and blanks are not named`() {
        assertThat(join(emptyList())).isEmpty()
        assertThat(join(listOf("Open-Meteo", " "))).isEqualTo("Open-Meteo")
    }
}
