/*
 * GroupInfoViewModel.kt
 * Family Connect (Android)
 *
 * A group's info (#107; docs/protocol.md, "Groups"): its name, its members
 * and who administers it, and the five things that can be done to it — the
 * admin's Rename, Add Members, Remove and Delete, and everybody's Leave.
 *
 * Renders from Room like every screen: the chat row carries the group's
 * name, member ids and admin, which `GET /chats`, the `chat_updated` frame
 * and each action's own answer keep current (GroupRepository). Who may do
 * what is GroupRules' — this only asks it.
 *
 * The chat id comes from the back-stack entry (`group_info/{chatId}`).
 */

package me.nettrash.familyconnect.ui.group

import android.content.Context
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.repo.ChatRepository
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.GroupRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.util.GroupRules
import me.nettrash.familyconnect.util.resolvedDisplayName
import javax.inject.Inject

/** One member of the group, as its info lists them. */
data class GroupMemberRow(
    val userId: Long,
    val name: String,
    /** Null when the roster does not hold them yet. */
    val username: String?,
    val avatarVersion: Long,
    val isAdmin: Boolean,
    val isMe: Boolean,
)

@HiltViewModel
class GroupInfoViewModel @Inject constructor(
    /** For `getString` only — see the note on ChatListViewModel. */
    @param:ApplicationContext private val appContext: Context,
    savedStateHandle: SavedStateHandle,
    chatRepository: ChatRepository,
    familyRepository: FamilyRepository,
    settings: SettingsRepository,
    private val groupRepository: GroupRepository,
) : ViewModel() {

    private val chatId: Long = checkNotNull(savedStateHandle.get<Long>("chatId")) {
        "group_info route requires a chatId argument"
    }

    /** Started eagerly with null: the screen tells "not loaded" from "gone" itself. */
    val chat: StateFlow<ChatEntity?> = chatRepository.observeChat(chatId)
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    val myUserId: StateFlow<Long?> = settings.state.map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /**
     * The group's members in the order they joined, each named from the
     * FULL roster — so somebody the roster has not caught up with still
     * gets a line — with the admin and the reader marked.
     */
    val members: StateFlow<List<GroupMemberRow>> =
        combine(chat, familyRepository.observeMembers(), myUserId) { chatEntity, roster, me ->
            val byId = roster.associateBy { it.userId }
            chatEntity?.memberIds.orEmpty().map { id ->
                val member = byId[id]
                GroupMemberRow(
                    userId = id,
                    name = member?.resolvedDisplayName(appContext) ?: appContext.getString(R.string.s_someone),
                    username = member?.username,
                    avatarVersion = member?.avatarVersion ?: 0L,
                    isAdmin = id == chatEntity?.adminUserId,
                    isMe = id == me,
                )
            }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /** Who Add Members offers ([GroupRules.addable]). */
    val addable: StateFlow<List<MemberEntity>> =
        combine(chat, familyRepository.observeActiveMembers()) { chatEntity, active ->
            val ids = GroupRules.addable(active.map { it.userId }, chatEntity?.memberIds.orEmpty()).toSet()
            active.filter { it.userId in ids }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    private val _busy = MutableStateFlow(false)

    /** One change is in flight; the screen's actions wait for it. */
    val busy: StateFlow<Boolean> = _busy

    private val _error = MutableStateFlow<String?>(null)

    /** What the last change's refusal said; null while there is nothing to say. */
    val error: StateFlow<String?> = _error

    fun dismissError() {
        _error.value = null
    }

    /** May the reader do [action] (to [target], for a removal)? */
    fun may(action: GroupRules.Action, target: Long? = null): Boolean {
        val current = chat.value ?: return false
        return GroupRules.may(action, current.kind, current.memberIds, current.adminUserId, myUserId.value, target)
    }

    /** Rename to [name]; [onDone] runs once it is accepted, so the dialog can close. */
    fun rename(name: String, onDone: () -> Unit) {
        val title = GroupRules.validName(name)
        if (title == null) {
            _error.value = appContext.getString(R.string.e_group_name_length)
            return
        }
        if (!may(GroupRules.Action.RENAME)) return
        run(renaming = true, onDone = onDone) { groupRepository.rename(chatId, title) }
    }

    fun addMembers(userIds: Set<Long>, onDone: () -> Unit) {
        if (userIds.isEmpty()) {
            _error.value = appContext.getString(R.string.e_group_choose_member)
            return
        }
        if (!may(GroupRules.Action.ADD_MEMBERS)) return
        run(onDone = onDone) { groupRepository.addMembers(chatId, userIds.toList()) }
    }

    fun removeMember(userId: Long) {
        if (!may(GroupRules.Action.REMOVE_MEMBER, userId)) return
        run { groupRepository.removeMember(chatId, userId) }
    }

    /** Leave — the row goes from this device on success, and the screen with it. */
    fun leave() {
        val me = myUserId.value ?: return
        if (!may(GroupRules.Action.LEAVE)) return
        run { groupRepository.leave(chatId, me) }
    }

    /** Delete it, for everyone — the row goes on success, and the screen with it. */
    fun delete() {
        if (!may(GroupRules.Action.DELETE)) return
        run { groupRepository.delete(chatId) }
    }

    private fun run(renaming: Boolean = false, onDone: () -> Unit = {}, call: suspend () -> ApiResult<*>) {
        if (_busy.value) return
        _busy.value = true
        _error.value = null
        viewModelScope.launch {
            val result = call()
            _busy.value = false
            if (result is ApiResult.Ok) {
                onDone()
            } else {
                _error.value = appContext.getString(GroupRules.changeFailure(result, renaming))
            }
        }
    }
}
