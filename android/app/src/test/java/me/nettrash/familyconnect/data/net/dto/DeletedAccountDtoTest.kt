/*
 * DeletedAccountDtoTest.kt
 * Family Connect (Android)
 *
 * The wire shape of a deleted account (docs/protocol.md, "Deleting an
 * account"). Three things have to hold at once, and each of them is a
 * compatibility rule the client would otherwise get wrong in a way that
 * only shows up against a real server:
 *
 *   - `deleted` is ABSENT when false — never `false`, never null — so a
 *     live member from any server decodes with the flag off.
 *   - `role` is OPTIONAL, because a tombstone holds none. A required
 *     field here would make every `former_members` payload undecodable,
 *     which is a blank family roster, not a missing name.
 *   - `former_members` is ABSENT when there are none, so the default has
 *     to be the empty list rather than a missing key that throws.
 *
 * Plus the one thing sent OUT: `POST /me/delete` carries the password
 * and nothing else.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class DeletedAccountDtoTest {

    private val json = Json { ignoreUnknownKeys = true }

    /** The house config — encodeDefaults=false is what omits a key. */
    private val houseJson = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    // -- The flag ------------------------------------------------------------

    @Test
    fun `a live member carries no deleted flag and reads as alive`() {
        val member = json.decodeFromString<MemberDto>(
            """{"id": 7, "username": "anna", "display_name": "Anna",
                "role": "owner", "avatar_version": 3}""",
        )
        assertThat(member.deleted).isNull()
        assertThat(member.isDeleted).isFalse()
        assertThat(member.role).isEqualTo("owner")
    }

    @Test
    fun `a tombstone decodes without a role`() {
        val member = json.decodeFromString<MemberDto>(
            """{"id": 11, "username": "junior", "display_name": "Deleted account",
                "avatar_version": 0, "deleted": true}""",
        )
        assertThat(member.isDeleted).isTrue()
        // No role, no picture, no birthday — everything that identified
        // the account is gone; the id and a placeholder name remain.
        assertThat(member.role).isNull()
        assertThat(member.avatarVersion).isEqualTo(0)
        assertThat(member.birthday).isNull()
    }

    @Test
    fun `a deleted user decodes the same way`() {
        val user = json.decodeFromString<UserDto>(
            """{"id": 11, "username": "junior", "display_name": "Deleted account",
                "created_at": "2026-01-01T00:00:00Z", "avatar_version": 0, "deleted": true}""",
        )
        assertThat(user.isDeleted).isTrue()

        val alive = json.decodeFromString<UserDto>(
            """{"id": 7, "username": "anna", "display_name": "Anna",
                "created_at": "2026-01-01T00:00:00Z", "avatar_version": 3}""",
        )
        assertThat(alive.isDeleted).isFalse()
    }

    // -- former_members --------------------------------------------------------

    @Test
    fun `a family with nobody deleted omits former_members entirely`() {
        val mine = json.decodeFromString<FamilyMineResponse>(
            """
            {"family": {"id": 3, "name": "The Smiths", "join_policy": "open"},
             "members": [{"id": 7, "username": "anna", "display_name": "Anna",
                          "role": "owner", "avatar_version": 3}]}
            """.trimIndent(),
        )
        // A missing key is the normal case, so it cannot throw — and it
        // has to arrive as "nobody", not as null to be handled at every
        // call site.
        assertThat(mine.formerMembers).isEmpty()
    }

    @Test
    fun `former members arrive alongside the live roster and are not in it`() {
        val mine = json.decodeFromString<FamilyMineResponse>(
            """
            {"family": {"id": 3, "name": "The Smiths", "join_policy": "open"},
             "members": [{"id": 7, "username": "anna", "display_name": "Anna",
                          "role": "owner", "avatar_version": 3}],
             "former_members": [{"id": 11, "username": "junior",
                                 "display_name": "Deleted account",
                                 "avatar_version": 0, "deleted": true}],
             "max_board_seq": 88}
            """.trimIndent(),
        )
        assertThat(mine.members.map { it.id }).containsExactly(7L)
        assertThat(mine.members.single().isDeleted).isFalse()
        assertThat(mine.formerMembers.map { it.id }).containsExactly(11L)
        assertThat(mine.formerMembers.single().isDeleted).isTrue()
        assertThat(mine.formerMembers.single().role).isNull()
    }

    // -- The request -------------------------------------------------------------

    @Test
    fun `the delete request carries the password and nothing else`() {
        val body = houseJson.encodeToString(DeleteAccountRequest(password = "hunter22"))
        assertThat(houseJson.parseToJsonElement(body))
            .isEqualTo(houseJson.parseToJsonElement("""{"password": "hunter22"}"""))
    }
}
