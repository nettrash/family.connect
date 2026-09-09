/*
 * BoardViewModel.kt
 * Family Connect (Android)
 *
 * The board screen's state: the notes themselves (straight from Room, so
 * the wall draws offline and updates the moment a frame lands), plus the
 * names needed to say who wrote what.
 *
 * Every mutation is fire-and-forget into the repository, which is where the
 * seq guard lives — the screen never writes to the cache itself.
 *
 * iOS counterpart: BoardView reads @Query directly and calls the
 * coordinator; SwiftUI needs no separate view model here.
 */

package me.nettrash.familyconnect.ui.board

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.repo.BoardRepository
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AttachmentApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.CoroutineScope
import android.net.Uri
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.util.BoardBadge
import me.nettrash.familyconnect.util.badgeMarks
import me.nettrash.familyconnect.util.marks
import me.nettrash.familyconnect.util.resolvedDisplayNames
import javax.inject.Inject

@HiltViewModel
class BoardViewModel @Inject constructor(
    /** For `getString` only — see the note on SettingsViewModel. */
    @param:ApplicationContext private val appContext: Context,
    private val boardRepository: BoardRepository,
    private val familyRepository: FamilyRepository,
    memberDao: MemberDao,
    private val settings: SettingsRepository,
    private val attachmentApi: AttachmentApi,
    private val mediaPrep: MediaPrep,
    @param:AppScope private val appScope: CoroutineScope,
) : ViewModel() {

    /** True while a picture is being prepared, uploaded and pinned. */
    private val _pinning = MutableStateFlow(false)
    val pinning: StateFlow<Boolean> = _pinning

    /** Set when a pin failed, cleared once the screen has said so. */
    private val _pinFailed = MutableStateFlow(false)
    val pinFailed: StateFlow<Boolean> = _pinFailed

    fun clearPinFailure() {
        _pinFailed.value = false
    }

    /**
     * Prepare, upload, pin — in that order, because the note may not exist
     * until the picture does: the server claims the upload inside the same
     * transaction that writes the note, and a note pointing at nothing is
     * the one state this must never produce (docs/protocol.md, "Board").
     *
     * The picture is downscaled first, by the same MediaPrep a message
     * uses: a wall tile is 220.dp, and shipping twelve megapixels to draw
     * it would cost the family's data for pixels nobody sees.
     *
     * APP scope, not viewModelScope: leaving the board must not cancel an
     * upload in flight, exactly as leaving a chat must not.
     */
    fun pinPhoto(uri: Uri, slot: Int) {
        if (_pinning.value) return
        _pinning.value = true
        appScope.launch {
            try {
                val prepared = runCatching { mediaPrep.preparePhoto(uri) }.getOrNull()
                if (prepared == null) {
                    _pinFailed.value = true
                    return@launch
                }
                try {
                    val uploaded = attachmentApi.upload(
                        file = prepared.file,
                        mime = prepared.mime,
                        kind = prepared.kind,
                        width = prepared.width,
                        height = prepared.height,
                        durationMs = null,
                    )
                    val attachment = (uploaded as? ApiResult.Ok)?.value?.attachment
                    if (attachment == null) {
                        _pinFailed.value = true
                        return@launch
                    }
                    // The preview the sticker draws, its own upload — the
                    // same second leg a photo message has.
                    prepared.previewJpeg?.let { jpeg ->
                        attachmentApi.uploadPreview(attachment.id, jpeg)
                    }
                    val pinned = boardRepository.addNote(
                        text = "",
                        color = NoteColors.palette[slot % NoteColors.palette.size],
                        size = NoteSizes.MEDIUM,
                        font = NoteFonts.PLAIN,
                        x = 0.12 + slot * 0.03,
                        y = 0.10 + slot * 0.06,
                        attachmentId = attachment.id,
                    )
                    if (!pinned) _pinFailed.value = true
                } finally {
                    // Staged bytes have no further job once they are up.
                    runCatching { prepared.file.delete() }
                }
            } finally {
                _pinning.value = false
            }
        }
    }

    val notes: StateFlow<List<NoteEntity>> = boardRepository.observeNotes()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /** Whose notes hide their content as well as their author. */
    val blockedUserIds: StateFlow<Set<Long>> = settings.state.map { it.blockedUserIds }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    val myUserId: StateFlow<Long?> = settings.state
        .map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    // The FULL roster, tombstones included: a note pinned by somebody
    // whose account is gone still has to say who wrote it. Their stored
    // name is the server's English placeholder, so this resolves the
    // translated one (docs/protocol.md, "Deleting an account").
    val memberNames: StateFlow<Map<Long, String>> = memberDao.observeMembers()
        .map { members -> members.resolvedDisplayNames(appContext) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

    /**
     * Opening the board catches up rather than re-reading: the family call
     * already knows the server's cursor, so a board nothing has happened on
     * costs one request instead of the whole wall.
     */
    fun refresh() {
        viewModelScope.launch {
            val serverMax = familyRepository.refreshMine().okOrNull()?.maxBoardSeq ?: 0L
            boardRepository.catchUpBoard(serverMax)
        }
    }

    /**
     * Everything on the wall has been shown, so the badge's marks move up
     * to it (BoardBadge, docs/protocol.md, "Board"). Both marks together:
     * one from before an update and one from after would be neither rule.
     */
    fun markBoardSeen() {
        viewModelScope.launch {
            val marks = BoardBadge.marksAfterShowing(
                boardRepository.observeNotes().first().marks(),
                settings.state.first().badgeMarks(),
            )
            settings.setBoardSeenNoteId(marks.seenNoteId)
            settings.setBoardSeenContentSeq(marks.seenContentSeq)
        }
    }

    fun addNote(text: String, color: String, size: String, font: String, x: Double, y: Double) {
        viewModelScope.launch { boardRepository.addNote(text, color, size, font, x, y) }
    }

    /** Anyone in the family may move any note. */
    fun moveNote(id: Long, x: Double, y: Double) {
        viewModelScope.launch { boardRepository.updateNote(id, x = x, y = y) }
    }

    /**
     * Author only, enforced server-side; the UI hides it for everyone else.
     * Size and font are author's fields like text and color — a move never
     * carries them (docs/protocol.md, "Board").
     */
    fun editNote(id: Long, text: String, color: String, size: String, font: String) {
        viewModelScope.launch {
            boardRepository.updateNote(id, text = text, color = color, size = size, font = font)
        }
    }

    /**
     * Add an event: a title, a start, an optional end and place. The kind
     * rides with the start (docs/protocol.md, "Board").
     */
    fun addEvent(
        title: String,
        color: String,
        startsAt: String,
        endsAt: String?,
        place: String?,
        x: Double,
        y: Double,
    ) {
        viewModelScope.launch {
            boardRepository.addNote(
                text = title,
                color = color,
                size = NoteSizes.MEDIUM,
                font = NoteFonts.PLAIN,
                x = x,
                y = y,
                startsAt = startsAt,
                endsAt = endsAt,
                place = place,
            )
        }
    }

    /** Answering is the SHARED act, like moving: any member may. */
    fun answerEvent(id: Long, answer: String?) {
        viewModelScope.launch { boardRepository.answerNote(id, answer) }
    }

    fun deleteNote(id: Long) {
        viewModelScope.launch { boardRepository.deleteNote(id) }
    }
}
