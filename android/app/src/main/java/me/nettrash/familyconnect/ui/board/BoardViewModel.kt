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
) : ViewModel() {

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

    fun addNote(text: String, color: String, size: String, x: Double, y: Double) {
        viewModelScope.launch { boardRepository.addNote(text, color, size, x, y) }
    }

    /** Anyone in the family may move any note. */
    fun moveNote(id: Long, x: Double, y: Double) {
        viewModelScope.launch { boardRepository.updateNote(id, x = x, y = y) }
    }

    /**
     * Author only, enforced server-side; the UI hides it for everyone else.
     * Size is an author's field like text and color — a move never carries
     * it (docs/protocol.md, "Board").
     */
    fun editNote(id: Long, text: String, color: String, size: String) {
        viewModelScope.launch { boardRepository.updateNote(id, text = text, color = color, size = size) }
    }

    fun deleteNote(id: Long) {
        viewModelScope.launch { boardRepository.deleteNote(id) }
    }
}
