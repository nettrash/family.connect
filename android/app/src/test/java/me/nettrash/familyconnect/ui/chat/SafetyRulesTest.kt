package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/**
 * WHICH SAFETY ROW A BUBBLE GETS (docs/protocol.md, "Reporting a member" and
 * "Reporting the assistant").
 *
 * The exclusion is the point: a member report names somebody in your family
 * and the OWNER reads it, an assistant report names a reply from an account
 * that belongs to no family and the OPERATOR reads it, and each endpoint
 * refuses the other's subject. A menu that offered the wrong row would fail
 * visibly on a safety screen — the one screen in this product whose design is
 * that refusals never show.
 */
class SafetyRulesTest {

    private val me = 7L
    private val anna = 11L
    private val assistant = 1L

    @Test
    fun `a member's acked message is the member's report and not the assistant's`() {
        assertThat(
            SafetyRules.canModerateMember(anna, me, assistant, isAiChat = false, hasServerId = true),
        ).isTrue()
        assertThat(
            SafetyRules.canReportAssistant(anna, me, assistant, isAiChat = false, hasServerId = true),
        ).isFalse()
    }

    @Test
    fun `an assistant reply is the assistant's report and never moderated as a member`() {
        // By its account, in the family chat.
        assertThat(
            SafetyRules.canReportAssistant(assistant, me, assistant, isAiChat = false, hasServerId = true),
        ).isTrue()
        assertThat(
            SafetyRules.canModerateMember(assistant, me, assistant, isAiChat = false, hasServerId = true),
        ).isFalse()
        // And by the chat it speaks in, for a client that has not read the
        // assistant's id yet.
        assertThat(
            SafetyRules.canReportAssistant(999L, me, null, isAiChat = true, hasServerId = true),
        ).isTrue()
        assertThat(
            SafetyRules.canModerateMember(999L, me, null, isAiChat = true, hasServerId = true),
        ).isFalse()
    }

    @Test
    fun `nothing is reportable before the server numbers it, or when it is your own`() {
        for (aiChat in listOf(true, false)) {
            assertThat(
                SafetyRules.canModerateMember(anna, me, assistant, aiChat, hasServerId = false),
            ).isFalse()
            assertThat(
                SafetyRules.canReportAssistant(assistant, me, assistant, aiChat, hasServerId = false),
            ).isFalse()
            assertThat(
                SafetyRules.canModerateMember(me, me, assistant, aiChat, hasServerId = true),
            ).isFalse()
            assertThat(
                SafetyRules.canReportAssistant(me, me, assistant, aiChat, hasServerId = true),
            ).isFalse()
        }
    }

    /** A signed-out client knows nobody: neither row applies. */
    @Test
    fun `without a reader there is no safety row at all`() {
        assertThat(
            SafetyRules.canModerateMember(anna, null, assistant, isAiChat = false, hasServerId = true),
        ).isFalse()
        assertThat(
            SafetyRules.canReportAssistant(assistant, null, assistant, isAiChat = true, hasServerId = true),
        ).isFalse()
    }

    /** Asserted over every bubble a chat can hold, not trusted to a reading. */
    @Test
    fun `no bubble is ever both reports`() {
        for (sender in listOf(me, anna, assistant, 999L)) {
            for (aiChat in listOf(true, false)) {
                for (hasServerId in listOf(true, false)) {
                    for (assistantId in listOf(assistant, null)) {
                        val member =
                            SafetyRules.canModerateMember(sender, me, assistantId, aiChat, hasServerId)
                        val model =
                            SafetyRules.canReportAssistant(sender, me, assistantId, aiChat, hasServerId)
                        assertThat(member && model).isFalse()
                    }
                }
            }
        }
    }
}
