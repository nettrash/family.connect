/*
 * ShareStashTest.kt
 * Family Connect (Android)
 *
 * The share hand-off's three rules: only the TARGETED chat's composer
 * can claim (and exactly once); a newer share replaces an unclaimed one,
 * files and all; a discarded share deletes its cache files.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Test
import java.io.File

class ShareStashTest {

    private fun tempPrepared(): MediaPrep.Prepared {
        val file = File.createTempFile("fc-share", ".jpg").apply { writeBytes(ByteArray(8) { 1 }) }
        return MediaPrep.Prepared(
            file = file,
            mime = "image/jpeg",
            kind = AttachmentDto.KIND_PHOTO,
            width = 10,
            height = 10,
            durationMs = null,
            previewJpeg = null,
        )
    }

    @Test
    fun `only the targeted chat claims, and exactly once`() {
        val stash = ShareStash()
        val item = tempPrepared()
        stash.deposit(listOf(item), text = null)
        stash.target(42L)

        // A chat opened by any other route finds nothing.
        assertThat(stash.claim(7L)).isNull()

        val claim = stash.claim(42L)
        assertThat(claim).isNotNull()
        assertThat(claim!!.items).containsExactly(item)
        assertThat(claim.text).isNull()

        // Exactly once: the same chat opened again finds nothing.
        assertThat(stash.claim(42L)).isNull()
    }

    @Test
    fun `an untargeted deposit is unclaimable`() {
        val stash = ShareStash()
        stash.deposit(listOf(tempPrepared()), text = null)
        assertThat(stash.claim(42L)).isNull()
    }

    @Test
    fun `a newer share replaces an unclaimed one, files and all`() {
        val stash = ShareStash()
        val old = tempPrepared()
        stash.deposit(listOf(old), text = null)

        val fresh = tempPrepared()
        stash.deposit(listOf(fresh), text = null)
        stash.target(42L)

        // The abandoned share's cache file went with it.
        assertThat(old.file.exists()).isFalse()
        assertThat(stash.claim(42L)!!.items).containsExactly(fresh)
        assertThat(fresh.file.exists()).isTrue()
    }

    @Test
    fun `discarding deletes the cache files`() {
        val stash = ShareStash()
        val item = tempPrepared()
        stash.deposit(listOf(item), text = null)
        stash.target(42L)

        stash.discard()

        assertThat(item.file.exists()).isFalse()
        assertThat(stash.claim(42L)).isNull()
    }

    /** Shared words ride the same stash, with no files to clean up. */
    @Test
    fun `shared text is claimed like items are`() {
        val stash = ShareStash()
        stash.deposit(emptyList(), text = "dinner at 7")
        stash.target(42L)

        val claim = stash.claim(42L)
        assertThat(claim!!.text).isEqualTo("dinner at 7")
        assertThat(claim.items).isEmpty()
    }
}
