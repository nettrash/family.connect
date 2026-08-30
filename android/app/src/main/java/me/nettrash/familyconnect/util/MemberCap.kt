/*
 * MemberCap.kt
 * Family Connect (Android)
 *
 * The owner's member cap, as arithmetic rather than as a `min(max(...))`
 * spread through a screen.
 *
 * Shared with the phone and the Mac by CONTRACT rather than by code:
 * ios/FamilyConnect/Models/MemberCap.swift holds the same three rules, and
 * MemberCapTest mirrors MemberCapTests.swift vector for vector. An owner
 * who sets a limit on one device and sees a different number on another
 * has been told the setting is unreliable.
 */

package me.nettrash.familyconnect.util

object MemberCap {

    /** What the caption says, which is three different sentences. */
    sealed interface State {
        /** No cap of the owner's own; the operator's ceiling is what binds. */
        data class OpenToCeiling(val ceiling: Int) : State

        /**
         * The cap is at or below the current roster. Legal and deliberate:
         * an owner who inherits a large family must still be able to shut
         * the door, and the cap is read at the join door and never
         * enforced over the room — so NOBODY is removed
         * (docs/protocol.md, `PATCH /families/mine`).
         */
        data class Frozen(val memberCount: Int) : State

        /** Room to spare. */
        data class Room(val memberCount: Int, val cap: Int) : State
    }

    fun state(cap: Int?, memberCount: Int, ceiling: Int): State = when {
        cap == null -> State.OpenToCeiling(ceiling)
        cap <= memberCount -> State.Frozen(memberCount)
        else -> State.Room(memberCount, cap)
    }

    /**
     * The cap to propose when the owner first turns the limit on: freeze
     * the family where it stands, which is what reaching for "limit
     * members" almost always means in the moment.
     */
    fun seed(memberCount: Int, ceiling: Int): Int = clamp(memberCount, ceiling)

    /**
     * A cap the owner typed or stepped to, held inside 1..ceiling.
     *
     * Clamped at BOTH ends. The floor is 1 because the protocol's range
     * starts there and an empty roster would otherwise propose 0; the
     * ceiling because an operator may lower `limits.max_family_members`
     * under a family already larger than the new ceiling, and a control
     * seeded above its own bound opens invalid.
     */
    fun clamp(value: Int, ceiling: Int): Int = value.coerceIn(1, maxOf(ceiling, 1))
}
