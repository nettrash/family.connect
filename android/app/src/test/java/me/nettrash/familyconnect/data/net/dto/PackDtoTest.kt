/*
 * PackDtoTest.kt
 * Family Connect (Android)
 *
 * The sticker pack on the wire (docs/protocol.md, "Sticker pack"), with the
 * literals transcribed from the protocol rather than derived from the
 * types — the same discipline as WsFrameSerdeTest.
 *
 * What is pinned here is mostly ABSENCE: `sticker` is present only when
 * true and never false; a tombstone carries nothing but its id, its flag
 * and its seq; a family read from a server that predates packs has no
 * ceilings; and an ordinary send stays byte-identical to what it was.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.net.ws.parseServerFrame
import org.junit.Test

class PackDtoTest {

    // Same configuration as AppModule.provideJson.
    private val json = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

    @Test
    fun `a live pack item decodes with its picture and its label`() {
        val item = json.decodeFromString<PackItemDto>(
            """
            {"id": 5, "added_by": 7,
             "attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 40211,
                            "width": 512, "height": 512, "has_preview": false},
             "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12, "label": "party cat"}
            """.trimIndent(),
        )

        assertThat(item.isTombstone).isFalse()
        assertThat(item.addedBy).isEqualTo(7)
        assertThat(item.packSeq).isEqualTo(12)
        assertThat(item.label).isEqualTo("party cat")
        assertThat(item.attachment?.mime).isEqualTo("image/webp")
        // The flag is a message's; a pack item's picture never carries it.
        assertThat(item.attachment?.sticker).isNull()
        assertThat(item.attachment?.isSticker).isFalse()
    }

    @Test
    fun `a label is absent rather than empty`() {
        val item = json.decodeFromString<PackItemDto>(
            """{"id": 5, "added_by": 7, "attachment": {"id": 71, "kind": "photo", "mime": "image/png", "size": 9},
                "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12}""",
        )

        assertThat(item.label).isNull()
    }

    @Test
    fun `a tombstone is its id, its flag and its seq`() {
        val item = json.decodeFromString<PackItemDto>("""{"id": 5, "deleted": true, "pack_seq": 14}""")

        assertThat(item.isTombstone).isTrue()
        assertThat(item.packSeq).isEqualTo(14)
        assertThat(item.addedBy).isNull()
        assertThat(item.attachment).isNull()
        assertThat(item.label).isNull()
    }

    @Test
    fun `the whole pack and the change feed decode`() {
        val pack = json.decodeFromString<PackResponse>(
            """{"items": [{"id": 5, "added_by": 7,
                 "attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 9},
                 "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12}], "max_pack_seq": 14}""",
        )
        assertThat(pack.items.single().id).isEqualTo(5)
        assertThat(pack.maxPackSeq).isEqualTo(14)

        // An untouched pack: no items and a mark of zero.
        val untouched = json.decodeFromString<PackResponse>("""{"items": [], "max_pack_seq": 0}""")
        assertThat(untouched.items).isEmpty()
        assertThat(untouched.maxPackSeq).isEqualTo(0)

        val changes = json.decodeFromString<PackChangesResponse>(
            """{"items": [{"id": 5, "deleted": true, "pack_seq": 14}]}""",
        )
        assertThat(changes.items.single().isTombstone).isTrue()
    }

    @Test
    fun `the claim carries the attachment and omits an absent label`() {
        assertThat(json.encodeToString(AddPackItemRequest.serializer(), AddPackItemRequest(71, "party cat")))
            .isEqualTo("""{"attachment_id":71,"label":"party cat"}""")
        assertThat(json.encodeToString(AddPackItemRequest.serializer(), AddPackItemRequest(71)))
            .isEqualTo("""{"attachment_id":71}""")
    }

    // -- The flag on an attachment -----------------------------------------------

    @Test
    fun `sticker is true when present and absent otherwise`() {
        val sent = json.decodeFromString<AttachmentDto>(
            """{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40211, "sticker": true}""",
        )
        assertThat(sent.sticker).isTrue()
        assertThat(sent.isSticker).isTrue()

        val photo = json.decodeFromString<AttachmentDto>(
            """{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40211}""",
        )
        assertThat(photo.sticker).isNull()
        assertThat(photo.isSticker).isFalse()
    }

    @Test
    fun `the flag survives the store and an ordinary attachment gains no key`() {
        val sticker = AttachmentDto(id = 90, kind = "photo", mime = "image/webp", size = 1, sticker = true)
        val photo = AttachmentDto(id = 91, kind = "photo", mime = "image/jpeg", size = 1)

        // What a message row keeps — the JSON column is how a cached
        // sticker is still a sticker after a relaunch.
        val stored = AttachmentsCodec.encode(listOf(sticker, photo))

        assertThat(AttachmentsCodec.decode(stored)).containsExactly(sticker, photo).inOrder()
        // Never `"sticker": false`: absent is the only other spelling.
        assertThat(stored).contains(""""sticker":true""")
        assertThat(stored).doesNotContain("false")
    }

    @Test
    fun `a sticker flag on something that is not a picture is not a sticker`() {
        val odd = AttachmentDto(id = 90, kind = "file", mime = "application/pdf", size = 1, sticker = true)

        assertThat(odd.isSticker).isFalse()
    }

    @Test
    fun `a message carries the flag in both spellings of its attachment`() {
        val message = json.decodeFromString<MessageDto>(
            """
            {"id": 1400, "chat_id": 42, "sender_id": 7, "client_msg_id": "u1", "body": "",
             "created_at": "2026-09-13T10:00:00Z",
             "attachment": {"id": 90, "kind": "photo", "mime": "image/webp", "size": 40211, "sticker": true},
             "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40211, "sticker": true}]}
            """.trimIndent(),
        )

        assertThat(message.resolvedAttachments.single().isSticker).isTrue()
    }

    // -- The family read ------------------------------------------------------------

    @Test
    fun `a family read carries the pack's mark and its two ceilings`() {
        val mine = json.decodeFromString<FamilyMineResponse>(
            """{"family": {"id": 3, "name": "The Smiths", "join_policy": "open"}, "members": [],
                "max_pack_seq": 14, "max_pack_items": 200, "max_pack_item_bytes": 524288}""",
        )

        assertThat(mine.maxPackSeq).isEqualTo(14)
        assertThat(mine.maxPackItems).isEqualTo(200)
        assertThat(mine.maxPackItemBytes).isEqualTo(524288)
    }

    @Test
    fun `a server that predates packs says nothing, and that is the signal`() {
        val mine = json.decodeFromString<FamilyMineResponse>(
            """{"family": {"id": 3, "name": "The Smiths", "join_policy": "open"}, "members": []}""",
        )

        assertThat(mine.maxPackItems).isNull()
        assertThat(mine.maxPackItemBytes).isNull()
        assertThat(mine.maxPackSeq).isNull()
    }

    @Test
    fun `an untouched pack has ceilings and no mark`() {
        val mine = json.decodeFromString<FamilyMineResponse>(
            """{"family": {"id": 3, "name": "The Smiths", "join_policy": "open"}, "members": [],
                "max_pack_items": 200, "max_pack_item_bytes": 524288}""",
        )

        assertThat(mine.maxPackItems).isEqualTo(200)
        assertThat(mine.maxPackSeq).isNull()
    }

    // -- Sending ----------------------------------------------------------------------

    @Test
    fun `a sticker send is the protocol's request`() {
        val encoded = json.encodeToString(
            SendMessageRequest.serializer(),
            SendMessageRequest("u1", "", attachmentIds = listOf(90), sticker = true),
        )

        assertThat(json.parseToJsonElement(encoded)).isEqualTo(
            json.parseToJsonElement(
                """{"client_msg_id": "u1", "body": "", "attachment_ids": [90], "sticker": true}""",
            ),
        )
    }

    @Test
    fun `an ordinary send carries no sticker key at all`() {
        val rest = json.encodeToString(
            SendMessageRequest.serializer(),
            SendMessageRequest("u1", "hi", attachmentIds = listOf(34)),
        )
        assertThat(rest).isEqualTo("""{"client_msg_id":"u1","body":"hi","attachment_ids":[34]}""")

        val frame = json.encodeToString<ClientFrame>(
            ClientFrame.Send(chatId = 42, clientMsgId = "u1", body = "hi"),
        )
        assertThat(frame).doesNotContain("sticker")
    }

    @Test
    fun `the send frame carries the flag`() {
        val encoded = json.encodeToString<ClientFrame>(
            ClientFrame.Send(chatId = 42, clientMsgId = "u1", body = "", attachmentIds = listOf(90), sticker = true),
        )

        assertThat(json.parseToJsonElement(encoded)).isEqualTo(
            json.parseToJsonElement(
                """{"type":"send","chat_id":42,"client_msg_id":"u1","body":"","attachment_ids":[90],"sticker":true}""",
            ),
        )
    }

    // -- The frame ----------------------------------------------------------------------

    @Test
    fun `a pack_item frame decodes live and as a tombstone`() {
        val added = parseServerFrame(
            json,
            """{"type": "pack_item", "item": {"id": 5, "added_by": 7,
                "attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 9},
                "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12}}""",
        )
        assertThat((added as ServerFrame.PackItem).item.attachment?.id).isEqualTo(71)

        val removed = parseServerFrame(
            json,
            """{"type": "pack_item", "item": {"id": 5, "deleted": true, "pack_seq": 14}}""",
        )
        assertThat((removed as ServerFrame.PackItem).item.isTombstone).isTrue()
    }
}
