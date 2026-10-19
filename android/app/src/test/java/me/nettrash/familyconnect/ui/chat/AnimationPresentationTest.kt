/*
 * AnimationPresentationTest.kt
 * Family Connect (Android)
 *
 * Which messages are drawn as an ANIMATION, and what that changes around it
 * (#96; docs/protocol.md, "Animations" → "How it is drawn"): the one test
 * every client shares — exactly one attachment, `kind=video`,
 * `animation: true` — the chat list's and the quote's "GIF" (and the
 * sticker's quote, which used to be blank), when a tile plays by itself, how
 * its video is fitted to its box, the BT.709 pixels the encoder writes, and
 * the one "GIF" every language reads. RoundPresentationTest's twin.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.repo.AnimationEncoder
import me.nettrash.familyconnect.data.repo.MessageRepository
import me.nettrash.familyconnect.ui.components.AnimationPlayback
import org.junit.Test
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory

class AnimationPresentationTest {

    private val gif = AttachmentDto(
        id = 93, kind = "video", mime = "video/mp4", size = 182_330, width = 480, height = 270,
        durationMs = 2_100, hasPreview = true, animation = true,
    )
    private val video = gif.copy(id = 92, animation = null)
    private val photo = AttachmentDto(id = 94, kind = "photo", mime = "image/jpeg", size = 9)
    private val sticker = AttachmentDto(id = 95, kind = "photo", mime = "image/webp", size = 9, sticker = true)

    private fun message(
        attachments: List<AttachmentDto>,
        body: String = "",
        serverId: Long? = 1,
        replyToMessageId: Long? = null,
    ) = MessageEntity(
        clientMsgId = "m$serverId-$replyToMessageId",
        serverId = serverId,
        chatId = 42,
        senderId = 9,
        body = body,
        createdAt = 1_700_000_000_000,
        status = MessageStatus.SENT,
        replyToMessageId = replyToMessageId,
        replySenderId = replyToMessageId?.let { 3 },
        replyExcerpt = replyToMessageId?.let { "" },
        attachmentsJson = AttachmentsCodec.encode(attachments).takeIf { attachments.isNotEmpty() },
    )

    // -- Which messages are animations ------------------------------------------------------

    @Test
    fun `one flagged video is an animation, captioned or not`() {
        assertThat(animationOf(message(listOf(gif)))).isEqualTo(gif)
        assertThat(animationOf(message(listOf(gif), body = "lol"))).isEqualTo(gif)
        assertThat(animationOf(message(listOf(gif), replyToMessageId = 41))).isEqualTo(gif)
    }

    @Test
    fun `anything else is the ordinary message it otherwise is`() {
        assertThat(animationOf(message(listOf(video)))).isNull()
        assertThat(animationOf(message(listOf(gif, photo)))).isNull()
        assertThat(animationOf(message(listOf(photo.copy(animation = true))))).isNull()
        assertThat(animationOf(message(emptyList(), body = "hi"))).isNull()
    }

    @Test
    fun `an animation keeps Edit, for its caption`() {
        assertThat(canEditMessage(message(listOf(gif), body = "lol").copy(senderId = 7), myUserId = 7)).isTrue()
    }

    // -- What it is called ----------------------------------------------------------------------

    @Test
    fun `a chat-list row says GIF where a video's says Video`() {
        assertThat(MessageRepository.previewText("", listOf(gif))).isEqualTo("GIF")
        assertThat(MessageRepository.previewText("", listOf(video))).isEqualTo("Video")
        // A caption is the row's words, as for any attachment.
        assertThat(MessageRepository.previewText("lol", listOf(gif))).isEqualTo("lol")
    }

    @Test
    fun `a quote names a gif, a circle and a sticker it holds, and a captioned one keeps its words`() {
        val circle = video.copy(id = 96, round = true)
        val items = buildChatItems(
            messagesNewestFirst = listOf(
                message(emptyList(), body = "ha", serverId = 9, replyToMessageId = 1),
                message(emptyList(), body = "ha", serverId = 8, replyToMessageId = 2),
                message(emptyList(), body = "ha", serverId = 7, replyToMessageId = 3),
                message(emptyList(), body = "ha", serverId = 6, replyToMessageId = 4),
                message(emptyList(), body = "ha", serverId = 5, replyToMessageId = 99),
                message(listOf(video), serverId = 4),
                message(listOf(sticker), serverId = 3),
                message(listOf(circle), serverId = 2),
                message(listOf(gif), serverId = 1),
            ),
            isFamilyChat = true,
            myUserId = 7,
            memberNames = emptyMap(),
            nowMillis = 1_700_000_000_000,
        )
        val byId = items.filterIsInstance<ChatListItem.MessageItem>().associateBy { it.entity.serverId }

        assertThat(byId.getValue(9).quotedMedia).isEqualTo(QuotedMedia.GIF)
        assertThat(byId.getValue(8).quotedMedia).isEqualTo(QuotedMedia.VIDEO_MESSAGE)
        assertThat(byId.getValue(8).quotesRound).isTrue()
        assertThat(byId.getValue(7).quotedMedia).isEqualTo(QuotedMedia.STICKER)
        assertThat(byId.getValue(6).quotedMedia).isNull()
        assertThat(byId.getValue(5).quotedMedia).isNull()
        // The reply banner's question, by id.
        assertThat(quotedMedia(items, 1)).isEqualTo(QuotedMedia.GIF)
        assertThat(quotedMedia(items, 3)).isEqualTo(QuotedMedia.STICKER)
        assertThat(quotedMedia(items, 4)).isNull()
        assertThat(quotedMedia(items, 99)).isNull()
    }

    // -- How it plays ------------------------------------------------------------------------------

    @Test
    fun `a tile plays by itself up to 8 MiB and never with animations off`() {
        val limit = 8L * 1024 * 1024
        assertThat(AnimationPlayback.TILE_DOWNLOAD_LIMIT).isEqualTo(limit)
        assertThat(AnimationPlayback.tilePlays(limit, animationsRemoved = false)).isTrue()
        assertThat(AnimationPlayback.tilePlays(limit + 1, animationsRemoved = false)).isFalse()
        assertThat(AnimationPlayback.tilePlays(1, animationsRemoved = true)).isFalse()
        assertThat(AnimationPlayback.viewerDownloads(limit)).isTrue()
        assertThat(AnimationPlayback.viewerDownloads(limit + 1)).isFalse()
    }

    @Test
    fun `the tile crops to its box and the viewer fits the picture whole`() {
        // A 2:1 video in a square view: the tile fills (wider than the view),
        // the viewer letterboxes (shorter than the view).
        assertThat(AnimationPlayback.textureScale(100, 100, 200, 100, crop = true)).isEqualTo(2f to 1f)
        assertThat(AnimationPlayback.textureScale(100, 100, 200, 100, crop = false)).isEqualTo(1f to 0.5f)
        // The same shape needs nothing.
        assertThat(AnimationPlayback.textureScale(320, 180, 640, 360, crop = true)).isEqualTo(1f to 1f)
        assertThat(AnimationPlayback.textureScale(0, 100, 200, 100, crop = true)).isEqualTo(1f to 1f)
    }

    // -- The encoder's pixels -------------------------------------------------------------------------

    @Test
    fun `white, black and grey land where BT709 limited range puts them`() {
        assertThat(AnimationEncoder.luma(0xFFFFFFFF.toInt())).isEqualTo(235)
        assertThat(AnimationEncoder.luma(0xFF000000.toInt())).isEqualTo(16)
        assertThat(AnimationEncoder.chromaBlue(255, 255, 255)).isEqualTo(128)
        assertThat(AnimationEncoder.chromaRed(255, 255, 255)).isEqualTo(128)
        assertThat(AnimationEncoder.chromaBlue(0, 0, 0)).isEqualTo(128)
        assertThat(AnimationEncoder.chromaRed(128, 128, 128)).isEqualTo(128)
        // Pure red: Y ≈ 63, Cr at the top of its range.
        assertThat(AnimationEncoder.luma(0xFFFF0000.toInt())).isEqualTo(63)
        assertThat(AnimationEncoder.chromaRed(255, 0, 0)).isEqualTo(240)
        assertThat(AnimationEncoder.chromaBlue(0, 0, 255)).isEqualTo(240)
    }

    // -- The word -----------------------------------------------------------------------------------------

    /**
     * "GIF" is "the same in every language": a name, kept once, untranslatable,
     * in the base strings — the house's way with a brand word (app_name) — so
     * every locale reads that one, and no translation can make it something else.
     */
    @Test
    fun `GIF is one untranslatable word that no language overrides`() {
        val res = File("src/main/res")
        val base = strings(File(res, "values/strings.xml"))
        assertThat(base["s_gif"]).isEqualTo("GIF" to "false")
        val locales = res.listFiles { file -> file.isDirectory && file.name.startsWith("values-") }.orEmpty()
            .map { File(it, "strings.xml") }.filter { it.isFile }
        assertThat(locales).isNotEmpty()
        for (file in locales) {
            assertThat(strings(file)).doesNotContainKey("s_gif")
        }
    }

    /** name → (text, translatable attribute). */
    private fun strings(file: File): Map<String, Pair<String, String>> {
        val document = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        val nodes = document.getElementsByTagName("string")
        return (0 until nodes.length).associate { index ->
            val element = nodes.item(index) as org.w3c.dom.Element
            element.getAttribute("name") to (element.textContent to element.getAttribute("translatable"))
        }
    }
}
