/*
 * AttachmentAlbumTest.kt
 * Family Connect (Android)
 *
 * The album value is what both the bubble's stack and the viewer's pager
 * are built from, so its three decisions are pinned here without any
 * composition: which attachments are media (order and kinds kept), what
 * shape the front card reserves from metadata alone (a ratio — the width
 * is the balloon's), and that an index can never point past the ends.
 */

package me.nettrash.familyconnect.ui.components

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import org.junit.Test

class AttachmentAlbumTest {

    private val photo1 = FakeAttachmentApi.attachment(id = 1, kind = "photo")
    private val video2 = FakeAttachmentApi.attachment(id = 2, kind = "video")
    private val file3 = FakeAttachmentApi.attachment(id = 3, kind = "file")
    private val photo4 = FakeAttachmentApi.attachment(id = 4, kind = "photo")
    private val audio5 = FakeAttachmentApi.attachment(id = 5, kind = "audio")
    private val place6 = FakeAttachmentApi.attachment(id = 6, kind = "location")

    private val mixed = listOf(photo1, file3, video2, audio5, photo4, place6)

    @Test
    fun mediaKeepsPhotosAndVideosInSentOrder() {
        assertThat(AttachmentAlbum.media(mixed)).containsExactly(photo1, video2, photo4).inOrder()
    }

    @Test
    fun rowsKeepFilesAudioAndLocationsInSentOrder() {
        assertThat(AttachmentAlbum.rows(mixed)).containsExactly(file3, audio5, place6).inOrder()
    }

    @Test
    fun mediaAndRowsPartitionTheMessage() {
        val together = AttachmentAlbum.media(mixed) + AttachmentAlbum.rows(mixed)
        assertThat(together).containsExactlyElementsIn(mixed)
        assertThat(AttachmentAlbum.media(emptyList())).isEmpty()
        assertThat(AttachmentAlbum.rows(listOf(photo1))).isEmpty()
    }

    @Test
    fun cardTakesTheFirstItemsAspect() {
        // 1600×1200 is 4:3, inside the clamp.
        assertThat(AttachmentAlbum.cardRatio(photo1)).isWithin(0.001f).of(4f / 3f)
    }

    @Test
    fun aPanoramaIsClampedToTheWidestCard() {
        val wide = photo1.copy(width = 4000, height = 1000)
        assertThat(AttachmentAlbum.cardRatio(wide)).isEqualTo(AttachmentAlbum.MAX_CARD_RATIO)
    }

    @Test
    fun aTallCropIsClampedToTheTallestCard() {
        val tall = photo1.copy(width = 1000, height = 4000)
        assertThat(AttachmentAlbum.cardRatio(tall)).isEqualTo(AttachmentAlbum.MIN_CARD_RATIO)
    }

    @Test
    fun missingDimensionsFallBackToTheDtoDefaultShape() {
        val unknown = photo1.copy(width = null, height = null)
        assertThat(AttachmentAlbum.cardRatio(unknown)).isEqualTo(AttachmentDto.DEFAULT_ASPECT)
    }

    @Test
    fun anIndexPastEitherEndIsPulledToThatEnd() {
        val items = listOf(photo1, video2, photo4)
        assertThat(AttachmentAlbum(items, -3).clampedIndex).isEqualTo(0)
        assertThat(AttachmentAlbum(items, -3).current).isEqualTo(photo1)
        assertThat(AttachmentAlbum(items, 9).clampedIndex).isEqualTo(2)
        assertThat(AttachmentAlbum(items, 9).current).isEqualTo(photo4)
        assertThat(AttachmentAlbum(items, 1).current).isEqualTo(video2)
    }

    @Test
    fun anEmptyAlbumIsRefused() {
        assertThat(runCatching { AttachmentAlbum(emptyList(), 0) }.exceptionOrNull())
            .isInstanceOf(IllegalArgumentException::class.java)
    }

    @Test
    fun openingATappedItemStartsOnItsPageAmongTheMessagesMedia() {
        val album = AttachmentAlbum.opening(mixed, photo4)
        assertThat(album.items).containsExactly(photo1, video2, photo4).inOrder()
        assertThat(album.index).isEqualTo(2)
    }

    @Test
    fun openingSomethingOutsideTheMessageShowsItAlone() {
        val stray = FakeAttachmentApi.attachment(id = 99, kind = "photo")
        val album = AttachmentAlbum.opening(mixed, stray)
        assertThat(album.items).containsExactly(stray)
        assertThat(album.index).isEqualTo(0)
    }
}
