/*
 * GroupDtoTest.kt
 * Family Connect (Android)
 *
 * The wire shape of a group (#107; docs/protocol.md, `Chat` and "Groups"):
 * `member_ids` and `admin_user_id` ride only on `kind: "group"`, so every
 * other chat — and every server before groups — decodes with neither; and
 * the three request bodies the group endpoints take, spelled as the
 * protocol spells them.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class GroupDtoTest {

    /** The house config (AppModule.provideJson). */
    private val json = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    @Test
    fun `a group decodes with its members in join order and its admin`() {
        val chat = json.decodeFromString<ChatDto>(
            """{"id": 77, "kind": "group", "title": "Cousins", "peer_user_id": null, "member_ids": [9, 7, 11], "admin_user_id": 9}""",
        )
        assertThat(chat.kind).isEqualTo("group")
        assertThat(chat.title).isEqualTo("Cousins")
        assertThat(chat.peerUserId).isNull()
        assertThat(chat.memberIds).containsExactly(9L, 7L, 11L).inOrder()
        assertThat(chat.adminUserId).isEqualTo(9L)
    }

    @Test
    fun `every other chat decodes with neither field`() {
        for (literal in listOf(
            """{"id": 1, "kind": "family", "title": "The Smiths", "peer_user_id": null}""",
            """{"id": 42, "kind": "direct", "title": "Ben", "peer_user_id": 9}""",
            """{"id": 5, "kind": "ai", "title": "Assistant"}""",
        )) {
            val chat = json.decodeFromString<ChatDto>(literal)
            assertThat(chat.memberIds).isNull()
            assertThat(chat.adminUserId).isNull()
        }
    }

    @Test
    fun `the chat list carries a group like any chat`() {
        val response = json.decodeFromString<ChatsResponse>(
            """{"chats": [{"chat": {"id": 77, "kind": "group", "title": "Cousins", "member_ids": [7, 9], "admin_user_id": 7}, "last_message": null, "unread_count": 0, "last_read_message_id": 0}]}""",
        )
        assertThat(response.chats.single().chat.memberIds).containsExactly(7L, 9L).inOrder()
    }

    @Test
    fun `the request bodies are spelled as the protocol spells them`() {
        assertThat(json.parseToJsonElement(json.encodeToString(CreateGroupRequest("Cousins", listOf(9, 11)))))
            .isEqualTo(json.parseToJsonElement("""{"title": "Cousins", "member_ids": [9, 11]}"""))
        assertThat(json.parseToJsonElement(json.encodeToString(RenameGroupRequest("Cousins 2026"))))
            .isEqualTo(json.parseToJsonElement("""{"title": "Cousins 2026"}"""))
        assertThat(json.parseToJsonElement(json.encodeToString(AddGroupMembersRequest(listOf(12)))))
            .isEqualTo(json.parseToJsonElement("""{"user_ids": [12]}"""))
    }
}
