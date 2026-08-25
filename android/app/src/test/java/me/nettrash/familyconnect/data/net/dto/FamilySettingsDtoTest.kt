/*
 * FamilySettingsDtoTest.kt
 * Family Connect (Android)
 *
 * The family's language, the assistant-history switch, and birthdays —
 * all three are fields a server may or may not have, so all three are
 * covered by the compatibility rule AvatarDtoTest pins for
 * `avatar_version`: a client must decode a payload from a server that
 * predates the field it is asking about.
 *
 * The other half of this file is the one place in the protocol where a
 * JSON `null` means something a missing key does not: clearing the
 * family's language. Kotlin cannot show the difference in its own types —
 * both are `null` — so the tests read the encoded BODY.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class FamilySettingsDtoTest {

    private val json = Json { ignoreUnknownKeys = true }

    /** The house config — encodeDefaults=false is what omits a key. */
    private val houseJson = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    // -- Decoding ----------------------------------------------------------

    @Test
    fun `a family from a server without either field decodes`() {
        val family = json.decodeFromString<FamilyDto>(
            """{"id": 3, "name": "The Smiths", "join_policy": "open"}""",
        )
        // Unset, and unset is NOT English — the absence has to survive as
        // an absence or the screen cannot tell the two apart.
        assertThat(family.language).isNull()
        // A boolean with a real default: the successor of a server that
        // sends nothing behaves as `true`.
        assertThat(family.aiHistory).isTrue()
    }

    @Test
    fun `a chosen language and a switched-off history both arrive`() {
        val family = json.decodeFromString<FamilyDto>(
            """
            {"id": 3, "name": "The Smiths", "join_policy": "approval",
             "language": "sr-Latn", "ai_history": false}
            """.trimIndent(),
        )
        // The canonical spelling comes back out, script and all, so it can
        // be compared against the client's own list without normalising.
        assertThat(family.language).isEqualTo("sr-Latn")
        assertThat(family.aiHistory).isFalse()
    }

    @Test
    fun `English is decoded as a choice, not as unset`() {
        val chosen = json.decodeFromString<FamilyDto>(
            """{"id": 3, "name": "x", "join_policy": "open", "language": "en"}""",
        )
        val unset = json.decodeFromString<FamilyDto>(
            """{"id": 3, "name": "x", "join_policy": "open"}""",
        )
        assertThat(chosen.language).isEqualTo("en")
        assertThat(unset.language).isNull()
    }

    @Test
    fun `a user and a member decode a birthday when there is one`() {
        val user = json.decodeFromString<UserDto>(
            """
            {"id": 7, "username": "anna", "display_name": "Anna",
             "birthday": {"month": 2, "day": 29}}
            """.trimIndent(),
        )
        val member = json.decodeFromString<MemberDto>(
            """
            {"id": 8, "username": "ben", "display_name": "Ben", "role": "member",
             "birthday": {"month": 3, "day": 14}}
            """.trimIndent(),
        )
        // 29 February, accepted: with no year there is none for it to
        // fail to exist in.
        assertThat(user.birthday).isEqualTo(BirthdayDto(month = 2, day = 29))
        assertThat(member.birthday).isEqualTo(BirthdayDto(month = 3, day = 14))
    }

    @Test
    fun `no birthday key means unset rather than a decode failure`() {
        val user = json.decodeFromString<UserDto>(
            """{"id": 7, "username": "anna", "display_name": "Anna"}""",
        )
        val member = json.decodeFromString<MemberDto>(
            """{"id": 8, "username": "ben", "display_name": "Ben", "role": "member"}""",
        )
        assertThat(user.birthday).isNull()
        assertThat(member.birthday).isNull()
    }

    // -- Encoding: the one place a null is not an omission -------------------

    @Test
    fun `clearing the language sends the key with a literal null`() {
        val body = houseJson.encodeToString(PatchFamilyRequest.language(null))

        // Not `{}`: an omitted key LEAVES the language alone, and this
        // request has to clear it. The difference is invisible in Kotlin
        // and total on the wire.
        assertThat(body).isEqualTo("""{"language":null}""")
    }

    @Test
    fun `choosing a language sends the tag`() {
        val body = houseJson.encodeToString(PatchFamilyRequest.language("ru"))
        assertThat(body).isEqualTo("""{"language":"ru"}""")
    }

    @Test
    fun `each patch carries only its own field`() {
        // Which fields are PRESENT decides what changes, so an untouched
        // one must not appear at all — including `ai_history: false`,
        // which is a value rather than an absence.
        assertThat(houseJson.encodeToString(PatchFamilyRequest.joinPolicy("approval")))
            .isEqualTo("""{"join_policy":"approval"}""")
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiHistory(false)))
            .isEqualTo("""{"ai_history":false}""")
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiHistory(true)))
            .isEqualTo("""{"ai_history":true}""")
    }

    @Test
    fun `a birthday request is two integers and no year`() {
        val body = houseJson.encodeToString(BirthdayRequest(month = 2, day = 29))
        assertThat(body).isEqualTo("""{"month":2,"day":29}""")
    }
}
