/*
 * MemberCapTest.kt
 * Family Connect (Android)
 *
 * The owner's member cap. A cross-platform contract: MemberCapTests.swift
 * pins the same vectors, so a limit set on the phone reads the same on the
 * Mac and here.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class MemberCapTest {

    @Test
    fun noCapOfYourOwnPointsAtTheOperatorsCeiling() {
        assertThat(MemberCap.state(cap = null, memberCount = 4, ceiling = 50))
            .isEqualTo(MemberCap.State.OpenToCeiling(50))
    }

    @Test
    fun roomToSpareCountsSeats() {
        assertThat(MemberCap.state(cap = 8, memberCount = 4, ceiling = 50))
            .isEqualTo(MemberCap.State.Room(memberCount = 4, cap = 8))
    }

    /**
     * The boundary is the interesting one: a cap EQUAL to the roster is
     * already a freeze, because the cap is read at the join door and the
     * next arrival would exceed it.
     */
    @Test
    fun aCapAtOrBelowTheRosterIsAFreezeNotAnError() {
        assertThat(MemberCap.state(cap = 4, memberCount = 4, ceiling = 50))
            .isEqualTo(MemberCap.State.Frozen(4))
        assertThat(MemberCap.state(cap = 2, memberCount = 6, ceiling = 50))
            .isEqualTo(MemberCap.State.Frozen(6))
        // One more seat is NOT a freeze, so the boundary is real rather
        // than an always-frozen predicate.
        assertThat(MemberCap.state(cap = 5, memberCount = 4, ceiling = 50))
            .isEqualTo(MemberCap.State.Room(memberCount = 4, cap = 5))
    }

    @Test
    fun turningTheLimitOnFreezesTheFamilyWhereItStands() {
        assertThat(MemberCap.seed(memberCount = 4, ceiling = 50)).isEqualTo(4)
    }

    /**
     * An operator may lower the ceiling under a family already larger.
     * Seeding at the roster would then open the control above its bound.
     */
    @Test
    fun theSeedNeverExceedsTheCeiling() {
        assertThat(MemberCap.seed(memberCount = 60, ceiling = 50)).isEqualTo(50)
    }

    @Test
    fun theSeedNeverFallsBelowOne() {
        assertThat(MemberCap.seed(memberCount = 0, ceiling = 50)).isEqualTo(1)
    }

    @Test
    fun aSteppedValueIsHeldInsideTheRange() {
        assertThat(MemberCap.clamp(7, ceiling = 50)).isEqualTo(7)
        assertThat(MemberCap.clamp(0, ceiling = 50)).isEqualTo(1)
        assertThat(MemberCap.clamp(-3, ceiling = 50)).isEqualTo(1)
        assertThat(MemberCap.clamp(99, ceiling = 50)).isEqualTo(50)
        assertThat(MemberCap.clamp(50, ceiling = 50)).isEqualTo(50)
        // A degenerate ceiling still yields a legal cap rather than a
        // range the control would trap on.
        assertThat(MemberCap.clamp(5, ceiling = 0)).isEqualTo(1)
    }
}
