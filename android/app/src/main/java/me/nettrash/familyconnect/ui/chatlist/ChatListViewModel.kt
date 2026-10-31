/*
 * ChatListViewModel.kt
 * Family Connect (Android)
 *
 * Renders straight from Room (ChatDao.observeChats already orders:
 * family chat pinned, then most recent activity). Network work is
 * refresh-on-resume plus whatever the socket manager's resyncs pull in.
 * The new-chat picker lists family members minus me; picking one runs
 * the idempotent POST /chats/direct and emits the chat id to navigate.
 * The same sheet starts a GROUP (#107; docs/protocol.md, "Groups"): a name
 * and the members to add, checked here before POST /chats/groups, and the
 * new chat opened the same way.
 *
 * Several families (#88; docs/protocol.md, "Several families"): the list
 * shows the chats whose `family_ids` contain the CURRENT family, and the
 * switcher above it lists every family with its unread, the pending join
 * requests and the invitations — which the banner also offers to answer.
 *
 * Paid families (#89; docs/protocol.md, "Paid families"): while the current
 * family is unpaid, a banner over its chats says when it will be deleted.
 *
 * iOS counterpart: ios/FamilyConnect/UI/ChatList/ChatListViewModel.swift
 */

package me.nettrash.familyconnect.ui.chatlist

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import me.nettrash.familyconnect.R
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.SocketState
import me.nettrash.familyconnect.data.db.NoteDao
import kotlinx.coroutines.flow.first
import me.nettrash.familyconnect.data.repo.ChatRepository
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.GroupRepository
import me.nettrash.familyconnect.data.repo.FamilyList
import me.nettrash.familyconnect.data.repo.PaidFamilies
import me.nettrash.familyconnect.data.settings.currentFamily
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.familyKey
import me.nettrash.familyconnect.ui.families.FamiliesUi
import me.nettrash.familyconnect.ui.families.FamilyErrors
import javax.inject.Inject
import me.nettrash.familyconnect.util.BoardBadge
import me.nettrash.familyconnect.util.badgeMarks
import me.nettrash.familyconnect.util.marks
import me.nettrash.familyconnect.util.GroupRules

@HiltViewModel
class ChatListViewModel @Inject constructor(
    /**
     * The application context, for `getString` only.
     *
     * A ViewModel holding a Context is usually a smell; the APPLICATION
     * context is the exception — it outlives every screen, so there is
     * nothing to leak. The alternative, carrying @StringRes ids through
     * every state field, spreads resource plumbing across code whose job
     * is state. The trade-off: a message is resolved when it is produced
     * rather than when it is drawn, so one already on screen keeps its
     * language if the system locale changes underneath it — and Android
     * recreates the activity then anyway.
     */
    @param:ApplicationContext private val appContext: Context,
    private val chatRepository: ChatRepository,
    private val familyRepository: FamilyRepository,
    // The DAO rather than BoardRepository: the badge needs one flow of
    // notes, not the board's whole sync machinery, and the narrower
    // dependency keeps this screen's tests free of board plumbing.
    private val noteDao: NoteDao,
    private val settingsRepository: SettingsRepository,
    settings: SettingsRepository,
    connectivity: ConnectivityObserver,
    socket: ChatSocket,
    private val groupRepository: GroupRepository,
) : ViewModel() {

    // Seeded with null, not emptyList(): null means "Room hasn't answered
    // yet" so the screen can show skeleton rows instead of flashing the
    // "No chats yet" empty state on cold entry.
    //
    // The CURRENT family's chats: those whose `family_ids` name it — a direct
    // chat appears in every family it names, and the family chat and `ai`
    // chat shown are this family's (docs/protocol.md, "Several families").
    val chats: StateFlow<List<ChatEntity>?> =
        combine(chatRepository.observeChats(), settings.state) { chats, s ->
            FamilyList.chatsOf(chats, s.currentFamilyId, s.families.firstOrNull()?.id)
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * What the switcher and the invitation banner draw (docs/protocol.md,
     * "Several families" — "Switching").
     */
    val families: StateFlow<FamiliesUi> =
        combine(chatRepository.observeChats(), settings.state) { chats, s ->
            FamiliesUi(
                families = s.families,
                currentFamilyId = s.currentFamilyId,
                unread = FamilyList.unreadByFamily(chats, s.families),
                pending = s.pendingJoinRequests,
                invitations = s.invitations,
                showsSwitcher = FamilyList.showsSwitcher(s),
                canStartAnother = FamilyList.canStartAnother(s),
                canJoinAnother = FamilyList.canJoinAnother(s),
            )
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), FamiliesUi())

    /**
     * The banner over the CURRENT family's chats while it is unpaid
     * (docs/protocol.md, "Paid families"), or null while it has no deadline
     * — and for a member, while the roster does not name the owner yet. A
     * family the app is not showing needs none. Live: a `family_plan` frame
     * moves `delete_at` on the family's entry, and this follows.
     */
    val unpaidBanner: StateFlow<PaidFamilies.UnpaidBanner?> =
        combine(settings.state, familyRepository.observeActiveMembers()) { s, members ->
            PaidFamilies.banner(
                s.currentFamily,
                ownerName = members.firstOrNull { it.role == FamilyList.OWNER }?.displayName,
            )
        }.distinctUntilChanged()
            .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /** The switcher's tap on a family. */
    fun switchFamily(familyId: Long) {
        viewModelScope.launch { familyRepository.switchTo(familyId) }
    }

    private val _answeringInvitation = MutableStateFlow<Long?>(null)

    /** The invitation whose Join or Decline is in flight; its buttons wait. */
    val answeringInvitation: StateFlow<Long?> = _answeringInvitation

    /**
     * "Join" on an invitation: the family is added and shown
     * (FamilyRepository.acceptInvitation). Its errors are said here.
     */
    fun acceptInvitation(invitationId: Long) = answerInvitation(invitationId) {
        familyRepository.acceptInvitation(invitationId)
    }

    /** "Decline" on an invitation. */
    fun declineInvitation(invitationId: Long) = answerInvitation(invitationId) {
        familyRepository.declineInvitation(invitationId)
    }

    private fun answerInvitation(invitationId: Long, call: suspend () -> ApiResult<*>) {
        if (_answeringInvitation.value != null) return
        _answeringInvitation.value = invitationId
        viewModelScope.launch {
            val result = call()
            _answeringInvitation.value = null
            _error.value = when (result) {
                is ApiResult.Ok -> null
                is ApiResult.HttpError ->
                    FamilyErrors.message(appContext, result.code, settingsRepository.state.first().maxFamiliesPerAccount)
                        ?: if (result.code == "family_full") {
                            appContext.getString(R.string.e_family_full_join)
                        } else {
                            result.message ?: appContext.getString(R.string.e_join_failed)
                        }
                is ApiResult.NetworkError -> appContext.getString(R.string.e_unreachable)
            }
        }
    }

    /**
     * userId → profile-picture version, so a direct-chat row can name its
     * peer's picture. One flow for the whole list rather than a lookup
     * per row.
     */
    val avatarVersions: StateFlow<Map<Long, Long>> = familyRepository.observeMembers()
        .map { members -> members.associate { it.userId to it.avatarVersion } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())

    /**
     * Picker candidates: everyone still in the family but me.
     *
     * ACTIVE members — you cannot start a chat with somebody who left, and
     * offering them would be offering a request the server refuses.
     * `avatarVersions` above deliberately reads the FULL roster instead:
     * it is drawing faces on existing rows, including a direct chat with
     * somebody who has since gone.
     */
    /**
     * Who the reader has blocked, and their own id — the chat list needs
     * both to decide whether a row's preview is somebody else's hidden
     * message.
     */
    val blockedUserIds: StateFlow<Set<Long>> = settings.state.map { it.blockedUserIds }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptySet())

    val myUserId: StateFlow<Long?> = settings.state.map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    val pickableMembers: StateFlow<List<MemberEntity>> =
        combine(familyRepository.observeActiveMembers(), settings.state) { members, s ->
            // Blocked members are left OUT rather than left in to have the
            // tap answered `blocked` — the protocol's own steer for what a
            // client does with a complete roster (docs/protocol.md,
            // "Blocking a member"). The roster itself is untouched: they
            // are still nameable everywhere a name is needed.
            members.filter { it.userId != s.myUserId && it.userId !in s.blockedUserIds }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /**
     * Who a new GROUP may be started with: everyone still in the family but
     * me ([GroupRules.startableWith]). Unlike [pickableMembers], the blocked
     * are offered — a block is never a reason to refuse an add, and inside a
     * group it works as it does in the family chat (docs/protocol.md,
     * "Groups").
     */
    val groupCandidates: StateFlow<List<MemberEntity>> =
        combine(familyRepository.observeActiveMembers(), settings.state) { members, s ->
            val ids = GroupRules.startableWith(members.map { it.userId }, s.myUserId).toSet()
            members.filter { it.userId in ids }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    private val _groupError = MutableStateFlow<String?>(null)

    /** Why the New Group form did not create one; null while there is nothing to say. */
    val groupError: StateFlow<String?> = _groupError

    private val _creatingGroup = MutableStateFlow(false)

    /** A `POST /chats/groups` is in flight: the form's button waits for it. */
    val creatingGroup: StateFlow<Boolean> = _creatingGroup

    /**
     * Create a group named [name] with [memberIds], and open it.
     *
     * Both checks the server makes are made first, so a form that can only
     * be refused is never sent: the trimmed name is 1 to 64 characters, and
     * somebody besides the creator is in it.
     */
    fun createGroup(name: String, memberIds: Set<Long>) {
        if (_creatingGroup.value) return
        val title = GroupRules.validName(name)
        if (title == null) {
            _groupError.value = appContext.getString(R.string.e_group_name_length)
            return
        }
        if (memberIds.isEmpty()) {
            _groupError.value = appContext.getString(R.string.e_group_choose_member)
            return
        }
        _groupError.value = null
        _creatingGroup.value = true
        viewModelScope.launch {
            val result = groupRepository.create(title, memberIds.toList())
            _creatingGroup.value = false
            when (result) {
                is ApiResult.Ok -> _navigateToChat.tryEmit(result.value.id)
                else -> _groupError.value = appContext.getString(GroupRules.createFailure(result))
            }
        }
    }

    /** The form was closed or edited: what it said no longer applies. */
    fun clearGroupError() {
        _groupError.value = null
    }

    /**
     * What is new to READ on the board since this device last showed it:
     * notes pinned, and notes whose text changed. Dragging, resizing and
     * recolouring are not news and never count — BoardBadge holds the rule,
     * shared with the phone and the Mac (issue #53, docs/protocol.md,
     * "Board").
     *
     * Never counted from `boardCursor`, which a background resync advances
     * and would silently clear the badge for somebody who never looked.
     */
    @OptIn(ExperimentalCoroutinesApi::class)
    val newNoteCount: StateFlow<Int> =
        settings.state.map { it.familyKey }.distinctUntilChanged()
            .flatMapLatest { familyId ->
                // The CURRENT family's board, against its own marks.
                combine(noteDao.observeNotes(familyId), settings.state) { notes, s ->
                    BoardBadge.unreadCount(notes.marks(), s.badgeMarks(familyId))
                }
            }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), 0)

    /**
     * The board is on screen, so everything on it has been shown.
     *
     * Called when the board is LEFT as well as when it is opened, which is
     * the half that was missing: marking at the tap alone marked an empty
     * cache seen, and then the whole wall — loaded a moment later, while
     * the user was looking straight at it — came back as unread the moment
     * they went back.
     */
    fun markBoardSeen() {
        viewModelScope.launch {
            val familyId = settingsRepository.state.first().familyKey
            val notes = noteDao.observeNotes(familyId).first().marks()
            val marks = BoardBadge.marksAfterShowing(
                notes,
                settingsRepository.state.first().badgeMarks(familyId),
            )
            settingsRepository.setBoardSeenNoteId(familyId, marks.seenNoteId)
            settingsRepository.setBoardSeenContentSeq(familyId, marks.seenContentSeq)
        }
    }

    val isOnline: StateFlow<Boolean> = connectivity.isOnline
    val socketState: StateFlow<SocketState> = socket.state

    private val _navigateToChat = MutableSharedFlow<Long>(extraBufferCapacity = 1)
    val navigateToChat: SharedFlow<Long> = _navigateToChat

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error

    fun refresh() {
        viewModelScope.launch {
            chatRepository.refreshChats()
        }
    }

    fun openDirectChat(userId: Long) {
        viewModelScope.launch {
            when (val result = chatRepository.createDirect(userId)) {
                is ApiResult.Ok -> _navigateToChat.tryEmit(result.value.id)
                is ApiResult.HttpError ->
                    _error.value = result.message ?: appContext.getString(R.string.e_open_chat_failed)
                is ApiResult.NetworkError ->
                    _error.value = appContext.getString(R.string.e_unreachable)
            }
        }
    }

    fun dismissError() {
        _error.value = null
    }
}
