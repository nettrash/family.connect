/*
 * FamilyListTest.kt
 * Family Connect (Android)
 *
 * The rules of several families (#88; docs/protocol.md, "Several
 * families") that are decisions rather than requests: which family is
 * shown, the status, when the switcher appears, when the two doors to
 * another family are offered, which chats a family lists, the switcher's
 * unread, where a push opens and who "Invite…" offers.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.GroupMembersCodec
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.InvitationDto
import me.nettrash.familyconnect.data.net.dto.PendingJoinRequestDto
import me.nettrash.familyconnect.data.net.dto.UserDto
import me.nettrash.familyconnect.data.settings.SettingsState
import org.junit.Test

class FamilyListTest {

    private fun family(id: Long, role: String = "member") =
        FamilyDto(id = id, name = "Family $id", joinPolicy = "open", role = role)

    private fun chat(id: Long, kind: String, familyIds: List<Long>?, unread: Int = 0) = ChatEntity(
        id = id,
        kind = kind,
        peerUserId = null,
        title = "chat $id",
        unreadCount = unread,
        myLastReadId = null,
        peerLastReadId = null,
        lastMessageBody = null,
        lastMessageAt = null,
        lastMessageSenderId = null,
        familyIdsJson = GroupMembersCodec.encode(familyIds),
    )

    private val pending = PendingJoinRequestDto(familyId = 9, familyName = "The Joneses", createdAt = "2026-10-22T10:00:00Z")

    private val invitation = InvitationDto(
        id = 4,
        familyId = 9,
        familyName = "The Joneses",
        invitedBy = UserDto(7, "anna", "Anna"),
        user = UserDto(11, "ben", "Ben"),
    )

    @Test
    fun `the family shown is the stored one while it is there, else the first`() {
        val families = listOf(family(3), family(5))
        assertThat(FamilyList.current(families, 5)).isEqualTo(5L)
        assertThat(FamilyList.current(families, 8)).isEqualTo(3L)
        assertThat(FamilyList.current(families, null)).isEqualTo(3L)
        assertThat(FamilyList.current(emptyList(), 5)).isNull()
    }

    @Test
    fun `the status is the current family's role, or today's gate with none`() {
        val families = listOf(family(3, "member"), family(5, "owner"))
        assertThat(FamilyList.status(families, 3, emptyList())).isEqualTo(FamilyStatus.MEMBER)
        assertThat(FamilyList.status(families, 5, emptyList())).isEqualTo(FamilyStatus.OWNER)
        // A pending request beside a family is a switcher row, not the status.
        assertThat(FamilyList.status(families, 3, listOf(pending))).isEqualTo(FamilyStatus.MEMBER)
        assertThat(FamilyList.status(emptyList(), null, listOf(pending))).isEqualTo(FamilyStatus.PENDING)
        assertThat(FamilyList.status(emptyList(), null, emptyList())).isEqualTo(FamilyStatus.NONE)
    }

    @Test
    fun `one family with nothing pending has no switcher`() {
        val one = SettingsState(families = listOf(family(3)))
        assertThat(FamilyList.showsSwitcher(one)).isFalse()
        assertThat(FamilyList.showsSwitcher(one.copy(families = listOf(family(3), family(5))))).isTrue()
        assertThat(FamilyList.showsSwitcher(one.copy(pendingJoinRequests = listOf(pending)))).isTrue()
        assertThat(FamilyList.showsSwitcher(one.copy(invitations = listOf(invitation)))).isTrue()
        // No family at all is the gate, not a switcher.
        assertThat(FamilyList.showsSwitcher(SettingsState(invitations = listOf(invitation)))).isFalse()
    }

    @Test
    fun `New Family is offered only to a non-owner under the cap on a server that allows it`() {
        val member = SettingsState(
            families = listOf(family(3)),
            memberFamiliesEnabled = true,
            maxFamiliesPerAccount = 2,
            multiFamilyServer = true,
        )
        assertThat(FamilyList.canStartAnother(member)).isTrue()
        assertThat(FamilyList.canStartAnother(member.copy(memberFamiliesEnabled = false))).isFalse()
        assertThat(FamilyList.canStartAnother(member.copy(families = listOf(family(3, "owner"))))).isFalse()
        assertThat(FamilyList.canStartAnother(member.copy(families = listOf(family(3), family(4))))).isFalse()
        // A server from before several families offers neither door.
        assertThat(FamilyList.canStartAnother(member.copy(multiFamilyServer = false))).isFalse()
    }

    @Test
    fun `Join Another Family is offered while under the cap, owner or not`() {
        val owner = SettingsState(
            families = listOf(family(3, "owner")),
            maxFamiliesPerAccount = 2,
            multiFamilyServer = true,
        )
        assertThat(FamilyList.canJoinAnother(owner)).isTrue()
        assertThat(FamilyList.canJoinAnother(owner.copy(families = listOf(family(3), family(4))))).isFalse()
        assertThat(FamilyList.canJoinAnother(owner.copy(maxFamiliesPerAccount = 0))).isFalse()
        assertThat(FamilyList.canJoinAnother(SettingsState(maxFamiliesPerAccount = 5, multiFamilyServer = true)))
            .isFalse()
    }

    @Test
    fun `a family lists the chats that name it, a direct chat in each, and an unsaid one is home's`() {
        val chats = listOf(
            chat(1, "family", listOf(3)),
            chat(2, "family", listOf(5)),
            chat(3, "direct", listOf(3, 5)),
            chat(4, "ai", listOf(5)),
            chat(5, "direct", null),
        )
        assertThat(FamilyList.chatsOf(chats, 3, homeFamilyId = 3).map { it.id }).containsExactly(1L, 3L, 5L).inOrder()
        assertThat(FamilyList.chatsOf(chats, 5, homeFamilyId = 3).map { it.id }).containsExactly(2L, 3L, 4L).inOrder()
    }

    @Test
    fun `the switcher counts a direct chat in every family that lists it`() {
        val chats = listOf(
            chat(1, "family", listOf(3), unread = 2),
            chat(2, "family", listOf(5), unread = 1),
            chat(3, "direct", listOf(3, 5), unread = 4),
        )
        assertThat(FamilyList.unreadByFamily(chats, listOf(family(3), family(5))))
            .containsExactly(3L, 6, 5L, 5)
    }

    @Test
    fun `a push opens in the family shown when the chat is listed there, else the first that lists it`() {
        val families = listOf(family(3), family(5), family(8))
        assertThat(FamilyList.familyForChat(listOf(5, 8), current = 8, families)).isEqualTo(8L)
        assertThat(FamilyList.familyForChat(listOf(8, 5), current = 3, families)).isEqualTo(5L)
        assertThat(FamilyList.familyForChat(null, current = 3, families)).isNull()
    }

    @Test
    fun `Invite offers the members of the owner's other families who are not in this one`() {
        fun row(familyId: Long, userId: Long, name: String, hasLeft: Boolean = false, deleted: Boolean = false) =
            MemberEntity(
                userId = userId, username = name, displayName = name, role = "member",
                hasLeft = hasLeft, deleted = deleted, familyId = familyId,
            )
        val rows = listOf(
            row(3, 7, "Me"),
            row(3, 9, "Zoe"),
            row(5, 7, "Me"),
            row(5, 9, "Zoe"),
            row(5, 11, "Ben"),
            row(5, 12, "Gone", hasLeft = true),
            row(8, 11, "Ben"),
            row(8, 13, "Ann"),
            row(8, 14, "Ghost", deleted = true),
        )
        assertThat(FamilyList.inviteCandidates(rows, familyId = 3, myUserId = 7).map { it.userId })
            .containsExactly(13L, 11L).inOrder()
        assertThat(FamilyList.hasOtherFamily(listOf(family(3)), 3)).isFalse()
        assertThat(FamilyList.hasOtherFamily(listOf(family(3), family(5)), 3)).isTrue()
    }
}
