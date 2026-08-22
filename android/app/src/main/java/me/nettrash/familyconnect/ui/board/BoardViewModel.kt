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

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.repo.BoardRepository
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import javax.inject.Inject

@HiltViewModel
class BoardViewModel @Inject constructor(
    private val boardRepository: BoardRepository,
    private val familyRepository: FamilyRepository,
    memberDao: MemberDao,
    settings: SettingsRepository,
) : ViewModel() {

    val notes: StateFlow<List<NoteEntity>> = boardRepository.observeNotes()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    val myUserId: StateFlow<Long?> = settings.state
        .map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    val memberNames: StateFlow<Map<Long, String>> = memberDao.observeMembers()
        .map { members -> members.associate { it.userId to it.displayName } }
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

    fun addNote(text: String, color: String, x: Double, y: Double) {
        viewModelScope.launch { boardRepository.addNote(text, color, x, y) }
    }

    /** Anyone in the family may move any note. */
    fun moveNote(id: Long, x: Double, y: Double) {
        viewModelScope.launch { boardRepository.updateNote(id, x = x, y = y) }
    }

    /** Author only, enforced server-side; the UI hides it for everyone else. */
    fun editNote(id: Long, text: String, color: String) {
        viewModelScope.launch { boardRepository.updateNote(id, text = text, color = color) }
    }

    fun deleteNote(id: Long) {
        viewModelScope.launch { boardRepository.deleteNote(id) }
    }
}
