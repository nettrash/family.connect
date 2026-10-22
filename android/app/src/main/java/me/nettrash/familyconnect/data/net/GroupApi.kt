/*
 * GroupApi.kt
 * Family Connect (Android)
 *
 * The five group endpoints of docs/protocol.md, "Groups" (added
 * 2026-10-22):
 *
 *   POST   /chats/groups                    {title, member_ids} → 201 {chat}
 *   PATCH  /chats/{id}                      {title}             → 200 {chat}
 *   POST   /chats/{id}/members              {user_ids}          → 200 {chat}
 *   DELETE /chats/{id}/members/{user_id}                        → 200 {chat} | 204
 *   DELETE /chats/{id}                                          → 204
 *
 * Kept out of ChatApi on purpose: a separate seam means none of the many
 * scripted ChatApi fakes the send pipeline's tests carry had to learn five
 * calls they never make, and the group screens' tests script only these.
 *
 * Interface + impl split so GroupRepository's tests can script the server
 * without an HTTP stack.
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.AddGroupMembersRequest
import me.nettrash.familyconnect.data.net.dto.ChatDto
import me.nettrash.familyconnect.data.net.dto.ChatResponse
import me.nettrash.familyconnect.data.net.dto.CreateGroupRequest
import me.nettrash.familyconnect.data.net.dto.RenameGroupRequest
import javax.inject.Inject
import javax.inject.Singleton

interface GroupApi {
    /** A new group: the caller is added and becomes its admin. */
    suspend fun create(title: String, memberIds: List<Long>): ApiResult<ChatResponse>

    /** Renames a group. Admin only (`not_group_admin`). */
    suspend fun rename(chatId: Long, title: String): ApiResult<ChatResponse>

    /** Adds members; an id already in it is ignored. Admin only. */
    suspend fun addMembers(chatId: Long, userIds: List<Long>): ApiResult<ChatResponse>

    /**
     * Removes [userId] from the group — the admin removing somebody, or
     * anybody with their own id, which is LEAVING.
     *
     * The answer is the group as it now is when somebody else was removed
     * (`200 {chat}`), and null when the caller left (`204`): there is no
     * group left for them to be told about.
     */
    suspend fun removeMember(chatId: Long, userId: Long): ApiResult<ChatDto?>

    /** Deletes a group and everything in it, for everyone. Admin only. */
    suspend fun delete(chatId: Long): ApiResult<Unit>
}

@Singleton
class DefaultGroupApi @Inject constructor(
    private val client: ApiClient,
) : GroupApi {

    override suspend fun create(title: String, memberIds: List<Long>): ApiResult<ChatResponse> =
        client.post("/chats/groups", CreateGroupRequest(title, memberIds))

    override suspend fun rename(chatId: Long, title: String): ApiResult<ChatResponse> =
        client.patch("/chats/$chatId", RenameGroupRequest(title))

    override suspend fun addMembers(chatId: Long, userIds: List<Long>): ApiResult<ChatResponse> =
        client.post("/chats/$chatId/members", AddGroupMembersRequest(userIds))

    override suspend fun removeMember(chatId: Long, userId: Long): ApiResult<ChatDto?> =
        // Raw, not `client.delete<ChatResponse>`: the same endpoint answers
        // 200 with a body or 204 with none, and the typed decode would read
        // an empty 204 as an unreadable answer.
        when (val result = client.raw("DELETE", "/chats/$chatId/members/$userId", null)) {
            is ApiResult.Ok -> decodeRemoval(result.value)
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    override suspend fun delete(chatId: Long): ApiResult<Unit> =
        client.delete("/chats/$chatId")

    private fun decodeRemoval(body: String): ApiResult<ChatDto?> =
        if (body.isBlank()) {
            ApiResult.Ok(null)
        } else {
            when (val decoded = client.decode<ChatResponse>(ApiResult.Ok(body))) {
                is ApiResult.Ok -> ApiResult.Ok(decoded.value.chat)
                is ApiResult.HttpError -> decoded
                is ApiResult.NetworkError -> decoded
            }
        }
}
