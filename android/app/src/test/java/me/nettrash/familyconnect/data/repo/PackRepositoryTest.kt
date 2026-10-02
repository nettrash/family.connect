/*
 * PackRepositoryTest.kt
 * Family Connect (Android)
 *
 * The family's sticker pack (docs/protocol.md, "Sticker pack"): the cursor
 * rules, which are the board's unchanged; the gone set; who may remove; the
 * two ceilings; and the one thing the pack may never do to a picture, which
 * is prepare it.
 *
 * "Pack", not "sticker": in this codebase a sticker is a board note, and
 * BoardRepositoryTest is where those live.
 *
 * iOS counterpart: the pack tests in ios/FamilyConnectTests.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.PackDao
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AttachmentResponse
import me.nettrash.familyconnect.data.net.dto.PackItemResponse
import me.nettrash.familyconnect.data.net.dto.PackResponse
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakePackApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.packItemDto
import me.nettrash.familyconnect.testutil.packTombstone
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class PackRepositoryTest {

    private companion object {
        const val ME = 7L
        const val MAX_ITEMS = 3
        const val MAX_BYTES = 8L * 1024

        /** `RIFF....WEBP` and a lossy chunk: a WebP as far as the magic number goes. */
        fun webp(size: Int = 64, fill: Byte = 1): ByteArray {
            val bytes = ByteArray(size) { fill }
            "RIFF".toByteArray().copyInto(bytes, 0)
            "WEBP".toByteArray().copyInto(bytes, 8)
            "VP8 ".toByteArray().copyInto(bytes, 12)
            return bytes
        }
    }

    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private lateinit var packDao: PackDao
    private lateinit var packApi: FakePackApi
    private lateinit var attachmentApi: FakeAttachmentApi
    private lateinit var settings: FakeSettingsRepository
    private lateinit var socket: FakeChatSocket

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        packDao = db.packDao()
        packApi = FakePackApi()
        attachmentApi = FakeAttachmentApi()
        settings = FakeSettingsRepository(
            SettingsState(myUserId = ME, packMaxItems = MAX_ITEMS, packMaxItemBytes = MAX_BYTES),
        )
        socket = FakeChatSocket()
        PackRepository.bytesDirectory(RuntimeEnvironment.getApplication()).deleteRecursively()
    }

    @After
    fun tearDown() {
        db.close()
    }

    /** The frame collector runs for the life of the app scope, so it belongs on backgroundScope. */
    private fun TestScope.repository() = PackRepository(
        context = RuntimeEnvironment.getApplication(),
        packApi = packApi,
        attachmentApi = attachmentApi,
        packDao = packDao,
        settings = settings,
        socket = socket,
        scope = backgroundScope,
    )

    private suspend fun heldIds(): List<Long> = packDao.items().map { it.id }

    /** A server that holds [bytes] under every attachment id asked for. */
    private fun serveBytes(bytes: ByteArray) {
        attachmentApi.downloadHandler = { _, _, destination ->
            destination.parentFile?.mkdirs()
            destination.writeBytes(bytes)
            ApiResult.Ok(Unit)
        }
    }

    // -- The per-item guard ---------------------------------------------------

    @Test
    fun `an item is written only when the incoming seq is greater`() = runTest(dispatcher) {
        val repository = repository()

        assertThat(repository.applyItem(packItemDto(id = 5, packSeq = 12, label = "party cat"))).isTrue()
        // The same state again, and an older one: neither is news.
        assertThat(repository.applyItem(packItemDto(id = 5, packSeq = 12, label = "other"))).isFalse()
        assertThat(repository.applyItem(packItemDto(id = 5, packSeq = 9, label = "older"))).isFalse()

        val held = packDao.findById(5)!!
        assertThat(held.label).isEqualTo("party cat")
        assertThat(held.packSeq).isEqualTo(12)
        assertThat(held.addedBy).isEqualTo(7)
        assertThat(held.attachment?.mime).isEqualTo("image/webp")
    }

    @Test
    fun `a pack item never keeps a sticker flag`() = runTest(dispatcher) {
        val repository = repository()
        val item = packItemDto(id = 5, packSeq = 12)

        // The flag is a MESSAGE's. Should one ever arrive on an item, it is
        // not stored — a pack picture is an ordinary photo attachment.
        repository.applyItem(item.copy(attachment = item.attachment!!.copy(sticker = true)))

        assertThat(packDao.findById(5)!!.attachment?.sticker).isNull()
    }

    @Test
    fun `a live item with no picture is dropped rather than drawn empty`() = runTest(dispatcher) {
        val repository = repository()

        assertThat(repository.applyItem(packItemDto(id = 5, packSeq = 12).copy(attachment = null))).isFalse()

        assertThat(heldIds()).isEmpty()
    }

    // -- Tombstones and the gone set ------------------------------------------

    @Test
    fun `a tombstone removes the item and an older copy cannot bring it back`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 5, packSeq = 12))

        assertThat(repository.applyItem(packTombstone(id = 5, packSeq = 14))).isTrue()
        assertThat(heldIds()).isEmpty()

        // The pre-removal copy, still travelling on a catch-up page — and
        // even a copy claiming a HIGHER seq. Item ids are never reused, so
        // gone is gone.
        assertThat(repository.applyItem(packItemDto(id = 5, packSeq = 12))).isFalse()
        assertThat(repository.applyItem(packItemDto(id = 5, packSeq = 99))).isFalse()
        assertThat(heldIds()).isEmpty()
    }

    @Test
    fun `a tombstone for an item never held is still remembered`() = runTest(dispatcher) {
        val repository = repository()

        // Nothing to remove, so nothing changed — but the copy it is about
        // may still be on its way.
        assertThat(repository.applyItem(packTombstone(id = 5, packSeq = 14))).isFalse()
        assertThat(repository.applyItem(packItemDto(id = 5, packSeq = 12))).isFalse()

        assertThat(heldIds()).isEmpty()
    }

    @Test
    fun `a stale tombstone does not remove a newer item`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 5, packSeq = 20))

        assertThat(repository.applyItem(packTombstone(id = 5, packSeq = 14))).isFalse()

        assertThat(heldIds()).containsExactly(5L)
    }

    // -- The full read --------------------------------------------------------

    @Test
    fun `a full read replaces what is held except items above its mark`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 1, packSeq = 10))
        repository.applyItem(packItemDto(id = 2, packSeq = 11))
        // Arrived AFTER the read below was taken: above its mark, it stays.
        repository.applyItem(packItemDto(id = 9, packSeq = 30))
        settings.setPackCursor(11)

        // The read left item 2 out: it was removed while this device was
        // not listening, and a full read never returns tombstones.
        packApi.pack = PackResponse(
            items = listOf(packItemDto(id = 1, packSeq = 10), packItemDto(id = 3, packSeq = 15)),
            maxPackSeq = 20,
        )
        assertThat(repository.loadPack()).isTrue()

        assertThat(heldIds()).containsExactly(1L, 3L, 9L).inOrder()
        assertThat(settings.current.packCursor).isEqualTo(20)
        // Left out by the read is gone for good, exactly like a tombstone.
        assertThat(repository.applyItem(packItemDto(id = 2, packSeq = 11))).isFalse()
    }

    @Test
    fun `the panel order is the order added whatever the seqs say`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 8, packSeq = 3))
        repository.applyItem(packItemDto(id = 2, packSeq = 40))
        repository.applyItem(packItemDto(id = 5, packSeq = 17))

        assertThat(repository.observeItems().first().map { it.id }).containsExactly(2L, 5L, 8L).inOrder()
    }

    @Test
    fun `a full read older than what is applied is ignored`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 1, packSeq = 5))
        repository.applyItem(packItemDto(id = 2, packSeq = 25))
        settings.setPackCursor(25)

        // A read whose mark is behind what this device has applied is
        // ignored WHOLE: applied as a replacement it would drop item 1
        // (at or below its mark, and not listed) and remember it as gone
        // for good, on the word of an answer older than the pack held.
        packApi.pack = PackResponse(items = emptyList(), maxPackSeq = 9)
        assertThat(repository.loadPack()).isTrue()

        assertThat(heldIds()).containsExactly(1L, 2L)
        assertThat(settings.current.packCursor).isEqualTo(25)
        assertThat(packDao.isGone(1)).isFalse()
    }

    @Test
    fun `a full read older than what is applied is ignored with nothing held too`() = runTest(dispatcher) {
        val repository = repository()
        // Every sticker was removed and this device has seen it happen: it
        // holds nothing, at 25.
        settings.setPackCursor(25)

        // A read taken at 9, arriving late, still lists an item removed
        // since — one this device never held and so never remembered as
        // gone. Applied, it would be drawn, and the cursor would go BACK.
        packApi.pack = PackResponse(items = listOf(packItemDto(id = 1, packSeq = 5)), maxPackSeq = 9)
        assertThat(repository.loadPack()).isTrue()

        assertThat(heldIds()).isEmpty()
        assertThat(settings.current.packCursor).isEqualTo(25)
    }

    @Test
    fun `a failed full read changes nothing`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 1, packSeq = 10))
        packApi.packResult = { ApiResult.NetworkError(IllegalStateException("offline")) }

        assertThat(repository.loadPack()).isFalse()

        assertThat(heldIds()).containsExactly(1L)
        assertThat(settings.current.packCursor).isEqualTo(0)
    }

    // -- The reconnect step ----------------------------------------------------

    @Test
    fun `a device that holds no pack reads the whole of it`() = runTest(dispatcher) {
        val repository = repository()
        packApi.pack = PackResponse(listOf(packItemDto(id = 1, packSeq = 10)), maxPackSeq = 14)

        repository.catchUpPack(serverMaxSeq = 14)

        assertThat(packApi.fullReads).isEqualTo(1)
        assertThat(packApi.changeRequests).isEmpty()
        assertThat(heldIds()).containsExactly(1L)
        assertThat(settings.current.packCursor).isEqualTo(14)
    }

    @Test
    fun `a pack nobody has touched costs no request`() = runTest(dispatcher) {
        val repository = repository()

        // `max_pack_seq` omitted: never written to — or a server with no
        // packs at all.
        repository.catchUpPack(serverMaxSeq = 0)

        assertThat(packApi.fullReads).isEqualTo(0)
        assertThat(packApi.changeRequests).isEmpty()
    }

    @Test
    fun `an emptied pack this device is level with is not read again`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 1, packSeq = 10))
        repository.applyItem(packTombstone(id = 1, packSeq = 14))
        settings.setPackCursor(14)

        repository.catchUpPack(serverMaxSeq = 14)

        assertThat(packApi.fullReads).isEqualTo(0)
    }

    @Test
    fun `catch-up loops the change feed from the cursor with tombstones`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 1, packSeq = 10))
        repository.applyItem(packItemDto(id = 2, packSeq = 11))
        settings.setPackCursor(11)
        // One FULL page and one short one: the loop ends on the short page.
        val firstPage = (0 until PackRepository.PACK_PAGE).map { index ->
            packItemDto(id = 100L + index, packSeq = 12L + index)
        }
        val lastSeq = 12L + PackRepository.PACK_PAGE
        packApi.changePages = mutableListOf(
            firstPage,
            listOf(packTombstone(id = 2, packSeq = lastSeq)),
        )

        repository.catchUpPack(serverMaxSeq = lastSeq)

        assertThat(packApi.fullReads).isEqualTo(0)
        // The second page is asked for from the highest seq of the first.
        assertThat(packApi.changeRequests).containsExactly(11L, lastSeq - 1).inOrder()
        assertThat(packDao.findById(2)).isNull()
        assertThat(packDao.count()).isEqualTo(1 + PackRepository.PACK_PAGE)
        assertThat(settings.current.packCursor).isEqualTo(lastSeq)
    }

    @Test
    fun `catch-up asks nothing when the cursor is level`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 1, packSeq = 10))
        settings.setPackCursor(14)

        repository.catchUpPack(serverMaxSeq = 14)

        assertThat(packApi.changeRequests).isEmpty()
        assertThat(packApi.fullReads).isEqualTo(0)
    }

    @Test
    fun `a cursor left by another family does not veto the new family's pack`() = runTest(dispatcher) {
        val repository = repository()
        // Pack seqs are server-wide: the family just joined may sit far
        // below the mark the last one left behind.
        settings.setPackCursor(900)
        packApi.pack = PackResponse(listOf(packItemDto(id = 1, packSeq = 40)), maxPackSeq = 41)

        repository.catchUpPack(serverMaxSeq = 41)

        assertThat(heldIds()).containsExactly(1L)
        assertThat(settings.current.packCursor).isEqualTo(41)
    }

    // -- Frames ---------------------------------------------------------------

    @Test
    fun `a pack_item frame is applied but moves no cursor before this connection has caught up`() =
        runTest(dispatcher) {
            repository()
            runCurrent()
            socket.setOpen(true)
            settings.setPackCursor(11)

            socket.emit(ServerFrame.PackItem(packItemDto(id = 5, packSeq = 12)))
            runCurrent()
            assertThat(heldIds()).containsExactly(5L)

            socket.emit(ServerFrame.PackItem(packTombstone(id = 5, packSeq = 14)))
            runCurrent()
            assertThat(heldIds()).isEmpty()
            // A frame that jumped the cursor before the catch-up read it
            // would have the catch-up start past everything it was there
            // to fetch.
            assertThat(settings.current.packCursor).isEqualTo(11)
        }

    @Test
    fun `once this connection has caught up a pack_item frame moves the cursor to its own seq`() =
        runTest(dispatcher) {
            val repository = repository()
            runCurrent()
            socket.setOpen(true)
            packApi.pack = PackResponse(listOf(packItemDto(id = 1, packSeq = 10)), maxPackSeq = 14)
            repository.catchUpPack(serverMaxSeq = 14, connection = repository.connectionNow())
            assertThat(settings.current.packCursor).isEqualTo(14)

            socket.emit(ServerFrame.PackItem(packItemDto(id = 5, packSeq = 17)))
            runCurrent()
            assertThat(settings.current.packCursor).isEqualTo(17)

            // A tombstone is a frame like any other.
            socket.emit(ServerFrame.PackItem(packTombstone(id = 5, packSeq = 19)))
            runCurrent()
            // Waited for rather than read: removing the item also deletes
            // its picture, on a real IO thread that virtual time does not
            // drive, and the cursor moves only after the apply is done.
            settings.state.first { it.packCursor == 19L }
            assertThat(heldIds()).containsExactly(1L)

            // And never BACK: a frame repeated late says nothing new.
            socket.emit(ServerFrame.PackItem(packItemDto(id = 1, packSeq = 10)))
            runCurrent()
            assertThat(settings.current.packCursor).isEqualTo(19)
        }

    @Test
    fun `a reconnect is not caught up until its own catch-up has run`() = runTest(dispatcher) {
        val repository = repository()
        runCurrent()
        socket.setOpen(true)
        packApi.pack = PackResponse(listOf(packItemDto(id = 1, packSeq = 10)), maxPackSeq = 14)
        repository.catchUpPack(serverMaxSeq = 14, connection = repository.connectionNow())

        // The wire drops and comes back: whatever happened in between was
        // on no connection at all.
        socket.setOpen(false)
        socket.setOpen(true)
        socket.emit(ServerFrame.PackItem(packItemDto(id = 9, packSeq = 30)))
        runCurrent()
        assertThat(heldIds()).containsExactly(1L, 9L)
        assertThat(settings.current.packCursor).isEqualTo(14)

        // Its catch-up reads what was missed FROM THE OLD CURSOR, and only
        // then do frames move it again.
        packApi.changePages = mutableListOf(
            listOf(packItemDto(id = 7, packSeq = 22), packItemDto(id = 9, packSeq = 30)),
        )
        repository.catchUpPack(serverMaxSeq = 30, connection = repository.connectionNow())
        assertThat(packApi.changeRequests).containsExactly(14L)
        assertThat(heldIds()).containsExactly(1L, 7L, 9L)

        socket.emit(ServerFrame.PackItem(packItemDto(id = 11, packSeq = 33)))
        runCurrent()
        assertThat(settings.current.packCursor).isEqualTo(33)
    }

    @Test
    fun `a catch-up a reconnect has overtaken does not vouch for the new connection`() =
        runTest(dispatcher) {
            val repository = repository()
            runCurrent()
            socket.setOpen(true)
            val startedOn = repository.connectionNow()
            // The wire drops and comes back WHILE the read is in flight:
            // the mark this pass catches up to was taken before the new
            // connection existed.
            packApi.packResult = {
                socket.setOpen(false)
                socket.setOpen(true)
                ApiResult.Ok(PackResponse(listOf(packItemDto(id = 1, packSeq = 10)), maxPackSeq = 14))
            }
            repository.catchUpPack(serverMaxSeq = 14, connection = startedOn)
            assertThat(settings.current.packCursor).isEqualTo(14)

            socket.emit(ServerFrame.PackItem(packItemDto(id = 5, packSeq = 17)))
            runCurrent()

            // Applied, as every frame is — and the cursor stays where the
            // read left it, for the new connection's own catch-up to move.
            assertThat(heldIds()).containsExactly(1L, 5L)
            assertThat(settings.current.packCursor).isEqualTo(14)
        }

    @Test
    fun `a catch-up with no socket open vouches for no connection`() = runTest(dispatcher) {
        val repository = repository()
        runCurrent()
        // Plain REST, the socket still down: the rows are caught up and
        // nothing is said about a connection that does not exist.
        assertThat(repository.connectionNow()).isNull()
        packApi.pack = PackResponse(listOf(packItemDto(id = 1, packSeq = 10)), maxPackSeq = 14)
        repository.catchUpPack(serverMaxSeq = 14, connection = repository.connectionNow())

        socket.setOpen(true)
        socket.emit(ServerFrame.PackItem(packItemDto(id = 5, packSeq = 17)))
        runCurrent()

        assertThat(settings.current.packCursor).isEqualTo(14)
    }

    @Test
    fun `a catch-up that failed leaves the connection not caught up`() = runTest(dispatcher) {
        val repository = repository()
        runCurrent()
        socket.setOpen(true)
        packApi.packResult = { ApiResult.NetworkError(IllegalStateException("offline")) }
        repository.catchUpPack(serverMaxSeq = 14, connection = repository.connectionNow())

        socket.emit(ServerFrame.PackItem(packItemDto(id = 5, packSeq = 17)))
        runCurrent()

        // Had the frame moved the cursor, the next catch-up would start at
        // 17 and never read what the failed one was there to fetch.
        assertThat(settings.current.packCursor).isEqualTo(0)
    }

    @Test
    fun `a blocked member's pack items are not hidden`() = runTest(dispatcher) {
        repository()
        runCurrent()
        // The adder is blocked by this reader. The pack is the family's
        // pictures and not anybody's words: the frame still reaches a
        // blocker, and the item is kept and shown.
        settings.setBlockedUserIds(listOf(11L))

        socket.emit(ServerFrame.PackItem(packItemDto(id = 5, packSeq = 12, addedBy = 11)))
        runCurrent()

        assertThat(packDao.items().single().addedBy).isEqualTo(11)
    }

    // -- Who may remove ---------------------------------------------------------

    @Test
    fun `whoever added it or the family owner may remove and nobody else`() {
        assertThat(PackRules.canRemove(addedBy = 7, myUserId = 7, isOwner = false)).isTrue()
        assertThat(PackRules.canRemove(addedBy = 9, myUserId = 7, isOwner = true)).isTrue()
        assertThat(PackRules.canRemove(addedBy = 9, myUserId = 7, isOwner = false)).isFalse()
        // Before this device knows who it is, it offers nothing.
        assertThat(PackRules.canRemove(addedBy = 9, myUserId = null, isOwner = false)).isFalse()
    }

    @Test
    fun `a removal is remembered before its tombstone arrives`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 5, packSeq = 12))

        assertThat(repository.remove(5)).isEqualTo(PackRepository.RemoveResult.REMOVED)

        assertThat(packApi.removed).containsExactly(5L)
        assertThat(heldIds()).isEmpty()
        // The frame confirming it, and a late copy of the item: both find
        // the id already remembered.
        assertThat(repository.applyItem(packTombstone(id = 5, packSeq = 14))).isFalse()
        assertThat(repository.applyItem(packItemDto(id = 5, packSeq = 12))).isFalse()
    }

    @Test
    fun `a refused removal leaves the item where it is`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 5, packSeq = 12, addedBy = 9))
        packApi.removeResult = { ApiResult.HttpError(403, "not_pack_item_author", "no") }

        assertThat(repository.remove(5)).isEqualTo(PackRepository.RemoveResult.NOT_ALLOWED)

        assertThat(heldIds()).containsExactly(5L)
    }

    @Test
    fun `removing an item the server no longer has is still removed here`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 5, packSeq = 12))
        packApi.removeResult = { ApiResult.HttpError(404, "pack_item_not_found", "gone") }

        assertThat(repository.remove(5)).isEqualTo(PackRepository.RemoveResult.REMOVED)

        assertThat(heldIds()).isEmpty()
    }

    @Test
    fun `a removal nobody answered removes nothing`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyItem(packItemDto(id = 5, packSeq = 12))
        packApi.removeResult = { ApiResult.NetworkError(IllegalStateException("offline")) }

        assertThat(repository.remove(5)).isEqualTo(PackRepository.RemoveResult.FAILED)

        assertThat(heldIds()).containsExactly(5L)
    }

    // -- Adding -----------------------------------------------------------------

    @Test
    fun `an add uploads the original bytes as a photo with no preview and claims them`() =
        runTest(dispatcher) {
            val repository = repository()
            val bytes = webp(size = 300)
            var uploaded: ByteArray? = null
            var uploadedMime: String? = null
            attachmentApi.uploadHandler = { file, mime, _ ->
                uploaded = file.readBytes()
                uploadedMime = mime
                ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(id = 71)))
            }

            val result = repository.add(PackPicture.Encoded(bytes, "image/webp", 512, 384), "  party cat ")

            assertThat(result).isEqualTo(PackRepository.AddResult.ADDED)
            // BYTE FOR BYTE what was picked: no downscale, no JPEG — the
            // photo path's preparation is never on this road.
            assertThat(uploaded).isEqualTo(bytes)
            assertThat(uploadedMime).isEqualTo("image/webp")
            assertThat(attachmentApi.uploadedMetadata.single()).isEqualTo(Triple("photo", 512, 384))
            // And no preview: a preview is a JPEG.
            assertThat(attachmentApi.calls).containsExactly("upload")
            assertThat(packApi.added).containsExactly(71L to "party cat")
            assertThat(heldIds()).containsExactly(5L)
        }

    @Test
    fun `the answer to this device's own add moves no cursor`() = runTest(dispatcher) {
        val repository = repository()
        settings.setPackCursor(3)

        repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null)

        assertThat(heldIds()).hasSize(1)
        assertThat(settings.current.packCursor).isEqualTo(3)
    }

    @Test
    fun `an upload the sweep took is uploaded again once and then claimed`() = runTest(dispatcher) {
        val repository = repository()
        var uploads = 0
        attachmentApi.uploadHandler = { _, _, _ ->
            uploads += 1
            ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(id = 70L + uploads)))
        }
        packApi.addResult = { attachmentId, label ->
            if (attachmentId == 71L) {
                ApiResult.HttpError(410, "attachment_expired", "swept")
            } else {
                ApiResult.Ok(
                    PackItemResponse(packItemDto(id = 5, packSeq = 12, attachmentId = attachmentId, label = label)),
                )
            }
        }

        val result = repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), "party cat")

        // By itself, and without a word: nobody is asked to try again.
        assertThat(result).isEqualTo(PackRepository.AddResult.ADDED)
        assertThat(uploads).isEqualTo(2)
        // The SECOND upload is the one claimed, with the same label.
        assertThat(packApi.added).containsExactly(71L to "party cat", 72L to "party cat").inOrder()
        assertThat(heldIds()).containsExactly(5L)
    }

    @Test
    fun `a second expiry is the one that is shown`() = runTest(dispatcher) {
        val repository = repository()
        var uploads = 0
        attachmentApi.uploadHandler = { _, _, _ ->
            uploads += 1
            ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(id = 70L + uploads)))
        }
        packApi.addResult = { _, _ -> ApiResult.HttpError(410, "attachment_expired", "swept") }

        val result = repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null)

        assertThat(result).isEqualTo(PackRepository.AddResult.FAILED)
        // Once more, and no more than once.
        assertThat(uploads).isEqualTo(2)
        assertThat(packApi.added).hasSize(2)
        assertThat(heldIds()).isEmpty()
    }

    @Test
    fun `a label over 64 characters is refused before any request and never cut`() = runTest(dispatcher) {
        val repository = repository()

        val result = repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), "a".repeat(65))

        assertThat(result).isEqualTo(PackRepository.AddResult.LABEL_TOO_LONG)
        assertThat(attachmentApi.calls).isEmpty()
        assertThat(packApi.added).isEmpty()
    }

    @Test
    fun `a label of 64 emoji is 64 characters and goes up whole`() = runTest(dispatcher) {
        val repository = repository()
        attachmentApi.uploadHandler = { _, _, _ ->
            ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(id = 71)))
        }
        val label = "\uD83D\uDE3A".repeat(64)

        val result = repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), " $label ")

        assertThat(result).isEqualTo(PackRepository.AddResult.ADDED)
        assertThat(packApi.added).containsExactly(71L to label)
    }

    @Test
    fun `a picture over the ceiling is refused before anything is uploaded`() = runTest(dispatcher) {
        val repository = repository()

        val result = repository.add(
            PackPicture.Encoded(webp(size = MAX_BYTES.toInt() + 1), "image/webp", 512, 512),
            null,
        )

        assertThat(result).isEqualTo(PackRepository.AddResult.TOO_LARGE)
        assertThat(attachmentApi.calls).isEmpty()
        assertThat(packApi.added).isEmpty()
    }

    @Test
    fun `a full pack is refused before anything is uploaded`() = runTest(dispatcher) {
        val repository = repository()
        repeat(MAX_ITEMS) { index ->
            repository.applyItem(packItemDto(id = 1L + index, packSeq = 10L + index, size = 999))
        }

        val result = repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null)

        assertThat(result).isEqualTo(PackRepository.AddResult.FULL)
        assertThat(attachmentApi.calls).isEmpty()
    }

    @Test
    fun `the server's own refusals are said the same way`() = runTest(dispatcher) {
        val repository = repository()

        packApi.addResult = { _, _ -> ApiResult.HttpError(409, "pack_full", "full") }
        assertThat(repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null))
            .isEqualTo(PackRepository.AddResult.FULL)

        packApi.addResult = { _, _ -> ApiResult.HttpError(413, "pack_item_too_large", "big") }
        assertThat(repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null))
            .isEqualTo(PackRepository.AddResult.TOO_LARGE)

        packApi.addResult = { _, _ -> ApiResult.HttpError(400, "invalid_attachment", "no") }
        assertThat(repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null))
            .isEqualTo(PackRepository.AddResult.FAILED)

        assertThat(heldIds()).isEmpty()
    }

    @Test
    fun `a type a pack may not hold is never uploaded`() = runTest(dispatcher) {
        val repository = repository()

        val result = repository.add(PackPicture.Encoded(ByteArray(64), "image/jpeg", 64, 64), null)

        assertThat(result).isEqualTo(PackRepository.AddResult.FAILED)
        assertThat(attachmentApi.calls).isEmpty()
    }

    @Test
    fun `a server without packs is offered nothing`() = runTest(dispatcher) {
        val repository = repository()
        // `max_pack_items` absent from the family read.
        settings.setPackLimits(null, null)

        assertThat(settings.current.packLimits).isNull()
        assertThat(repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null))
            .isEqualTo(PackRepository.AddResult.FAILED)
        assertThat(attachmentApi.calls).isEmpty()
    }

    @Test
    fun `a 200 for bytes the pack already holds adds nothing`() = runTest(dispatcher) {
        val repository = repository()
        // Held at the SAME seq the server will answer with; this device's
        // copy of the bytes is a different size, so only the server knows.
        repository.applyItem(packItemDto(id = 5, packSeq = 12, attachmentId = 70, size = 999))
        packApi.addResult = { _, _ ->
            // The item that was there: its attachment id is NOT the one sent.
            ApiResult.Ok(PackItemResponse(packItemDto(id = 5, packSeq = 12, attachmentId = 70, size = 999)))
        }

        val result = repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null)

        assertThat(result).isEqualTo(PackRepository.AddResult.ALREADY_IN_PACK)
        assertThat(heldIds()).containsExactly(5L)
        assertThat(packDao.findById(5)!!.attachment?.id).isEqualTo(70)
    }

    @Test
    fun `a new add whose own frame arrives before the answer is still an add`() = runTest(dispatcher) {
        val repository = repository()
        attachmentApi.uploadHandler = { _, _, _ ->
            ApiResult.Ok(AttachmentResponse(FakeAttachmentApi.attachment(id = 71)))
        }
        // The server's order: the `pack_item` frame goes to every
        // connection, this one included, and only THEN is the 201 written.
        // So by the time the answer is read, the item is already held.
        packApi.addResult = { attachmentId, _ ->
            val item = packItemDto(id = 5, packSeq = 12, attachmentId = attachmentId)
            repository.applyItem(item)
            ApiResult.Ok(PackItemResponse(item))
        }

        val result = repository.add(PackPicture.Encoded(webp(), "image/webp", 64, 64), null)

        // "Already in family stickers" would be said about a sticker that
        // was added a moment ago. The 201 claims the upload this call made.
        assertThat(result).isEqualTo(PackRepository.AddResult.ADDED)
        assertThat(packDao.findById(5)!!.attachment?.id).isEqualTo(71)
    }

    @Test
    fun `bytes the pack visibly holds are not uploaded a second time`() = runTest(dispatcher) {
        val repository = repository()
        val bytes = webp(size = 200)
        serveBytes(bytes)
        repository.applyItem(packItemDto(id = 5, packSeq = 12, size = 200))

        val result = repository.add(PackPicture.Encoded(bytes, "image/webp", 64, 64), null)

        assertThat(result).isEqualTo(PackRepository.AddResult.ALREADY_IN_PACK)
        assertThat(attachmentApi.calls).doesNotContain("upload")
        assertThat(packApi.added).isEmpty()
    }

    // -- "Is this sticker in the pack?" ------------------------------------------

    @Test
    fun `holding is decided by size and type and then by the bytes`() = runTest(dispatcher) {
        val repository = repository()
        val bytes = webp(size = 200, fill = 3)
        serveBytes(bytes)
        repository.applyItem(packItemDto(id = 5, packSeq = 12, size = 200))

        var asked = 0
        // A different size: not held, and the bytes are never even read.
        assertThat(repository.holding("image/webp", 201) { asked++; bytes }).isNull()
        // The same size under another type: likewise.
        assertThat(repository.holding("image/png", 200) { asked++; bytes }).isNull()
        assertThat(asked).isEqualTo(0)

        // Same size and type, different bytes: not held.
        assertThat(repository.holding("image/webp", 200) { webp(size = 200, fill = 4) }).isNull()
        // The very bytes: held.
        assertThat(repository.holding("image/webp", 200) { bytes }?.id).isEqualTo(5)
    }

    // -- The pictures and the send copy -----------------------------------------

    @Test
    fun `a pack picture is fetched as the original and kept`() = runTest(dispatcher) {
        val repository = repository()
        val bytes = webp(size = 200)
        serveBytes(bytes)
        // `has_preview` TRUE, which dedup can make it: still the original.
        val item = packItemDto(id = 5, packSeq = 12, size = 200)
        val picture = item.attachment!!.copy(hasPreview = true)

        val first = repository.fileFor(picture)
        val second = repository.fileFor(picture)

        assertThat(first!!.readBytes()).isEqualTo(bytes)
        assertThat(second).isEqualTo(first)
        // Never `/preview`, and only once: the file is kept under the id.
        assertThat(attachmentApi.downloads).containsExactly(picture.id to false)
        // Under filesDir: a cache the system may reclaim is no place for
        // the bytes a sticker is SENT from.
        assertThat(first.absolutePath)
            .startsWith(RuntimeEnvironment.getApplication().filesDir.absolutePath)
    }

    @Test
    fun `one picture the server refuses does not stop the prefetch of the rest`() = runTest(dispatcher) {
        val repository = repository()
        val bytes = webp(size = 200)
        (1L..3L).forEach { id -> repository.applyItem(packItemDto(id = id, packSeq = 10 + id, size = 200)) }
        // Item 1's picture is refused — removed, say, by a tombstone this
        // device has not applied yet. The pass starts from the lowest id
        // every time, so stopping here would stop here on every resync.
        attachmentApi.downloadHandler = { id, _, destination ->
            if (id == 71L) {
                ApiResult.HttpError(404, "attachment_not_found", "gone")
            } else {
                destination.parentFile?.mkdirs()
                destination.writeBytes(bytes)
                ApiResult.Ok(Unit)
            }
        }

        repository.prefetch()

        assertThat(attachmentApi.downloads.map { it.first }).containsExactly(71L, 72L, 73L).inOrder()
        // Held now, so a send from either needs no network.
        attachmentApi.downloadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        assertThat(repository.stagedCopy(packDao.findById(2)!!)).isNotNull()
        assertThat(repository.stagedCopy(packDao.findById(3)!!)).isNotNull()
    }

    @Test
    fun `a prefetch nobody answers stops at the first picture`() = runTest(dispatcher) {
        val repository = repository()
        (1L..3L).forEach { id -> repository.applyItem(packItemDto(id = id, packSeq = 10 + id)) }
        attachmentApi.downloadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }

        repository.prefetch()

        // A dead network fails them all alike; one request says so.
        assertThat(attachmentApi.downloads).containsExactly(71L to false)
    }

    @Test
    fun `the copy a send takes is the original bytes with no preview`() = runTest(dispatcher) {
        val repository = repository()
        val bytes = webp(size = 321, fill = 9)
        serveBytes(bytes)
        repository.applyItem(packItemDto(id = 5, packSeq = 12, size = 321))

        val prepared = repository.stagedCopy(packDao.findById(5)!!)!!

        assertThat(prepared.file.readBytes()).isEqualTo(bytes)
        assertThat(prepared.mime).isEqualTo("image/webp")
        assertThat(prepared.kind).isEqualTo("photo")
        assertThat(prepared.previewJpeg).isNull()
        assertThat(prepared.file.name).endsWith(".webp")
        // A COPY: the pack keeps its own, so a second send needs no fetch.
        val kept = repository.fileFor(packDao.findById(5)!!.attachment!!)!!
        assertThat(kept.absolutePath).isNotEqualTo(prepared.file.absolutePath)
        assertThat(kept.isFile).isTrue()
        prepared.file.delete()
    }

    @Test
    fun `a sticker whose bytes cannot be had cannot be staged`() = runTest(dispatcher) {
        val repository = repository()
        attachmentApi.downloadHandler = { _, _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        repository.applyItem(packItemDto(id = 5, packSeq = 12))

        assertThat(repository.stagedCopy(packDao.findById(5)!!)).isNull()
    }

    @Test
    fun `removing an item drops its bytes`() = runTest(dispatcher) {
        val repository = repository()
        serveBytes(webp())
        val item = packItemDto(id = 5, packSeq = 12)
        repository.applyItem(item)
        val file: File = repository.fileFor(item.attachment!!)!!
        assertThat(file.isFile).isTrue()

        repository.applyItem(packTombstone(id = 5, packSeq = 14))

        assertThat(file.exists()).isFalse()
    }

    // -- Recents ------------------------------------------------------------------

    @Test
    fun `recently used is this device's own list, newest first`() = runTest(dispatcher) {
        val repository = repository()

        repository.noteUsed(3)
        repository.noteUsed(8)
        repository.noteUsed(3)

        assertThat(settings.current.packRecents).containsExactly(3L, 8L).inOrder()
    }

    @Test
    fun `recents drop what the pack no longer holds and are capped`() {
        val held = listOf(1L, 2L, 3L)

        assertThat(PackRules.recents(listOf(3L, 9L, 1L), held) { it }).containsExactly(3L, 1L).inOrder()
        val many = (1L..40L).fold(emptyList<Long>()) { list, id -> PackRules.used(list, id) }
        assertThat(many).hasSize(PackRules.MAX_RECENTS)
        assertThat(many.first()).isEqualTo(40)
    }
}
