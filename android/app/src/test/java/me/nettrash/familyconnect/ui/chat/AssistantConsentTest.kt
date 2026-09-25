package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/**
 * Nothing a member writes reaches the model before that member has said
 * yes (docs/protocol.md, "Consenting to the assistant").
 *
 * The vectors are the ones `AssistantConsentTests.swift`,
 * `fc_text::assistant_consent` and the server's own `model_surface` carry:
 * a disagreement between them is either a message refused after it was
 * typed or one sent having asked nothing.
 */
class AssistantConsentTest {

    private val processor = "Microsoft — Azure OpenAI"

    @Test
    fun `the assistant's own chat always reaches the model`() {
        for (body in listOf("hello", "", "/draw a cat", "no mention here")) {
            assertThat(AssistantConsent.reachesTheModel("ai", body)).isTrue()
        }
    }

    @Test
    fun `the family chat reaches it only on a mention`() {
        assertThat(AssistantConsent.reachesTheModel("family", "@ai when is dinner?")).isTrue()
        assertThat(AssistantConsent.reachesTheModel("family", "hey @AI")).isTrue()
        // A picture request in the family chat IS a mention; a bare one
        // asks nobody.
        assertThat(AssistantConsent.reachesTheModel("family", "@ai /draw a cat")).isTrue()
        assertThat(AssistantConsent.reachesTheModel("family", "/draw a cat")).isFalse()
        assertThat(AssistantConsent.reachesTheModel("family", "dinner at 7?")).isFalse()
        assertThat(AssistantConsent.reachesTheModel("family", "write to anna@ai.example")).isFalse()
        assertThat(AssistantConsent.reachesTheModel("family", "@aiden said so")).isFalse()
    }

    @Test
    fun `nowhere else reaches it`() {
        for (kind in listOf("direct", "unknown", null)) {
            assertThat(AssistantConsent.reachesTheModel(kind, "@ai hello")).isFalse()
        }
    }

    @Test
    fun `asked once and not again`() {
        assertThat(AssistantConsent.isRequired("ai", "hello", processor, null)).isTrue()
        assertThat(AssistantConsent.isRequired("ai", "hello", processor, "2026-09-19T19:34:43Z"))
            .isFalse()
        assertThat(AssistantConsent.isRequired("family", "dinner at 7?", processor, null)).isFalse()
        assertThat(AssistantConsent.isRequired("direct", "@ai hello", processor, null)).isFalse()
    }

    @Test
    fun `a server that names nobody offers no assistant`() {
        assertThat(AssistantConsent.isAvailable(null)).isFalse()
        assertThat(AssistantConsent.isAvailable("")).isFalse()
        assertThat(AssistantConsent.isAvailable("   ")).isFalse()
        assertThat(AssistantConsent.isAvailable(processor)).isTrue()
        assertThat(AssistantConsent.isRequired("ai", "hello", null, null)).isFalse()
    }

    /**
     * A server that HAS an assistant but names nobody: consent cannot be
     * asked for, so the message is held back rather than sent.
     */
    @Test
    fun `an unnamed assistant withholds the message`() {
        assertThat(
            AssistantConsent.isWithheldFromAnUnnamedAssistant("ai", "hello", true, null),
        ).isTrue()
        assertThat(
            AssistantConsent.isWithheldFromAnUnnamedAssistant("family", "@ai hi", true, ""),
        ).isTrue()
        assertThat(
            AssistantConsent.isWithheldFromAnUnnamedAssistant("ai", "hello", true, processor),
        ).isFalse()
    }

    /** And the case that must NOT be swallowed. */
    @Test
    fun `where there is no assistant at all the words are just words`() {
        assertThat(
            AssistantConsent.isWithheldFromAnUnnamedAssistant("family", "@ai hi", false, null),
        ).isFalse()
    }
}
