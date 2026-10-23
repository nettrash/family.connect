/*
 * GroupRepository.kt
 * Family Connect (Android)
 *
 * Making and changing groups (#107; docs/protocol.md, "Groups"): create,
 * rename, add members, remove a member, leave, delete. Each call writes its
 * answer into Room through ChatRepository the moment it lands — the group
 * as it now is ([ChatRepository.applyChat]), or gone
 * ([ChatRepository.deleteChat]) — so the screen that asked shows the result
 * without waiting for the `chat_updated` / `chat_removed` frame the server
 * also sends. The frame, arriving after, writes the same thing again.
 *
 * A `chat_not_found` answer means the group is no longer this person's —
 * left, removed from or deleted elsewhere while they looked at it — and it
 * is dropped here too, which is what `GET /chats` would do next anyway.
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.flow.first
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.GroupApi
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.familyKey
import me.nettrash.familyconnect.data.settings.pathFamily
import me.nettrash.familyconnect.util.GroupRules
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class GroupRepository @Inject constructor(
    private val groupApi: GroupApi,
    private val chatRepository: ChatRepository,
    /** Which family a new group is in — the one shown (docs/protocol.md, "Several families"). */
    private val settings: SettingsRepository,
) {

    /**
     * `POST /chats/groups`. [title] must already be [GroupRules.validName]
     * and [memberIds] non-empty — the screen checks both before asking.
     */
    suspend fun create(title: String, memberIds: List<Long>): ApiResult<ChatEntity> =
        when (val result = groupApi.create(title, memberIds, settings.state.first().let { it.pathFamily(it.familyKey) })) {
            is ApiResult.Ok -> stored(result.value.chat)
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    /** `PATCH /chats/{id}` — admin only. */
    suspend fun rename(chatId: Long, title: String): ApiResult<ChatEntity> =
        applied(chatId, groupApi.rename(chatId, title).map { it.chat })

    /** `POST /chats/{id}/members` — admin only. */
    suspend fun addMembers(chatId: Long, userIds: List<Long>): ApiResult<ChatEntity> =
        applied(chatId, groupApi.addMembers(chatId, userIds).map { it.chat })

    /** `DELETE /chats/{id}/members/{user_id}` for somebody else — admin only. */
    suspend fun removeMember(chatId: Long, userId: Long): ApiResult<ChatEntity?> =
        when (val result = groupApi.removeMember(chatId, userId)) {
            is ApiResult.Ok -> {
                val chat = result.value
                if (chat == null) {
                    // A 204 here is the server saying the caller is out —
                    // which only their own id asks for. Never expected from
                    // a removal, so treat it as what it says.
                    chatRepository.deleteChat(chatId)
                    ApiResult.Ok(null)
                } else {
                    ApiResult.Ok(chatRepository.applyChat(chat))
                }
            }
            is ApiResult.HttpError -> forgetIfGone(chatId, result)
            is ApiResult.NetworkError -> result
        }

    /**
     * Leave: `DELETE /chats/{id}/members/{me}` → `204`. The group goes from
     * this device at once, with its messages; an admin who leaves hands it
     * on, and the last one out deletes it — the server's business.
     */
    suspend fun leave(chatId: Long, me: Long): ApiResult<Unit> =
        when (val result = groupApi.removeMember(chatId, me)) {
            is ApiResult.Ok -> {
                chatRepository.deleteChat(chatId)
                ApiResult.Ok(Unit)
            }
            is ApiResult.HttpError -> forgetIfGone(chatId, result)
            is ApiResult.NetworkError -> result
        }

    /** `DELETE /chats/{id}` — admin only; final, for everyone. */
    suspend fun delete(chatId: Long): ApiResult<Unit> =
        when (val result = groupApi.delete(chatId)) {
            is ApiResult.Ok -> {
                chatRepository.deleteChat(chatId)
                ApiResult.Ok(Unit)
            }
            is ApiResult.HttpError -> forgetIfGone(chatId, result)
            is ApiResult.NetworkError -> result
        }

    private suspend fun applied(chatId: Long, result: ApiResult<ChatDto>): ApiResult<ChatEntity> =
        when (result) {
            is ApiResult.Ok -> stored(result.value)
            is ApiResult.HttpError -> forgetIfGone(chatId, result)
            is ApiResult.NetworkError -> result
        }

    private suspend fun stored(dto: ChatDto): ApiResult<ChatEntity> {
        val entity = chatRepository.applyChat(dto)
            ?: return ApiResult.NetworkError(IllegalStateException("not a group"))
        return ApiResult.Ok(entity)
    }

    private suspend fun forgetIfGone(chatId: Long, error: ApiResult.HttpError): ApiResult.HttpError {
        if (GroupRules.isGone(error)) chatRepository.deleteChat(chatId)
        return error
    }

    private inline fun <T, R> ApiResult<T>.map(transform: (T) -> R): ApiResult<R> = when (this) {
        is ApiResult.Ok -> ApiResult.Ok(transform(value))
        is ApiResult.HttpError -> this
        is ApiResult.NetworkError -> this
    }
}
