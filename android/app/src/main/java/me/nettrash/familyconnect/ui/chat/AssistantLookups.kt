//
//  AssistantLookups.kt
//  FamilyConnect
//
//  The decisions behind "Looking things up" (docs/protocol.md, "Looking
//  things up", and "Consenting to the assistant", amended 2026-10-03):
//  when the lookup question may be asked at all, which buttons the consent
//  screen offers, what the member's Settings row says, and how the
//  providers are named in a sentence.
//
//  Arithmetic, kept free of Compose and Android like [AssistantConsent],
//  so a plain JUnit test can pin it.
//

package me.nettrash.familyconnect.ui.chat

object AssistantLookups {

    /**
     * The providers the server named (`assistant.lookups`), trimmed and
     * without blanks, in the server's order. Absent and `[]` are the same
     * answer — nobody to name — so both come out empty.
     */
    fun providers(named: List<String>?): List<String> =
        named.orEmpty().map { it.trim() }.filter { it.isNotEmpty() }

    /**
     * May this client ask the lookup question at all?
     *
     * Only where it offers the assistant (a named `processor`, see
     * [AssistantConsent.isAvailable]) AND can name every provider a query
     * would go to: "a client that cannot name the providers does not ask"
     * (docs/protocol.md, "Consenting to the assistant").
     */
    fun isOffered(processor: String?, lookups: List<String>?): Boolean =
        AssistantConsent.isAvailable(processor) && providers(lookups).isNotEmpty()

    /**
     * Is the owner's `ai_lookups` switch drawn? Only when the server has a
     * source: the switch does nothing on a server whose `assistant.lookups`
     * is absent, and its footnote could name nobody.
     */
    fun showsOwnerSwitch(lookups: List<String>?): Boolean = providers(lookups).isNotEmpty()

    /** The buttons under the consent screen. */
    enum class ConsentButtons {
        /** No lookup part: the screen as it always was — "I Agree" / "Not Now". */
        AGREE,

        /**
         * The member has agreed to nothing yet and lookups are offered:
         * "Agree With Lookups" (both consents), "Agree Without Lookups"
         * (the assistant only) and "Not Now".
         */
        AGREE_WITH_OR_WITHOUT_LOOKUPS,

        /**
         * The member already agreed to the assistant and is asked about
         * lookups alone: "I Agree" grants the lookup consent, "Not Now".
         */
        AGREE_TO_LOOKUPS,
    }

    /**
     * Which buttons the consent screen draws. [lookupsOffered] is
     * [isOffered] AND the caller wired the second agreement; a screen that
     * cannot record it shows no lookup part at all rather than a button
     * that does nothing.
     */
    fun consentButtons(lookupsOffered: Boolean, assistantAgreedAt: String?): ConsentButtons = when {
        !lookupsOffered -> ConsentButtons.AGREE
        assistantAgreedAt.isNullOrBlank() -> ConsentButtons.AGREE_WITH_OR_WITHOUT_LOOKUPS
        else -> ConsentButtons.AGREE_TO_LOOKUPS
    }

    /** The member's lookup row in Settings. */
    enum class SettingsRow {
        /** Lookups are not offered here: no row at all. */
        HIDDEN,

        /** Not agreed: "Review and Allow Lookups…", which opens the consent screen. */
        ALLOW,

        /** Agreed: "Agreed", and "Stop Lookups" under it. */
        AGREED,
    }

    fun settingsRow(processor: String?, lookups: List<String>?, lookupAgreedAt: String?): SettingsRow = when {
        !isOffered(processor, lookups) -> SettingsRow.HIDDEN
        lookupAgreedAt.isNullOrBlank() -> SettingsRow.ALLOW
        else -> SettingsRow.AGREED
    }

    /**
     * The providers as one phrase for a sentence's `%1$s`: one on its own,
     * two through [two] ("%1$s and %2$s"), three through [three]
     * ("%1$s, %2$s and %3$s") — the translated joiners, so Japanese and
     * Chinese get 、 and と / 和 rather than an English comma.
     *
     * The server names at most three today (web search, weather,
     * Wikipedia). More would still be named, every one of them: all but
     * the last two are joined with ", " into the first slot. Empty gives
     * "" — but no caller asks then, because [isOffered] is false.
     */
    fun joinProviders(
        names: List<String>,
        two: (String, String) -> String,
        three: (String, String, String) -> String,
    ): String {
        val clean = providers(names)
        return when (clean.size) {
            0 -> ""
            1 -> clean[0]
            2 -> two(clean[0], clean[1])
            3 -> three(clean[0], clean[1], clean[2])
            else -> three(
                clean.subList(0, clean.size - 2).joinToString(", "),
                clean[clean.size - 2],
                clean[clean.size - 1],
            )
        }
    }
}
