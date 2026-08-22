/*
 * AvatarDtoTest.kt
 * Family Connect (Android)
 *
 * The compatibility rule from docs/protocol.md, pinned: a client must
 * decode a payload from a server that predates the field it is asking
 * about. `avatar_version` is the newest such field, and a missing one has
 * to read as 0 ("no picture"), not as a decode failure that would take
 * the whole /me response down with it.
 *
 * iOS counterpart: the hand-written UserDTO.init(from:) — Swift's
 * synthesized decoder does NOT fall back to a property default, so the
 * same guarantee there costs a custom initializer.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class AvatarDtoTest {

    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun `a user from a pre-avatars server decodes with version zero`() {
        val user = json.decodeFromString<UserDto>(
            """{"id": 7, "username": "anna", "display_name": "Anna", "created_at": "2026-08-01T10:00:00Z"}""",
        )
        assertThat(user.avatarVersion).isEqualTo(0)
    }

    @Test
    fun `a user with a picture carries its version`() {
        val user = json.decodeFromString<UserDto>(
            """{"id": 7, "username": "anna", "display_name": "Anna", "avatar_version": 12}""",
        )
        assertThat(user.avatarVersion).isEqualTo(12)
    }

    @Test
    fun `members decode the same way`() {
        val without = json.decodeFromString<MemberDto>(
            """{"id": 8, "username": "ben", "display_name": "Ben", "role": "member"}""",
        )
        val with = json.decodeFromString<MemberDto>(
            """{"id": 8, "username": "ben", "display_name": "Ben", "role": "member", "avatar_version": 4}""",
        )
        assertThat(without.avatarVersion).isEqualTo(0)
        assertThat(with.avatarVersion).isEqualTo(4)
    }

    @Test
    fun `the upload response carries the bumped version`() {
        val response = json.decodeFromString<AvatarResponse>(
            """{"user": {"id": 7, "username": "anna", "display_name": "Anna", "avatar_version": 1}}""",
        )
        assertThat(response.user.avatarVersion).isEqualTo(1)
    }
}
