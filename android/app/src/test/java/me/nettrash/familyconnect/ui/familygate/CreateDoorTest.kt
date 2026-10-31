/*
 * CreateDoorTest.kt
 * Family Connect (Android)
 *
 * What stands where "Create a family" goes (docs/protocol.md, "Starting a
 * family", "Several families" and "Paid families"): a shut server keeps
 * priority and its own words, the paid rule comes after it, and "Join
 * Another Family…" has no create card at all.
 */

package me.nettrash.familyconnect.ui.familygate

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class CreateDoorTest {

    @Test
    fun `the gate is open with nothing in the way`() {
        assertThat(CreateDoor.of(FamilyGateMode.GATE, registrationEnabled = true, mustPayToStart = false))
            .isEqualTo(CreateDoor.OPEN)
        assertThat(CreateDoor.of(FamilyGateMode.NEW_FAMILY, registrationEnabled = true, mustPayToStart = false))
            .isEqualTo(CreateDoor.OPEN)
    }

    @Test
    fun `a server that charges, to somebody who does not pay, says it cannot be started here`() {
        assertThat(CreateDoor.of(FamilyGateMode.GATE, registrationEnabled = true, mustPayToStart = true))
            .isEqualTo(CreateDoor.CANNOT_START_HERE)
        // "New Family…" stays on the menu and shows the sentence instead of the form.
        assertThat(CreateDoor.of(FamilyGateMode.NEW_FAMILY, registrationEnabled = true, mustPayToStart = true))
            .isEqualTo(CreateDoor.CANNOT_START_HERE)
    }

    /** Nobody is told about paying for a family the server would not take anyway. */
    @Test
    fun `a shut server keeps priority and its own words`() {
        assertThat(CreateDoor.of(FamilyGateMode.GATE, registrationEnabled = false, mustPayToStart = true))
            .isEqualTo(CreateDoor.CLOSED_SERVER)
        assertThat(CreateDoor.of(FamilyGateMode.GATE, registrationEnabled = false, mustPayToStart = false))
            .isEqualTo(CreateDoor.CLOSED_SERVER)
    }

    /**
     * `registration` is about a FIRST family only: "New Family…" is the
     * `member_families` door, which the switcher offers only while it is
     * open — so behind it only the paid rule can apply.
     */
    @Test
    fun `registration does not shut New Family`() {
        assertThat(CreateDoor.of(FamilyGateMode.NEW_FAMILY, registrationEnabled = false, mustPayToStart = false))
            .isEqualTo(CreateDoor.OPEN)
        assertThat(CreateDoor.of(FamilyGateMode.NEW_FAMILY, registrationEnabled = false, mustPayToStart = true))
            .isEqualTo(CreateDoor.CANNOT_START_HERE)
    }

    @Test
    fun `joining another family has no create card`() {
        for (registration in listOf(true, false)) {
            for (pay in listOf(true, false)) {
                assertThat(CreateDoor.of(FamilyGateMode.JOIN_FAMILY, registration, pay)).isEqualTo(CreateDoor.HIDDEN)
            }
        }
    }
}
