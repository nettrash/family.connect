/*
 * CreateDoor.kt
 * Family Connect (Android)
 *
 * What stands where "Create a family" goes, on the gate and behind "New
 * Family…" — a decision rather than a nest of ifs in a screen, so a test
 * can hold its order.
 */

package me.nettrash.familyconnect.ui.familygate

/** What the gate draws in the create card's place. */
enum class CreateDoor {
    /** "Join Another Family…": there is no create card at all. */
    HIDDEN,

    /**
     * The server takes no new families (`family_registration_enabled:
     * false`, docs/protocol.md "Starting a family"): why, and the way to a
     * server of one's own.
     */
    CLOSED_SERVER,

    /**
     * The server charges for a new family and the caller holds no active
     * subscription (docs/protocol.md, "Paid families"): the sentence that a
     * new family cannot be started from this app, and nothing to tap. It
     * names no other app and no store — Google Play's payments policy.
     */
    CANNOT_START_HERE,

    /** The name field and Create, as always. */
    OPEN;

    companion object {
        /**
         * The shut door keeps priority and keeps its own words: nobody is
         * told about paying for a family the server would not take anyway.
         * `registration` is about an account's FIRST family only — "New
         * Family…" is the separate `member_families` door, which the
         * switcher offers only while it is open (docs/protocol.md, "Several
         * families") — so behind it only the paid rule can apply.
         */
        fun of(mode: FamilyGateMode, registrationEnabled: Boolean, mustPayToStart: Boolean): CreateDoor = when {
            mode == FamilyGateMode.JOIN_FAMILY -> HIDDEN
            mode == FamilyGateMode.GATE && !registrationEnabled -> CLOSED_SERVER
            mustPayToStart -> CANNOT_START_HERE
            else -> OPEN
        }
    }
}
