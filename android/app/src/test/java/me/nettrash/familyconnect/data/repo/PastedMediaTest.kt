/*
 * PastedMediaTest.kt
 * Family Connect (Android)
 *
 * What a clipboard item is sent as, and what it is called.
 *
 * Three rules earn their own tests because breaking any of them is
 * silent — the send succeeds and what arrives is wrong:
 *
 *  - An animated GIF sent as a photo would be re-encoded to a single
 *    JPEG frame, which kills the animation. It must go as a file.
 *  - A copied LINK is a Uri too. Trying to attach one would replace an
 *    ordinary text paste with a failed download.
 *  - `kind=file` is refused outright without a name, and a pasted item
 *    very often has none.
 *
 * Pure: no Android, no Robolectric — the decision is made from a scheme
 * string and a media type string.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Test

class PastedMediaTest {

    private fun kind(mime: String?, scheme: String? = "content") =
        PastedMedia.kindFor(scheme, mime)

    // -- Kinds ---------------------------------------------------------------

    @Test
    fun `the four magic-checked image types go as photos`() {
        assertThat(kind("image/jpeg")).isEqualTo(AttachmentDto.KIND_PHOTO)
        assertThat(kind("image/png")).isEqualTo(AttachmentDto.KIND_PHOTO)
        assertThat(kind("image/heic")).isEqualTo(AttachmentDto.KIND_PHOTO)
        assertThat(kind("image/heif")).isEqualTo(AttachmentDto.KIND_PHOTO)
        // Not a real media type, but providers emit it and it means jpeg.
        assertThat(kind("image/jpg")).isEqualTo(AttachmentDto.KIND_PHOTO)
    }

    /**
     * The one that matters most: a GIF re-encoded to JPEG is a still
     * picture of a moving one, and the server would refuse `image/gif`
     * as a photo anyway.
     */
    @Test
    fun `an animated gif goes as a file, not a photo`() {
        assertThat(kind("image/gif")).isEqualTo(AttachmentDto.KIND_FILE)
    }

    @Test
    fun `webp and bmp go as files too`() {
        assertThat(kind("image/webp")).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(kind("image/bmp")).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(kind("image/svg+xml")).isEqualTo(AttachmentDto.KIND_FILE)
    }

    @Test
    fun `every video type goes as a video, because the rest are re-encoded`() {
        assertThat(kind("video/mp4")).isEqualTo(AttachmentDto.KIND_VIDEO)
        assertThat(kind("video/quicktime")).isEqualTo(AttachmentDto.KIND_VIDEO)
        assertThat(kind("video/webm")).isEqualTo(AttachmentDto.KIND_VIDEO)
        assertThat(kind("video/x-matroska")).isEqualTo(AttachmentDto.KIND_VIDEO)
    }

    @Test
    fun `audio the server can check gets a player and the rest go as files`() {
        assertThat(kind("audio/mpeg")).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(kind("audio/mp4")).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(kind("audio/wav")).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(kind("audio/ogg")).isEqualTo(AttachmentDto.KIND_AUDIO)
        // Not in the server's list: a file, where nothing is verified.
        assertThat(kind("audio/flac")).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(kind("audio/midi")).isEqualTo(AttachmentDto.KIND_FILE)
    }

    @Test
    fun `anything else is a file, including a type nobody declared`() {
        assertThat(kind("application/pdf")).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(kind("application/octet-stream")).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(kind(null)).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(kind("")).isEqualTo(AttachmentDto.KIND_FILE)
    }

    @Test
    fun `a media type is read case-insensitively and without its parameters`() {
        assertThat(kind("IMAGE/JPEG")).isEqualTo(AttachmentDto.KIND_PHOTO)
        assertThat(kind("image/png; charset=binary")).isEqualTo(AttachmentDto.KIND_PHOTO)
        assertThat(kind("  image/gif  ")).isEqualTo(AttachmentDto.KIND_FILE)
    }

    // -- What is not an attachment at all ------------------------------------

    /**
     * A copied link. Pasting one must put the address in the composer;
     * attaching it would mean downloading somebody's web page and sending
     * it as a document.
     */
    @Test
    fun `a copied link is not attachable`() {
        assertThat(kind("image/jpeg", scheme = "https")).isNull()
        assertThat(kind("text/plain", scheme = "http")).isNull()
        assertThat(kind(null, scheme = "mailto")).isNull()
        assertThat(kind("image/png", scheme = null)).isNull()
    }

    @Test
    fun `content and file uris are the ones whose bytes can be opened`() {
        assertThat(PastedMedia.isReadable("content")).isTrue()
        assertThat(PastedMedia.isReadable("file")).isTrue()
        assertThat(PastedMedia.isReadable("CONTENT")).isTrue()
        assertThat(PastedMedia.isReadable("https")).isFalse()
    }

    // -- Names ---------------------------------------------------------------

    @Test
    fun `a synthesised name carries the extension of its type`() {
        assertThat(PastedMedia.nameFor("Pasted image", "image/gif")).isEqualTo("Pasted image.gif")
        assertThat(PastedMedia.nameFor("Pasted file", "application/pdf")).isEqualTo("Pasted file.pdf")
        assertThat(PastedMedia.nameFor("Pasted sound", "audio/mpeg")).isEqualTo("Pasted sound.mp3")
    }

    @Test
    fun `an unknown type still produces a usable extension`() {
        assertThat(PastedMedia.extensionFor("application/zip")).isEqualTo("zip")
        assertThat(PastedMedia.extensionFor("application/x-tar")).isEqualTo("tar")
        assertThat(PastedMedia.extensionFor("image/svg+xml")).isEqualTo("svg")
        // Nothing word-like left: better a dull extension than a broken one.
        assertThat(PastedMedia.extensionFor("application/vnd.ms-excel")).isEqualTo("bin")
        assertThat(PastedMedia.extensionFor(null)).isEqualTo("bin")
        assertThat(PastedMedia.extensionFor("")).isEqualTo("bin")
    }

    /** The server's ceiling on a name (protocol.md, "Files"). */
    @Test
    fun `a name never exceeds what the server accepts`() {
        val name = PastedMedia.nameFor("x".repeat(400), "application/pdf")
        assertThat(name.length).isAtMost(MediaPrep.MAX_NAME_LEN)
    }

    @Test
    fun `a name is never empty, whatever it was built from`() {
        assertThat(PastedMedia.nameFor("   ", null)).isEqualTo("file.bin")
    }

    @Test
    fun `the top-level type picks which word a name is built on`() {
        assertThat(PastedMedia.topLevelType("image/gif")).isEqualTo("image")
        assertThat(PastedMedia.topLevelType("AUDIO/MPEG")).isEqualTo("audio")
        assertThat(PastedMedia.topLevelType("application/pdf")).isEqualTo("application")
        assertThat(PastedMedia.topLevelType(null)).isEmpty()
    }

    // -- A whole clipboard ---------------------------------------------------
    //
    // The same rule one level up. It exists because every paste DOOR used
    // to answer this for itself, and they disagreed: the attach menu took
    // the picture out of a picture-and-words clip and dropped the words,
    // while the text field took the picture AND pasted the words. One
    // answer now, and a door's only job is to obey it.

    private fun item(scheme: String? = null, mime: String? = null, text: String? = null) =
        PastedMedia.Item(scheme = scheme, mime = mime, text = text)

    @Test
    fun `a clipboard of words is words`() {
        val verdict = PastedMedia.decide(listOf(item(text = "dinner at 7")))
        assertThat(verdict).isEqualTo(PastedMedia.Verdict.Words("dinner at 7"))
    }

    @Test
    fun `a clipboard holding a picture is an attachment`() {
        val verdict = PastedMedia.decide(listOf(item(scheme = "content", mime = "image/png")))
        assertThat(verdict)
            .isEqualTo(PastedMedia.Verdict.Attach(index = 0, kind = AttachmentDto.KIND_PHOTO))
    }

    @Test
    fun `a clipboard holding a file is an attachment`() {
        val verdict = PastedMedia.decide(listOf(item(scheme = "content", mime = "application/pdf")))
        assertThat(verdict)
            .isEqualTo(PastedMedia.Verdict.Attach(index = 0, kind = AttachmentDto.KIND_FILE))
    }

    /**
     * The shape a browser's "copy image" has. The picture is the paste;
     * the words it came with are its own address, and nobody wants that
     * typed next to the picture — a caption is written, not inherited.
     */
    @Test
    fun `an attachable item wins over words in the same clip`() {
        val verdict = PastedMedia.decide(
            listOf(
                item(text = "https://example.com/holiday.jpg"),
                item(scheme = "content", mime = "image/jpeg"),
            ),
        )
        assertThat(verdict)
            .isEqualTo(PastedMedia.Verdict.Attach(index = 1, kind = AttachmentDto.KIND_PHOTO))
    }

    /** A copied link is a Uri and is still words. */
    @Test
    fun `a link is words, not an attachment`() {
        val verdict = PastedMedia.decide(
            listOf(item(scheme = "https", mime = "image/jpeg", text = "https://example.com/a.jpg")),
        )
        assertThat(verdict).isEqualTo(PastedMedia.Verdict.Words("https://example.com/a.jpg"))
    }

    @Test
    fun `an empty clipboard is nothing, and so is one with only empty words`() {
        assertThat(PastedMedia.decide(emptyList())).isEqualTo(PastedMedia.Verdict.Empty)
        assertThat(PastedMedia.decide(listOf(item(text = "")))).isEqualTo(PastedMedia.Verdict.Empty)
        // A link with no text of its own: not attachable, nothing to paste.
        assertThat(PastedMedia.decide(listOf(item(scheme = "https"))))
            .isEqualTo(PastedMedia.Verdict.Empty)
    }

    /** The first attachable item, since a message carries only one. */
    @Test
    fun `the first attachable item is the one taken`() {
        val verdict = PastedMedia.decide(
            listOf(
                item(scheme = "content", mime = "video/mp4"),
                item(scheme = "content", mime = "image/png"),
            ),
        )
        assertThat(verdict)
            .isEqualTo(PastedMedia.Verdict.Attach(index = 0, kind = AttachmentDto.KIND_VIDEO))
    }
}
