/*
 * PackRepository.kt
 * Family Connect (Android)
 *
 * The family's STICKER PACK (docs/protocol.md, "Sticker pack"): the
 * pictures anybody in the family has added, which everybody in it may send
 * in a chat. Cached locally so the panel draws instantly and a sticker can
 * be sent with no network — the items in Room, and the bytes on disk under
 * the attachment id, which never names different bytes.
 *
 * "PACK", NOT "STICKER", in every name here. In this codebase "sticker" has
 * always meant a note on the board (NoteEntity, BoardRepository); this is
 * the other thing, and the wire keeps the two apart the same way.
 *
 * THE SYNC IS THE BOARD'S, UNCHANGED, and deliberately so — no client
 * learns a new idea:
 *
 *  - Every apply goes through ONE guarded path, [applyItem]: an item is
 *    written only when the incoming `pack_seq` is greater than the one held.
 *  - A full read REPLACES what is held, except items held above its mark.
 *  - A removal is remembered (the gone set): ids are never reused, so an
 *    older copy arriving late can never bring an item back.
 *  - The cursor moves in three ways and no others: a full read sets it to
 *    its `max_pack_seq`, a catch-up page to the highest `pack_seq` on the
 *    page, and a `pack_item` frame to its own `pack_seq` — the frame ONLY
 *    ONCE THIS CONNECTION HAS CAUGHT UP ([caughtUpOn]). A frame that jumped
 *    the cursor before the catch-up read it would have the catch-up start
 *    past everything it was there to fetch.
 *  - It only ever moves FORWARD ([advanceCursor]), except back to the start
 *    when it is provably another family's.
 *
 * TOMBSTONES are not stored. The server keeps one so its change feed can
 * say "gone"; a client that has been told deletes its row and its bytes.
 *
 * A BLOCK HIDES NOTHING HERE. A blocked member's sticker MESSAGE is hidden
 * like any message of theirs; their PACK ITEMS are the family's pictures
 * and not their words, so nothing in this file reads the block list.
 *
 * iOS counterpart: the pack section of ChatSyncCoordinator.swift.
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.data.db.GonePackItemEntity
import me.nettrash.familyconnect.data.db.PackDao
import me.nettrash.familyconnect.data.db.PackItemEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.PackApi
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.PackItemDto
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.TimeFormat
import java.io.File
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

/**
 * The pack's two ceilings, as `GET /families/mine` reported them. Its
 * EXISTENCE is the capability check: null means this server predates packs,
 * and nothing about stickers is offered there.
 */
data class PackLimits(val maxItems: Int, val maxItemBytes: Long)

val SettingsState.packLimits: PackLimits?
    get() = if (packMaxItems > 0 && packMaxItemBytes > 0) {
        PackLimits(packMaxItems, packMaxItemBytes)
    } else {
        null
    }

/** The rules of the pack that are decisions and not requests, held where a test can reach them. */
object PackRules {

    /**
     * Who may remove an item: WHOEVER ADDED IT, or THE FAMILY OWNER.
     *
     * A permission shape the board does not have — a note is its author's
     * alone — and the reason is the pack's other difference: it is the
     * family's, and a member who has left, or deleted their account, leaves
     * their stickers behind. Under an author-only rule nobody could ever
     * remove those. The server enforces it (`not_pack_item_author`); this is
     * what decides whether the button is drawn.
     */
    fun canRemove(addedBy: Long, myUserId: Long?, isOwner: Boolean): Boolean =
        isOwner || (myUserId != null && addedBy == myUserId)

    /**
     * The panel's "recently used" row: the ids this device sent most
     * recently, newest first, MINUS whatever the pack no longer holds —
     * a removed sticker must not linger as an empty cell.
     */
    fun <T> recents(recentIds: List<Long>, held: List<T>, id: (T) -> Long): List<T> {
        val byId = held.associateBy(id)
        return recentIds.mapNotNull { byId[it] }
    }

    /** The list after [itemId] was just used: it first, once, and no more than [MAX_RECENTS]. */
    fun used(recentIds: List<Long>, itemId: Long): List<Long> =
        (listOf(itemId) + recentIds.filter { it != itemId }).take(MAX_RECENTS)

    /** Two rows of the panel's grid at its narrowest. */
    const val MAX_RECENTS = 16
}

@Singleton
class PackRepository @Inject constructor(
    @param:ApplicationContext private val context: Context,
    private val packApi: PackApi,
    private val attachmentApi: AttachmentApi,
    private val packDao: PackDao,
    private val settings: SettingsRepository,
    private val socket: ChatSocket,
    @param:AppScope private val scope: CoroutineScope,
) {

    /**
     * The connection ([ChatSocket.connectionSerial]) a catch-up pass has
     * COMPLETED on, or -1 while none has. A frame moves the cursor only
     * while this names the connection that is open now; a reconnect makes
     * a new serial, so the answer goes back to "not yet" by itself.
     */
    @Volatile
    private var caughtUpOn = -1L

    /** One catch-up pass at a time: two resyncs can start at the same moment. */
    private val syncGuard = Mutex()

    /** The cursor's read-then-write, so a frame and a page cannot undo each other. */
    private val cursorGuard = Mutex()

    init {
        scope.launch {
            socket.frames.collect { frame ->
                if (frame is ServerFrame.PackItem) {
                    // Every member gets this frame, a blocker of the adder
                    // included — nothing is filtered (see the header).
                    val changed = applyItem(frame.item)
                    // "A pack_item frame [moves the cursor] to its own
                    // pack_seq — the frame only once this connection has
                    // caught up." Whether or not the item itself was news:
                    // what the frame says about the cursor is where the
                    // pack has got to, not whether this device had it.
                    if (caughtUpOn == socket.connectionSerial) advanceCursor(frame.item.packSeq)
                    if (changed) {
                        // The picture too, now, while there is a network:
                        // a sticker somebody added this afternoon should be
                        // sendable on the train home.
                        frame.item.attachment?.let { picture -> scope.launch { fileFor(picture) } }
                    }
                }
            }
        }
    }

    /** What became of an add. Each one is said to the person who asked. */
    enum class AddResult {
        ADDED,

        /**
         * The pack already holds those bytes — the server answered `200`
         * with the item that was there, or this device found the match
         * itself. Not an error and not two stickers.
         */
        ALREADY_IN_PACK,

        /** `pack_full`, or this device could already count to the ceiling. */
        FULL,

        /** `pack_item_too_large`, or over the ceiling before the upload. */
        TOO_LARGE,

        /** The label is over the protocol's 64 characters. Nothing was sent. */
        LABEL_TOO_LONG,

        /** Anything else: no network, a refused upload, a server without packs. */
        FAILED,
    }

    enum class RemoveResult { REMOVED, NOT_ALLOWED, FAILED }

    /** Picture fetches in flight, keyed by attachment id. */
    private val fetches = HashMap<Long, Deferred<Fetched>>()
    private val guard = Mutex()

    /** One prefetch pass at a time; a flapping socket must not start a second. */
    private val prefetchGuard = Mutex()

    /** The pack in the order it was added, which is the order a panel shows it in. */
    fun observeItems(): Flow<List<PackItemEntity>> = packDao.observeItems()

    private suspend fun packCursor(): Long = settings.state.first().packCursor

    /** Move the cursor to [seq] if that is forward. It never goes back this way. */
    private suspend fun advanceCursor(seq: Long) {
        cursorGuard.withLock {
            if (seq > packCursor()) settings.setPackCursor(seq)
        }
    }

    /**
     * The connection a catch-up pass is about to be run FOR, or null when
     * no socket is open — a pass over plain REST catches the rows up and
     * says nothing about any connection.
     *
     * Asked BEFORE `GET /families/mine`, not when the pass starts: the mark
     * that read reports is what the pass catches up TO, so it has to have
     * been taken on the connection the pass will vouch for. Taken before a
     * reconnect, it would miss whatever changed while the wire was down —
     * and the new connection would be called caught up to a stale mark.
     */
    fun connectionNow(): Long? {
        // The serial first: a reconnect between the two reads then leaves
        // an old number beside `Open`, which no pass can complete on.
        val serial = socket.connectionSerial
        return serial.takeIf { socket.state.value == SocketState.Open }
    }

    // -- Sync -----------------------------------------------------------------

    /**
     * Apply one item under the per-item seq guard. Returns whether anything
     * changed.
     */
    suspend fun applyItem(item: PackItemDto): Boolean {
        // A TOMBSTONE IS THE LAST WORD, as on the board: item ids are never
        // reused, so an item this device has seen removed — by a tombstone,
        // by a full read that left it out, or by its own DELETE — is never
        // brought back by an older copy arriving late.
        if (packDao.isGone(item.id)) return false
        val existing = packDao.findById(item.id)
        // "An item is written only when the incoming pack_seq is greater
        // than the one held." An equal seq is the same server state, and
        // unlike a note a pack item has no later fields an old row could be
        // missing — so equal is refused too.
        if (existing != null && item.packSeq <= existing.packSeq) return false

        if (item.isTombstone) {
            // Remembered even for an item this device never HELD: the copy
            // the tombstone is about may still be on its way.
            packDao.remember(GonePackItemEntity(item.id))
            if (existing != null) {
                packDao.delete(item.id)
                existing.attachment?.let { dropBytes(it.id) }
            }
            return existing != null
        }

        val addedBy = item.addedBy
        val attachment = item.attachment
        if (addedBy == null || attachment == null) {
            // A live item missing its picture is a server bug; dropping it
            // beats an empty cell in the panel.
            return false
        }
        packDao.upsert(
            PackItemEntity(
                id = item.id,
                addedBy = addedBy,
                // Stored WITHOUT the `sticker` flag whatever arrived: the
                // flag is a message's, never a pack item's.
                attachmentJson = AttachmentsCodec.encode(listOf(attachment.copy(sticker = null))),
                // "Never null or empty" on the wire; an empty one that
                // slipped through is still no label.
                label = item.label?.trim()?.ifEmpty { null },
                createdAt = item.createdAt?.let(TimeFormat::parseTimestamp)
                    ?: existing?.createdAt
                    ?: System.currentTimeMillis(),
                packSeq = item.packSeq,
            ),
        )
        return true
    }

    /** Full pack read — a device that holds no pack. */
    suspend fun loadPack(): Boolean {
        val pack = packApi.getPack().okOrNull() ?: return false
        // "A client ignores a full read whose max_pack_seq is below one it
        // has already applied": such a read was taken before changes this
        // device already has, and REPLACING with it would undo them —
        // held or not, since an item it lists may be one this device has
        // since seen removed by a read that left it out. (A cursor that
        // outlived its family is put back to the start in [catchUp], so it
        // cannot veto the new family's first read from here.)
        if (pack.maxPackSeq < packCursor()) return true
        // It REPLACES what is held: the read never returns tombstones, so an
        // item it leaves out is an item that is gone. One held ABOVE the
        // read's mark arrived after the read was taken, and stays —
        // `max_pack_seq` is read before the items and a family's pack
        // changes commit in seq order, so the mark is a promise.
        val listed = pack.items.map { it.id }
        // Read BEFORE they are dropped, and remembered as gone for the
        // reason a tombstone is.
        val dropped = packDao.idsNotListed(pack.maxPackSeq, listed)
        dropped.forEach { id ->
            packDao.findById(id)?.attachment?.let { dropBytes(it.id) }
            packDao.remember(GonePackItemEntity(id))
        }
        packDao.deleteNotListed(pack.maxPackSeq, listed)
        pack.items.forEach { applyItem(it) }
        // A full read sets the cursor to its max_pack_seq — forward only:
        // a frame may have carried it further while the read was applied.
        advanceCursor(pack.maxPackSeq)
        return true
    }

    /**
     * The pack's step of the reconnect resync, after `GET /families/mine`
     * said where the server is: a device that holds no pack reads the whole
     * of it; one that does, and whose cursor is below [serverMaxSeq], loops
     * the change feed — tombstones included, until a short page.
     *
     * [serverMaxSeq] is 0 when the field was absent: the pack has never
     * been written to, or the server has no packs at all. Either way there
     * is nothing to read and no request is made.
     *
     * [connection] is what [connectionNow] answered before the family read
     * that produced [serverMaxSeq]. A pass that completes while that is
     * still the open connection marks it caught up, and from then on a
     * `pack_item` frame moves the cursor.
     */
    suspend fun catchUpPack(serverMaxSeq: Long, connection: Long? = null) {
        syncGuard.withLock {
            // Only a pass that COMPLETED, and only for the connection it
            // was started for: one a reconnect has overtaken read a mark
            // from before the new connection, and must not vouch for it.
            if (catchUp(serverMaxSeq) && connection != null && connection == socket.connectionSerial) {
                caughtUpOn = connection
            }
        }
    }

    /** The pass itself. False when a request failed and the pack may still be behind. */
    private suspend fun catchUp(serverMaxSeq: Long): Boolean {
        val cursor = packCursor()
        if (packDao.count() == 0) {
            return when {
                // Untouched. A cursor left over from somewhere else means
                // nothing here, so it goes back to the start.
                serverMaxSeq <= 0L -> {
                    if (cursor != 0L) cursorGuard.withLock { settings.setPackCursor(0L) }
                    true
                }
                // Level with the server and holding nothing: every sticker
                // was removed, and this device has seen it happen.
                serverMaxSeq == cursor -> true
                // One full read beats replaying the history of every item
                // that ever existed.
                else -> {
                    // A cursor ABOVE the server's own maximum, with nothing
                    // held, is not this pack's: pack seqs are server-wide,
                    // and the family just joined may sit far below the mark
                    // the last one left. Back to the start, or the read
                    // below would be ignored as older than what is applied.
                    if (cursor > serverMaxSeq) cursorGuard.withLock { settings.setPackCursor(0L) }
                    loadPack()
                }
            }
        }
        if (serverMaxSeq <= cursor) return true
        var after = cursor
        while (true) {
            val page = packApi.getPackChanges(after, PACK_PAGE).okOrNull()?.items ?: return false
            page.forEach { applyItem(it) }
            val pageMax = page.maxOfOrNull { it.packSeq }
            if (pageMax != null) {
                advanceCursor(pageMax)
                after = pageMax
            }
            if (page.size < PACK_PAGE) return true
        }
    }

    // -- The pictures ----------------------------------------------------------

    /**
     * The ORIGINAL bytes of a pack item, as a file — downloading them if
     * this device does not hold them yet. Null when they could not be
     * fetched; the caller draws nothing and says so if it matters.
     *
     * Always `GET /attachments/{id}`, never `/preview`, whatever
     * `has_preview` says: a preview is a JPEG — no transparency, one frame
     * — and the flag can be true by dedup inheritance.
     *
     * Under filesDir and not the cache, unlike AttachmentRepository: the
     * system reclaims a cache whenever it likes, and these are the bytes a
     * sticker is SENT from. A pack the phone quietly forgot would be a
     * panel that cannot send on the train.
     */
    suspend fun fileFor(attachment: AttachmentDto): File? = fetch(attachment).file

    /**
     * What became of asking for one picture. [answered] is what a caller
     * with MORE pictures to ask for needs and [file] cannot say: a null
     * file from a server that refused is that one picture's problem, and a
     * null file from a network nobody answered on is every picture's.
     */
    private class Fetched(val file: File?, val answered: Boolean)

    private suspend fun fetch(attachment: AttachmentDto): Fetched {
        val file = bytesFile(attachment.id)
        if (withContext(Dispatchers.IO) { file.isFile && file.length() > 0 }) return Fetched(file, answered = true)
        // One download per picture however many cells ask for it.
        val running = guard.withLock {
            fetches.getOrPut(attachment.id) {
                scope.async {
                    withContext(Dispatchers.IO) {
                        file.parentFile?.mkdirs()
                        when (attachmentApi.download(attachment.id, preview = false, destination = file)) {
                            is ApiResult.Ok -> Fetched(file.takeIf { it.isFile }, answered = true)
                            is ApiResult.HttpError -> Fetched(null, answered = true)
                            is ApiResult.NetworkError -> Fetched(null, answered = false)
                        }
                    }
                }
            }
        }
        return try {
            running.await()
        } finally {
            guard.withLock { fetches.remove(attachment.id) }
        }
    }

    /**
     * Fetch every picture this device holds an item for and no bytes of —
     * after a resync, in the background, so the pack is whole before the
     * network goes rather than when somebody first opens the panel without
     * one.
     */
    fun prefetchInBackground() {
        scope.launch { prefetch() }
    }

    /**
     * The pass itself, apart from the launch so a test can wait for it.
     *
     * Stops when NOBODY ANSWERED: a dead network fails them all alike, and
     * asking two hundred more times proves it two hundred times. A picture
     * the server REFUSED is skipped instead — an item removed whose
     * tombstone this device has not applied yet answers 404, and the pass
     * starts from the lowest id every time, so stopping there would leave
     * every later sticker unfetched on every resync, and unsendable on the
     * train, for as long as that one item sat in the way.
     */
    internal suspend fun prefetch() {
        if (!prefetchGuard.tryLock()) return
        try {
            for (item in packDao.items()) {
                val picture = item.attachment ?: continue
                val fetched = fetch(picture)
                if (fetched.file == null && !fetched.answered) break
            }
        } finally {
            prefetchGuard.unlock()
        }
    }

    private fun bytesFile(attachmentId: Long): File =
        File(bytesDirectory(context), attachmentId.toString())

    private suspend fun dropBytes(attachmentId: Long) {
        withContext(Dispatchers.IO) { bytesFile(attachmentId).delete() }
    }

    /** Keep bytes this device already has under the id the pack names them by. */
    private suspend fun keepBytes(attachmentId: Long, bytes: ByteArray) {
        withContext(Dispatchers.IO) {
            val file = bytesFile(attachmentId)
            if (file.isFile && file.length() == bytes.size.toLong()) return@withContext
            file.parentFile?.mkdirs()
            val part = File(file.parentFile, file.name + ".part")
            runCatching {
                part.writeBytes(bytes)
                if (!part.renameTo(file)) part.delete()
            }.onFailure { part.delete() }
        }
    }

    /**
     * The pack item that IS these bytes, or null when the pack does not
     * hold them.
     *
     * "Holds it" is decided by the CLIENT from bytes it already has
     * (docs/protocol.md, "Sticker pack"): an item whose `attachment.size`
     * and `mime` match and whose bytes are the same. Nothing on the wire
     * names the item a message was sent from, and nothing needs to — a
     * wrong "no" here only offers "Add to family stickers" once too often,
     * and the server answers that `200` with the item that was there.
     *
     * [bytes] is asked for only when some item matches on size and type,
     * which for most stickers in most chats is never.
     */
    suspend fun holding(mime: String, size: Long, bytes: suspend () -> ByteArray?): PackItemEntity? {
        val candidates = packDao.items().filter { item ->
            val picture = item.attachment
            picture != null && picture.size == size && picture.mime == mime
        }
        if (candidates.isEmpty()) return null
        val mine = bytes() ?: return null
        for (candidate in candidates) {
            val picture = candidate.attachment ?: continue
            val file = fileFor(picture) ?: continue
            val theirs = withContext(Dispatchers.IO) { runCatching { file.readBytes() }.getOrNull() }
            if (theirs != null && theirs.contentEquals(mine)) return candidate
        }
        return null
    }

    // -- Adding and removing ---------------------------------------------------

    /**
     * Add a picture to the family's pack. ANY member may.
     *
     * The bytes go up AS THEY ARE — `POST /attachments?kind=photo` with
     * their own media type and NO preview — and are then claimed by
     * `POST /families/mine/pack`. Nothing here, and nothing on the way
     * here, runs MediaPrep: a JPEG re-encode would cost a sticker its
     * transparency and its animation (docs/protocol.md, "A sticker is NOT
     * prepared before upload").
     *
     * The two ceilings are checked HERE first, where the person is
     * choosing, so the refusal is a sentence beside the picker rather than
     * a rejected request; the server's own `pack_full` and
     * `pack_item_too_large` are handled the same way for the race this
     * cannot see.
     */
    suspend fun add(picture: PackPicture.Encoded, label: String?): AddResult {
        val limits = settings.state.first().packLimits ?: return AddResult.FAILED
        if (!PackPicture.isStickerType(picture.mime)) return AddResult.FAILED
        // Refused, never cut: the panel says so before it ever asks, and
        // this is the same answer for anything that got past it — a label
        // silently shortened is not the one somebody wrote.
        if (!PackPicture.labelFits(label)) return AddResult.LABEL_TOO_LONG
        val wireLabel = label?.trim()?.ifEmpty { null }
        if (picture.bytes.size > limits.maxItemBytes) return AddResult.TOO_LARGE
        // Before the count: adding a sticker the pack already has is not an
        // error even when the pack is full — it is simply already there.
        holding(picture.mime, picture.bytes.size.toLong()) { picture.bytes }
            ?.let { return AddResult.ALREADY_IN_PACK }
        if (packDao.count() >= limits.maxItems) return AddResult.FULL

        val upload = withContext(Dispatchers.IO) {
            val directory = File(context.cacheDir, UPLOAD_DIR).apply { mkdirs() }
            File(directory, "pack-${UUID.randomUUID()}").takeIf {
                runCatching { it.writeBytes(picture.bytes) }.isSuccess
            }
        } ?: return AddResult.FAILED
        try {
            // `attachment_expired` on the claim: the unclaimed sweep took
            // the upload before the claim reached it, "and the client
            // uploads again" (docs/protocol.md, "The pack") — ONCE, by
            // itself. Only a second failure is somebody's to read about.
            var uploadedAgain = false
            while (true) {
                val uploaded = attachmentApi.upload(
                    file = upload,
                    mime = picture.mime,
                    kind = AttachmentDto.KIND_PHOTO,
                    width = picture.width,
                    height = picture.height,
                    durationMs = null,
                )
                val attachment = uploaded.okOrNull()?.attachment ?: return AddResult.FAILED
                when (val claimed = packApi.addItem(attachment.id, wireLabel)) {
                    is ApiResult.Ok -> {
                        val item = claimed.value.item
                        // A `200` names the item the pack ALREADY had, and the
                        // status itself does not travel this far (ApiResult.Ok
                        // carries none). What does is the attachment id: a
                        // `201` claims the very upload this call just made,
                        // while a `200` dropped it and answers with the id the
                        // pack already had (docs/protocol.md, "The pack").
                        //
                        // NOT "did this device hold the item": the server fans
                        // the `pack_item` frame out to this connection too,
                        // BEFORE it writes the 201, so the collector above can
                        // have stored a brand-new item by the time the answer
                        // is read — and a sticker just added would be reported
                        // as one that was already there.
                        val already = item.attachment?.id != attachment.id
                        // The bytes are in hand; keeping them saves a download
                        // of what this device just sent.
                        item.attachment?.let { keepBytes(it.id, picture.bytes) }
                        // Applied under the per-item guard, and moving NO
                        // cursor: the answer to this device's own POST says
                        // nothing about another item's lower seq.
                        applyItem(item)
                        return if (already) AddResult.ALREADY_IN_PACK else AddResult.ADDED
                    }
                    is ApiResult.HttpError -> when (claimed.code) {
                        "pack_full" -> return AddResult.FULL
                        "pack_item_too_large" -> return AddResult.TOO_LARGE
                        "attachment_expired" -> {
                            if (uploadedAgain) return AddResult.FAILED
                            uploadedAgain = true
                        }
                        else -> return AddResult.FAILED
                    }
                    is ApiResult.NetworkError -> return AddResult.FAILED
                }
            }
        } finally {
            withContext(Dispatchers.IO) { upload.delete() }
        }
    }

    /**
     * "Add to family stickers", on a sticker somebody sent: upload the
     * MESSAGE's bytes again, unprepared, and claim them — the pack's own
     * flow (docs/protocol.md, "Sticker pack").
     */
    suspend fun addFromMessage(attachment: AttachmentDto, file: File): AddResult {
        val limits = settings.state.first().packLimits ?: return AddResult.FAILED
        val length = withContext(Dispatchers.IO) { file.length() }
        if (length > limits.maxItemBytes) return AddResult.TOO_LARGE
        val bytes = withContext(Dispatchers.IO) { runCatching { file.readBytes() }.getOrNull() }
            ?: return AddResult.FAILED
        // What the bytes ARE, not what the message said: the server checks
        // the same magic number against the declared type.
        val mime = PackPicture.sniff(bytes) ?: return AddResult.FAILED
        val size = PackPicture.dimensions(bytes)
        return add(
            PackPicture.Encoded(
                bytes = bytes,
                mime = mime,
                width = size?.first ?: attachment.width,
                height = size?.second ?: attachment.height,
            ),
            label = null,
        )
    }

    /**
     * Remove an item: whoever added it, or the family owner
     * ([PackRules.canRemove] decides the button; the server decides).
     *
     * Every message ever sent with that sticker keeps its own copy and goes
     * on drawing — removing an item breaks nothing that was sent.
     */
    suspend fun remove(id: Long): RemoveResult = when (val result = packApi.removeItem(id)) {
        is ApiResult.Ok -> {
            forget(id)
            RemoveResult.REMOVED
        }
        is ApiResult.HttpError -> when (result.code) {
            // "No such item in the caller's family": it is gone already —
            // removed from another device, whose tombstone this one has
            // not applied yet. The same end state as a 204.
            "pack_item_not_found" -> {
                forget(id)
                RemoveResult.REMOVED
            }
            "not_pack_item_author" -> RemoveResult.NOT_ALLOWED
            else -> RemoveResult.FAILED
        }
        is ApiResult.NetworkError -> RemoveResult.FAILED
    }

    /**
     * The third of the protocol's three doors to "gone": a tombstone, a
     * full read that left it out, and this client's own DELETE. The frame
     * that confirms it arrives later and finds the id already remembered.
     */
    private suspend fun forget(id: Long) {
        val existing = packDao.findById(id)
        packDao.remember(GonePackItemEntity(id))
        packDao.delete(id)
        existing?.attachment?.let { dropBytes(it.id) }
    }

    // -- Sending ---------------------------------------------------------------

    /**
     * A pack item's bytes, copied somewhere the send path may take
     * ownership of — the ORIGINAL bytes, with NO preview.
     *
     * This is the bypass of photo preparation, and it is structural rather
     * than a flag somebody could forget: the composer's photo path is
     * `MediaPrep.preparePhoto(uri)`, which decodes, scales to 2048 px and
     * writes JPEG; this never touches a Uri or a decoder at all. What goes
     * up is byte for byte what the pack holds, so the message's upload
     * hashes to the pack item's file and the copy costs the server nothing
     * (docs/protocol.md, "Sending one").
     *
     * Null when this device does not hold the bytes and cannot fetch them —
     * the one case a sticker cannot be sent offline.
     */
    suspend fun stagedCopy(item: PackItemEntity): MediaPrep.Prepared? {
        val picture = item.attachment ?: return null
        val source = fileFor(picture) ?: return null
        val copy = withContext(Dispatchers.IO) {
            // In the cache, where MediaStaging.adopt RENAMES rather than
            // copies — and with the extension the type names, since the
            // staged file's name is what a later share would carry.
            val directory = File(context.cacheDir, UPLOAD_DIR).apply { mkdirs() }
            val extension = if (picture.mime == PackPicture.MIME_PNG) "png" else "webp"
            File(directory, "sticker-${UUID.randomUUID()}.$extension").takeIf { target ->
                runCatching { source.copyTo(target, overwrite = true) }.isSuccess
            }
        } ?: return null
        return MediaPrep.Prepared(
            file = copy,
            mime = picture.mime,
            kind = AttachmentDto.KIND_PHOTO,
            width = picture.width,
            height = picture.height,
            durationMs = null,
            // None, and it must stay none: a preview is a JPEG.
            previewJpeg = null,
        )
    }

    /**
     * Remember that this device just sent [itemId], for the panel's
     * "recently used" row. Device-local and never on the wire: it says
     * something about a person's habits and nothing about the family's pack.
     */
    suspend fun noteUsed(itemId: Long) {
        val current = settings.state.first().packRecents
        val next = PackRules.used(current, itemId)
        if (next != current) settings.setPackRecents(next)
    }

    companion object {
        /** The change feed's largest page (docs/protocol.md: `limit` max 200). */
        const val PACK_PAGE = 200

        private const val BYTES_DIR = "pack"
        private const val UPLOAD_DIR = "pack-uploads"

        /**
         * Where the pack's pictures live. Named here rather than inside the
         * instance so the session wipe (AppModule's LocalDataWiper) can
         * delete them without building a repository.
         */
        fun bytesDirectory(context: Context): File = File(context.filesDir, BYTES_DIR)
    }
}
